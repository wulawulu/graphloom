//! `create_base_text_units` workflow.

use async_trait::async_trait;
use futures_util::StreamExt;
use graphloom_chunking::{Chunker, MetadataTransform, TextTransform, add_metadata, create_chunker};
use graphloom_input::{TextDocument, gen_sha512_hash};
use graphloom_llm::{TiktokenTokenizer, Tokenizer};
use polars_core::frame::row::Row;
use serde_json::Value;

use crate::{
    GraphRagConfig, IndexPipelineContext, IndexWorkflow, IndexWorkflowOutput, Result,
    dataframe::{optional_string_at, string_at, usize_to_i64},
    operations::text_units::{TextUnitRow, text_units_dataframe},
};

/// IndexWorkflow name.
pub const CREATE_BASE_TEXT_UNITS_WORKFLOW: &str = "create_base_text_units";

/// Create base text units from documents.
#[derive(Debug, Clone, Copy, Default)]
pub struct CreateBaseTextUnitsWorkflow;

#[async_trait]
impl IndexWorkflow for CreateBaseTextUnitsWorkflow {
    fn name(&self) -> &'static str {
        CREATE_BASE_TEXT_UNITS_WORKFLOW
    }

    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut IndexPipelineContext,
    ) -> Result<IndexWorkflowOutput> {
        let tokenizer = TiktokenTokenizer::new(&config.chunking.encoding_model)?;
        let chunker = create_chunker(&config.chunking)?;
        let prepend_metadata = &config.chunking.prepend_metadata;
        let should_prepend_metadata = !prepend_metadata.is_empty();

        let mut documents = context
            .output_table_provider()
            .open("documents", false)
            .await?;
        let input_rows = documents.length();
        let mut text_units = context
            .output_table_provider()
            .open("text_units", true)
            .await?;
        let mut rows = Vec::new();
        let mut sample = Vec::new();
        let mut processed_documents = 0usize;

        let mut document_rows = documents.rows();
        while let Some(document) = document_rows.next().await {
            let row = document?;
            let document = document_from_row(&row)?;
            if should_prepend_metadata {
                let transform = metadata_transform(&document, prepend_metadata);
                let transform_fn = move |text: &str| transform.transform(text);
                append_document_chunks(
                    &document,
                    chunker.as_ref(),
                    &tokenizer,
                    Some(&transform_fn),
                    &mut rows,
                    &mut sample,
                )?;
            } else {
                append_document_chunks(
                    &document,
                    chunker.as_ref(),
                    &tokenizer,
                    None,
                    &mut rows,
                    &mut sample,
                )?;
            }
            processed_documents = processed_documents.saturating_add(1);
            context.callbacks.progress(
                CREATE_BASE_TEXT_UNITS_WORKFLOW,
                processed_documents,
                Some(input_rows),
            );
        }

        if !rows.is_empty() {
            text_units.write(text_units_dataframe(&rows)?).await?;
        }
        text_units.close().await?;
        context.stats.text_unit_count = rows.len();
        Ok(IndexWorkflowOutput {
            result: sample,
            stop: false,
            input_rows,
            output_rows: rows.len(),
        })
    }
}

fn append_document_chunks(
    document: &TextDocument,
    chunker: &dyn Chunker,
    tokenizer: &dyn Tokenizer,
    transform: Option<&TextTransform>,
    rows: &mut Vec<TextUnitRow>,
    sample: &mut Vec<Value>,
) -> Result<()> {
    for chunk in chunker.chunk(&document.text, transform)? {
        let n_tokens = resolve_chunk_token_count(chunk.token_count, &chunk.text, tokenizer)?;
        let row = TextUnitRow {
            id: gen_sha512_hash([chunk.text.as_str()]),
            human_readable_id: usize_to_i64(
                rows.len(),
                CREATE_BASE_TEXT_UNITS_WORKFLOW,
                "human_readable_id",
            )?,
            text: chunk.text,
            n_tokens: usize_to_i64(n_tokens, CREATE_BASE_TEXT_UNITS_WORKFLOW, "n_tokens")?,
            document_id: document.id.clone(),
            entity_ids: Vec::new(),
            relationship_ids: Vec::new(),
            covariate_ids: Vec::new(),
        };
        if sample.len() < 5 {
            sample.push(row.to_value());
        }
        rows.push(row);
    }
    Ok(())
}

fn resolve_chunk_token_count(
    token_count: Option<usize>,
    text: &str,
    tokenizer: &dyn Tokenizer,
) -> Result<usize> {
    token_count.map_or_else(|| tokenizer.count(text).map_err(Into::into), Ok)
}

fn metadata_transform(document: &TextDocument, prepend_metadata: &[String]) -> MetadataTransform {
    add_metadata(&document.collect(prepend_metadata), ": ", ".\n", false)
}

fn document_from_row(row: &Row<'static>) -> Result<TextDocument> {
    Ok(TextDocument::new(
        string_at(row, 0, "id", CREATE_BASE_TEXT_UNITS_WORKFLOW)?,
        string_at(row, 3, "text", CREATE_BASE_TEXT_UNITS_WORKFLOW)?,
        optional_string_at(row, 2).unwrap_or_default(),
        optional_string_at(row, 5),
        optional_string_at(row, 6)
            .map(|raw| serde_json::from_str(&raw))
            .transpose()?,
    ))
}

#[cfg(test)]
mod tests {
    use std::{
        num::NonZeroUsize,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use graphloom_chunking::ChunkingConfig;
    use graphloom_llm::{LlmError, Result as LlmResult};
    use graphloom_storage::{MemoryTableProvider, TableProvider};

    use super::*;
    use crate::{
        IndexWorkflowCallbacks,
        workflows::input_documents::{DocumentRow, documents_dataframe},
    };

    #[derive(Debug, Default)]
    struct CountingTokenizer {
        calls: AtomicUsize,
    }

    impl Tokenizer for CountingTokenizer {
        fn encode(&self, _text: &str) -> LlmResult<Vec<u32>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![1, 2, 3])
        }

        fn decode(&self, _tokens: &[u32]) -> LlmResult<String> {
            Err(LlmError::Tokenizer {
                encoding_model: "test".to_owned(),
                message: "decode is not used in this test".to_owned(),
            })
        }
    }

    #[test]
    fn test_should_reuse_chunk_token_count_without_calling_tokenizer() {
        let tokenizer = CountingTokenizer::default();

        assert_eq!(
            resolve_chunk_token_count(Some(7), "text", &tokenizer).expect("count"),
            7
        );
        assert_eq!(tokenizer.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_should_fallback_to_tokenizer_when_chunk_count_is_missing() {
        let tokenizer = CountingTokenizer::default();

        assert_eq!(
            resolve_chunk_token_count(None, "text", &tokenizer).expect("count"),
            3
        );
        assert_eq!(tokenizer.calls.load(Ordering::SeqCst), 1);
    }

    #[derive(Debug, Default)]
    struct ProgressCallbacks {
        calls: Mutex<Vec<(usize, Option<usize>)>>,
    }

    impl IndexWorkflowCallbacks for ProgressCallbacks {
        fn progress(&self, workflow_name: &str, completed: usize, total: Option<usize>) {
            if workflow_name == CREATE_BASE_TEXT_UNITS_WORKFLOW {
                self.calls
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push((completed, total));
            }
        }
    }

    #[tokio::test]
    async fn test_should_report_progress_once_per_document() {
        let provider = Arc::new(MemoryTableProvider::new());
        let documents = vec![
            DocumentRow {
                id: "doc-1".to_owned(),
                human_readable_id: 0,
                title: None,
                text: "alpha beta gamma delta".to_owned(),
                text_unit_ids: Vec::new(),
                creation_date: None,
                raw_data: None,
            },
            DocumentRow {
                id: "doc-2".to_owned(),
                human_readable_id: 1,
                title: None,
                text: "xy".to_owned(),
                text_unit_ids: Vec::new(),
                creation_date: None,
                raw_data: None,
            },
        ];
        provider
            .write_dataframe(
                "documents",
                documents_dataframe(&documents).expect("documents dataframe"),
            )
            .await
            .expect("documents should write");
        let callbacks = Arc::new(ProgressCallbacks::default());
        let mut context =
            IndexPipelineContext::for_test(provider).with_callbacks(callbacks.clone());
        let config = GraphRagConfig {
            chunking: ChunkingConfig::new(NonZeroUsize::new(1).expect("nonzero"), 0, Vec::new())
                .expect("valid chunking config"),
            ..Default::default()
        };

        let output = CreateBaseTextUnitsWorkflow
            .run(&config, &mut context)
            .await
            .expect("workflow should run");

        assert!(output.output_rows > documents.len());
        assert_eq!(context.stats.text_unit_count, output.output_rows);
        assert_eq!(
            *callbacks
                .calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![(1, Some(2)), (2, Some(2))]
        );
    }
}
