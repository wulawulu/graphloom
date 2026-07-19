//! `GraphRAG` 3.1-compatible Basic Search Sources context.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use graphloom_llm::{EmbeddingModel, EmbeddingRequest, Tokenizer};
use graphloom_vectors::{VectorError, VectorIndexSchema, VectorStore};
use polars_core::prelude::{DataFrame, NamedFrom, Series};

use super::super::{
    QueryContext, QueryContextRecords, QueryContextText, QueryError, QueryUsageCategory, Result,
    SearchMethod, TextUnit,
};
use crate::BasicSearchConfig;

#[derive(Debug)]
pub(crate) struct BasicContextBuilder {
    pub(crate) config: BasicSearchConfig,
    pub(crate) text_units: Vec<TextUnit>,
    pub(crate) embedding_model: Arc<dyn EmbeddingModel>,
    pub(crate) embedding_model_id: String,
    pub(crate) vector_store: Arc<dyn VectorStore>,
    pub(crate) vector_schema: VectorIndexSchema,
    pub(crate) tokenizer: Arc<dyn Tokenizer>,
}

#[derive(Debug)]
pub(crate) struct BasicContextBuild {
    pub(crate) context: QueryContext,
    pub(crate) usage: QueryUsageCategory,
}

impl BasicContextBuilder {
    pub(crate) async fn build(&self, query: &str) -> Result<BasicContextBuild> {
        let (matched_ids, usage) = if query.is_empty() {
            (BTreeSet::new(), QueryUsageCategory::default())
        } else {
            self.retrieve(query).await?
        };
        let candidates = self
            .text_units
            .iter()
            .filter(|text_unit| matched_ids.contains(&text_unit.id))
            .collect::<Vec<_>>();
        let selected = self.within_budget(&candidates)?;
        let context_text = render_sources(&selected)?;
        let ids = selected
            .iter()
            .map(|text_unit| text_unit.short_id.as_str())
            .collect::<Vec<_>>();
        let texts = selected
            .iter()
            .map(|text_unit| text_unit.text.as_str())
            .collect::<Vec<_>>();
        let records = DataFrame::new(
            selected.len(),
            vec![
                Series::new("id".into(), ids).into(),
                Series::new("text".into(), texts).into(),
            ],
        )
        .map_err(|source| QueryError::QueryContext {
            method: SearchMethod::Basic,
            operation: "build Sources records",
            message: source.to_string(),
        })?;
        let context = QueryContext {
            text: QueryContextText::Text(context_text),
            records: QueryContextRecords::Tables(BTreeMap::from([("sources".to_owned(), records)])),
        };
        Ok(BasicContextBuild { context, usage })
    }

    async fn retrieve(&self, query: &str) -> Result<(BTreeSet<String>, QueryUsageCategory)> {
        let response = self
            .embedding_model
            .embed(EmbeddingRequest::new(vec![query.to_owned()]))
            .await
            .map_err(|source| QueryError::QueryEmbedding {
                method: SearchMethod::Basic,
                operation: "embed Basic Search query",
                model: self.embedding_model_id.clone(),
                source: Box::new(source),
            })?;
        let provider_prompt_tokens =
            usize::try_from(response.usage.prompt_tokens).unwrap_or(usize::MAX);
        let prompt_tokens = if provider_prompt_tokens == 0 {
            self.tokenizer
                .count(query)
                .map_err(|source| QueryError::QueryEmbedding {
                    method: SearchMethod::Basic,
                    operation: "count Basic Search embedding input tokens",
                    model: self.embedding_model_id.clone(),
                    source: Box::new(source),
                })?
        } else {
            provider_prompt_tokens
        };
        let vector = response
            .into_embeddings()
            .into_iter()
            .next()
            .ok_or_else(|| QueryError::QueryEmbedding {
                method: SearchMethod::Basic,
                operation: "read Basic Search query embedding",
                model: self.embedding_model_id.clone(),
                source: Box::new(graphloom_llm::LlmError::InvalidResponse {
                    model_instance: self.embedding_model_id.clone(),
                    operation: "embedding conversion",
                    message: "provider returned no query embedding".to_owned(),
                }),
            })?;
        if vector.iter().any(|value| !value.is_finite()) {
            return Err(QueryError::QueryEmbedding {
                method: SearchMethod::Basic,
                operation: "validate Basic Search query embedding",
                model: self.embedding_model_id.clone(),
                source: Box::new(graphloom_llm::LlmError::InvalidResponse {
                    model_instance: self.embedding_model_id.clone(),
                    operation: "embedding conversion",
                    message: "provider returned a non-finite query embedding".to_owned(),
                }),
            });
        }
        let results = self
            .vector_store
            .similarity_search_by_vector(&self.vector_schema, &vector, self.config.k, false)
            .await
            .map_err(|source| match source {
                source @ VectorError::MissingIndex { .. } => QueryError::MissingVectorIndex {
                    method: SearchMethod::Basic,
                    operation: "search text_unit_text",
                    index: self.vector_schema.index_name.clone(),
                    source: Box::new(source),
                },
                source => QueryError::InvalidVectorIndex {
                    method: SearchMethod::Basic,
                    operation: "search text_unit_text",
                    index: self.vector_schema.index_name.clone(),
                    source: Box::new(source),
                },
            })?;
        Ok((
            results
                .into_iter()
                .map(|result| result.document.id)
                .collect(),
            QueryUsageCategory {
                llm_calls: 1,
                prompt_tokens,
                output_tokens: 0,
            },
        ))
    }

    fn within_budget<'a>(&self, candidates: &[&'a TextUnit]) -> Result<Vec<&'a TextUnit>> {
        let mut tokens = self
            .tokenizer
            .count("id|text\n")
            .map_err(|source| context_token_error(&source))?;
        let mut selected = Vec::new();
        for text_unit in candidates {
            let row = format!("{}|{}\n", text_unit.short_id, text_unit.text);
            let row_tokens = self
                .tokenizer
                .count(&row)
                .map_err(|source| context_token_error(&source))?;
            if tokens.saturating_add(row_tokens) > self.config.max_context_tokens {
                break;
            }
            tokens = tokens.saturating_add(row_tokens);
            selected.push(*text_unit);
        }
        Ok(selected)
    }
}

fn render_sources(text_units: &[&TextUnit]) -> Result<String> {
    let mut writer = csv::WriterBuilder::new()
        .delimiter(b'|')
        .escape(b'\\')
        .from_writer(Vec::new());
    writer
        .write_record(["id", "text"])
        .map_err(|source| csv_context_error(&source))?;
    for text_unit in text_units {
        let id = pandas_escape_field(&text_unit.short_id);
        let text = pandas_escape_field(&text_unit.text);
        writer
            .write_record([id.as_ref(), text.as_ref()])
            .map_err(|source| csv_context_error(&source))?;
    }
    let bytes = writer
        .into_inner()
        .map_err(|source| csv_context_error(&source.into_error().into()))?;
    String::from_utf8(bytes).map_err(|source| QueryError::QueryContext {
        method: SearchMethod::Basic,
        operation: "encode Sources CSV",
        message: source.to_string(),
    })
}

fn pandas_escape_field(value: &str) -> std::borrow::Cow<'_, str> {
    if value.contains('\\') {
        std::borrow::Cow::Owned(value.replace('\\', "\\\\"))
    } else {
        std::borrow::Cow::Borrowed(value)
    }
}

fn csv_context_error(source: &csv::Error) -> QueryError {
    QueryError::QueryContext {
        method: SearchMethod::Basic,
        operation: "render Sources CSV",
        message: source.to_string(),
    }
}

fn context_token_error(source: &graphloom_llm::LlmError) -> QueryError {
    QueryError::QueryContext {
        method: SearchMethod::Basic,
        operation: "count Sources tokens",
        message: source.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use graphloom_llm::{EmbeddingResponse, LlmError};
    use graphloom_vectors::{VectorDocument, VectorSearchResult};

    use super::*;

    #[derive(Debug, Default)]
    struct ByteTokenizer;

    impl Tokenizer for ByteTokenizer {
        fn encode(&self, text: &str) -> graphloom_llm::Result<Vec<u32>> {
            Ok(text
                .as_bytes()
                .iter()
                .map(|value| u32::from(*value))
                .collect())
        }

        fn decode(&self, tokens: &[u32]) -> graphloom_llm::Result<String> {
            let bytes = tokens
                .iter()
                .map(|value| u8::try_from(*value))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|source| LlmError::Tokenizer {
                    encoding_model: "bytes".to_owned(),
                    message: source.to_string(),
                })?;
            String::from_utf8(bytes).map_err(|source| LlmError::Tokenizer {
                encoding_model: "bytes".to_owned(),
                message: source.to_string(),
            })
        }
    }

    #[derive(Debug)]
    struct RecordingEmbedding {
        calls: Arc<AtomicUsize>,
        inputs: Arc<Mutex<Vec<Vec<String>>>>,
    }

    #[async_trait]
    impl EmbeddingModel for RecordingEmbedding {
        async fn embed(
            &self,
            request: EmbeddingRequest,
        ) -> graphloom_llm::Result<EmbeddingResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inputs
                .lock()
                .expect("recording mutex")
                .push(request.input);
            Ok(EmbeddingResponse::vectors_for_test(
                "embedding",
                vec![vec![0.25, 0.75]],
            ))
        }
    }

    #[derive(Debug)]
    struct RecordingVectorStore {
        results: Vec<VectorSearchResult>,
        calls: Arc<AtomicUsize>,
        queries: RecordedQueries,
    }

    type VectorQuery = (Vec<f32>, usize, bool);
    type RecordedQueries = Arc<Mutex<Vec<VectorQuery>>>;
    type BuilderFixture = (
        BasicContextBuilder,
        Arc<AtomicUsize>,
        Arc<Mutex<Vec<Vec<String>>>>,
        Arc<AtomicUsize>,
        RecordedQueries,
    );

    #[async_trait]
    impl VectorStore for RecordingVectorStore {
        async fn ensure_index(&self, _schema: &VectorIndexSchema) -> graphloom_vectors::Result<()> {
            Ok(())
        }

        async fn reset_index(&self, _schema: &VectorIndexSchema) -> graphloom_vectors::Result<()> {
            Ok(())
        }

        async fn upsert_documents(
            &self,
            _schema: &VectorIndexSchema,
            _documents: &[VectorDocument],
        ) -> graphloom_vectors::Result<()> {
            Ok(())
        }

        async fn count(&self, _schema: &VectorIndexSchema) -> graphloom_vectors::Result<usize> {
            Ok(self.results.len())
        }

        async fn ids(&self, _schema: &VectorIndexSchema) -> graphloom_vectors::Result<Vec<String>> {
            Ok(self
                .results
                .iter()
                .map(|result| result.document.id.clone())
                .collect())
        }

        async fn get_by_id(
            &self,
            _schema: &VectorIndexSchema,
            id: &str,
        ) -> graphloom_vectors::Result<Option<VectorDocument>> {
            Ok(self
                .results
                .iter()
                .find(|result| result.document.id == id)
                .map(|result| result.document.clone()))
        }

        async fn similarity_search_by_vector(
            &self,
            _schema: &VectorIndexSchema,
            query_vector: &[f32],
            k: usize,
            include_vectors: bool,
        ) -> graphloom_vectors::Result<Vec<VectorSearchResult>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.queries.lock().expect("recording mutex").push((
                query_vector.to_vec(),
                k,
                include_vectors,
            ));
            Ok(self.results.clone())
        }
    }

    fn text_unit(id: &str, short_id: &str, text: &str) -> TextUnit {
        TextUnit {
            id: id.to_owned(),
            short_id: short_id.to_owned(),
            text: text.to_owned(),
            entity_ids: Vec::new(),
            relationship_ids: Vec::new(),
            covariate_ids: Vec::new(),
            n_tokens: None,
            document_id: None,
        }
    }

    fn result(id: &str, score: f32) -> VectorSearchResult {
        VectorSearchResult {
            document: VectorDocument {
                id: id.to_owned(),
                vector: Vec::new(),
            },
            score,
        }
    }

    fn builder(
        text_units: Vec<TextUnit>,
        results: Vec<VectorSearchResult>,
        max_context_tokens: usize,
    ) -> BuilderFixture {
        let embedding_calls = Arc::new(AtomicUsize::new(0));
        let embedding_inputs = Arc::new(Mutex::new(Vec::new()));
        let vector_calls = Arc::new(AtomicUsize::new(0));
        let vector_queries = Arc::new(Mutex::new(Vec::new()));
        let config = BasicSearchConfig {
            k: 2,
            max_context_tokens,
            ..BasicSearchConfig::default()
        };
        (
            BasicContextBuilder {
                config,
                text_units,
                embedding_model: Arc::new(RecordingEmbedding {
                    calls: Arc::clone(&embedding_calls),
                    inputs: Arc::clone(&embedding_inputs),
                }),
                embedding_model_id: "embedding".to_owned(),
                vector_store: Arc::new(RecordingVectorStore {
                    results,
                    calls: Arc::clone(&vector_calls),
                    queries: Arc::clone(&vector_queries),
                }),
                vector_schema: VectorIndexSchema::for_embedding_name("text_unit_text", 2),
                tokenizer: Arc::new(ByteTokenizer),
            },
            embedding_calls,
            embedding_inputs,
            vector_calls,
            vector_queries,
        )
    }

    fn context_text(build: &BasicContextBuild) -> &str {
        match &build.context.text {
            QueryContextText::Text(value) => value,
            _ => panic!("expected Basic text context"),
        }
    }

    #[tokio::test]
    async fn test_should_use_ann_as_id_set_and_preserve_text_unit_table_order() {
        let (builder, _, embedding_inputs, _, vector_queries) = builder(
            vec![text_unit("A", "0", "first"), text_unit("B", "1", "second")],
            vec![result("B", 0.9), result("A", 0.8)],
            usize::MAX,
        );

        let built = builder.build("question").await.expect("context");

        assert_eq!(context_text(&built), "id|text\n0|first\n1|second\n");
        assert_eq!(
            built.usage,
            QueryUsageCategory {
                llm_calls: 1,
                prompt_tokens: "question".len(),
                output_tokens: 0,
            }
        );
        assert_eq!(
            embedding_inputs.lock().expect("inputs").as_slice(),
            &[vec!["question".to_owned()]]
        );
        assert_eq!(
            vector_queries.lock().expect("queries").as_slice(),
            &[(vec![0.25, 0.75], 2, false)]
        );
    }

    #[tokio::test]
    async fn test_should_match_pandas_sources_csv_escaping_golden() {
        let texts = [
            "pipe|value",
            "quote \"value\"",
            "back\\slash",
            "line1\nline2",
            "",
            "Unicode 世界",
        ];
        let units = texts
            .iter()
            .enumerate()
            .map(|(index, text)| text_unit(&format!("id-{index}"), &index.to_string(), text))
            .collect::<Vec<_>>();
        let results = (0..texts.len())
            .rev()
            .map(|index| result(&format!("id-{index}"), 1.0))
            .collect::<Vec<_>>();
        let (builder, _, _, _, _) = builder(units, results, usize::MAX);

        let built = builder.build("question").await.expect("context");

        assert_eq!(
            context_text(&built),
            "id|text\n0|\"pipe|value\"\n1|\"quote \
             \"\"value\"\"\"\n2|back\\\\slash\n3|\"line1\nline2\"\n4|\n5|Unicode 世界\n"
        );
    }

    #[tokio::test]
    async fn test_should_count_header_and_keep_only_whole_rows_within_budget() {
        let (builder, _, _, _, _) = builder(
            vec![text_unit("A", "0", "A"), text_unit("B", "1", "BBBB")],
            vec![result("A", 1.0), result("B", 0.9)],
            "id|text\n0|A\n".len(),
        );

        let built = builder.build("question").await.expect("context");

        assert_eq!(context_text(&built), "id|text\n0|A\n");
    }

    #[tokio::test]
    async fn test_should_skip_retrieval_only_for_exactly_empty_query() {
        let (builder, embedding_calls, _, vector_calls, _) = builder(
            vec![text_unit("A", "0", "A")],
            vec![result("A", 1.0)],
            usize::MAX,
        );

        let empty = builder.build("").await.expect("empty context");
        assert_eq!(context_text(&empty), "id|text\n");
        assert_eq!(empty.usage, QueryUsageCategory::default());
        assert_eq!(embedding_calls.load(Ordering::SeqCst), 0);
        assert_eq!(vector_calls.load(Ordering::SeqCst), 0);

        builder.build("   ").await.expect("whitespace query");
        assert_eq!(embedding_calls.load(Ordering::SeqCst), 1);
        assert_eq!(vector_calls.load(Ordering::SeqCst), 1);
    }
}
