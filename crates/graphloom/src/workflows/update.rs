//! `GraphRAG` 3.1.0-compatible incremental update workflows.

use std::{collections::BTreeSet, path::Path, sync::Arc};

use async_trait::async_trait;
use futures_util::TryStreamExt;
use graphloom_llm::TiktokenTokenizer;
use graphloom_storage::TableProvider;
use polars_core::prelude::DataFrame;

use super::{
    GenerateTextEmbeddingsWorkflow,
    common::resolve_completion_model,
    generate_text_embeddings::run_text_embeddings,
    input_documents::{DocumentRow, documents_dataframe},
};
use crate::{
    GraphLoomError, GraphRagConfig, IndexPipelineContext, IndexWorkflow, IndexWorkflowOutput,
    IndexWorkflowRegistry, IndexWorkflowRequirements, Result,
    dataframe::usize_to_i64,
    operations::{
        graph::{DescriptionSummarizeConfig, summarize_entities, summarize_relationships},
        update::{
            concatenate_with_rebased_ids, entity_summary_inputs, merge_communities,
            merge_community_reports, merge_entities, merge_relationships, merge_text_units,
            relationship_summary_inputs, summarized_entities_dataframe,
            summarized_relationships_dataframe,
        },
    },
    prompts::{PromptKind, PromptRepository},
};

/// Load current input and retain documents whose titles are absent from the previous index.
pub const LOAD_UPDATE_DOCUMENTS_WORKFLOW: &str = "load_update_documents";
/// Merge delta documents into final output.
pub const UPDATE_FINAL_DOCUMENTS_WORKFLOW: &str = "update_final_documents";
/// Merge and resummarize entities and relationships.
pub const UPDATE_ENTITIES_RELATIONSHIPS_WORKFLOW: &str = "update_entities_relationships";
/// Merge text units and remap entity IDs.
pub const UPDATE_TEXT_UNITS_WORKFLOW: &str = "update_text_units";
/// Merge optional covariates.
pub const UPDATE_COVARIATES_WORKFLOW: &str = "update_covariates";
/// Rebase and merge communities.
pub const UPDATE_COMMUNITIES_WORKFLOW: &str = "update_communities";
/// Rebase and merge community reports.
pub const UPDATE_COMMUNITY_REPORTS_WORKFLOW: &str = "update_community_reports";
/// Regenerate configured embeddings from final tables.
pub const UPDATE_TEXT_EMBEDDINGS_WORKFLOW: &str = "update_text_embeddings";
/// Clear update-only ID mappings.
pub const UPDATE_CLEAN_STATE_WORKFLOW: &str = "update_clean_state";

/// Update-only merge workflow suffix.
pub const UPDATE_WORKFLOWS: &[&str] = &[
    UPDATE_FINAL_DOCUMENTS_WORKFLOW,
    UPDATE_ENTITIES_RELATIONSHIPS_WORKFLOW,
    UPDATE_TEXT_UNITS_WORKFLOW,
    UPDATE_COVARIATES_WORKFLOW,
    UPDATE_COMMUNITIES_WORKFLOW,
    UPDATE_COMMUNITY_REPORTS_WORKFLOW,
    UPDATE_TEXT_EMBEDDINGS_WORKFLOW,
    UPDATE_CLEAN_STATE_WORKFLOW,
];

/// Register all update-specific workflows.
pub fn register_update_workflows(registry: &mut IndexWorkflowRegistry) -> Result<()> {
    registry.register(LoadUpdateDocumentsWorkflow)?;
    registry.register(UpdateFinalDocumentsWorkflow)?;
    registry.register(UpdateEntitiesRelationshipsWorkflow)?;
    registry.register(UpdateTextUnitsWorkflow)?;
    registry.register(UpdateCovariatesWorkflow)?;
    registry.register(UpdateCommunitiesWorkflow)?;
    registry.register(UpdateCommunityReportsWorkflow)?;
    registry.register(UpdateTextEmbeddingsWorkflow)?;
    registry.register(UpdateCleanStateWorkflow)
}

/// Incremental input loader.
#[derive(Debug, Clone, Copy, Default)]
pub struct LoadUpdateDocumentsWorkflow;

#[async_trait]
impl IndexWorkflow for LoadUpdateDocumentsWorkflow {
    fn name(&self) -> &'static str {
        LOAD_UPDATE_DOCUMENTS_WORKFLOW
    }

    async fn run(
        &self,
        _config: &GraphRagConfig,
        context: &mut IndexPipelineContext,
    ) -> Result<IndexWorkflowOutput> {
        let previous = Arc::clone(
            &context
                .update_state(LOAD_UPDATE_DOCUMENTS_WORKFLOW)?
                .previous_table_provider,
        );
        let previous_documents = previous.read_dataframe("documents").await?;
        let previous_titles = document_titles(&previous_documents)?;
        let reader = context.input_reader();
        let mut stream = reader.read_documents();
        let mut input_index = 0usize;
        let mut new_rows = Vec::new();
        let mut sample = Vec::new();

        while let Some(document) = stream.try_next().await? {
            let human_readable_id = usize_to_i64(
                input_index,
                LOAD_UPDATE_DOCUMENTS_WORKFLOW,
                "human_readable_id",
            )?;
            input_index = input_index.saturating_add(1);
            if previous_titles.contains(&document.title) {
                continue;
            }
            let raw_data = document
                .raw_data
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            let row = DocumentRow {
                id: document.id,
                human_readable_id,
                title: Some(document.title),
                text: document.text,
                text_unit_ids: Vec::new(),
                creation_date: document.creation_date,
                raw_data,
            };
            if sample.len() < 5 {
                sample.push(row.to_value());
            }
            new_rows.push(row);
        }

        context.stats.update_document_count = new_rows.len();
        if new_rows.is_empty() {
            return Ok(IndexWorkflowOutput {
                result: Vec::new(),
                stop: true,
                input_rows: input_index,
                output_rows: 0,
            });
        }
        context
            .output_table_provider()
            .write_dataframe("documents", documents_dataframe(&new_rows)?)
            .await?;
        Ok(IndexWorkflowOutput {
            result: sample,
            stop: false,
            input_rows: input_index,
            output_rows: new_rows.len(),
        })
    }
}

fn document_titles(dataframe: &DataFrame) -> Result<BTreeSet<String>> {
    let titles = dataframe.column("title")?.str()?;
    Ok((0..dataframe.height())
        .filter_map(|index| titles.get(index).map(str::to_owned))
        .collect())
}

#[derive(Debug, Clone, Copy, Default)]
struct UpdateFinalDocumentsWorkflow;

#[async_trait]
impl IndexWorkflow for UpdateFinalDocumentsWorkflow {
    fn name(&self) -> &'static str {
        UPDATE_FINAL_DOCUMENTS_WORKFLOW
    }

    async fn run(
        &self,
        _config: &GraphRagConfig,
        context: &mut IndexPipelineContext,
    ) -> Result<IndexWorkflowOutput> {
        let (previous, delta, final_provider) =
            update_providers(context, UPDATE_FINAL_DOCUMENTS_WORKFLOW)?;
        let old = previous.read_dataframe("documents").await?;
        let new = delta.read_dataframe("documents").await?;
        let merged = concatenate_with_rebased_ids(&old, &new)?;
        final_provider
            .write_dataframe("documents", merged.clone())
            .await?;
        context.stats.document_count = merged.height();
        Ok(workflow_output(
            old.height() + new.height(),
            merged.height(),
        ))
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct UpdateEntitiesRelationshipsWorkflow;

#[async_trait]
impl IndexWorkflow for UpdateEntitiesRelationshipsWorkflow {
    fn name(&self) -> &'static str {
        UPDATE_ENTITIES_RELATIONSHIPS_WORKFLOW
    }

    fn requirements(&self, config: &GraphRagConfig) -> Result<IndexWorkflowRequirements> {
        let mut requirements = IndexWorkflowRequirements::default();
        requirements.require_completion_model(&config.summarize_descriptions.completion_model_id);
        requirements.require_prompt(
            PromptKind::SummarizeDescriptions,
            config.summarize_descriptions.prompt.clone(),
        );
        requirements.require_tokenizer("chunking.encoding_model", &config.chunking.encoding_model);
        Ok(requirements)
    }

    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut IndexPipelineContext,
    ) -> Result<IndexWorkflowOutput> {
        let (previous, delta, final_provider) =
            update_providers(context, UPDATE_ENTITIES_RELATIONSHIPS_WORKFLOW)?;
        let old_entities = previous.read_dataframe("entities").await?;
        let delta_entities = delta.read_dataframe("entities").await?;
        let (merged_entities, mapping) = merge_entities(&old_entities, &delta_entities)?;
        let entity_titles = merged_entities
            .iter()
            .map(|row| row.title.clone())
            .collect::<BTreeSet<_>>();
        let old_relationships = previous.read_dataframe("relationships").await?;
        let delta_relationships = delta.read_dataframe("relationships").await?;
        let merged_relationships =
            merge_relationships(&old_relationships, &delta_relationships, &entity_titles)?;

        let model = resolve_completion_model(
            context,
            &config.summarize_descriptions.completion_model_id,
            &config.summarize_descriptions.model_instance_name,
            UPDATE_ENTITIES_RELATIONSHIPS_WORKFLOW,
        )?;
        let tokenizer = TiktokenTokenizer::new(&config.chunking.encoding_model)?;
        let prompt = PromptRepository::new(context.prompt_root())
            .load(
                PromptKind::SummarizeDescriptions,
                config
                    .summarize_descriptions
                    .prompt
                    .as_deref()
                    .map(Path::new),
            )
            .await?;
        let summarize_config = DescriptionSummarizeConfig {
            max_length: config.summarize_descriptions.max_length,
            max_input_tokens: config.summarize_descriptions.max_input_tokens,
            concurrency: config.concurrent_requests.max(1),
        };
        let entity_inputs = entity_summary_inputs(&merged_entities);
        let entity_summaries = summarize_entities(
            model.as_ref(),
            &prompt,
            &tokenizer,
            &entity_inputs,
            summarize_config,
            &|completed, total| {
                context.callbacks.progress(
                    UPDATE_ENTITIES_RELATIONSHIPS_WORKFLOW,
                    completed,
                    Some(total.saturating_add(merged_relationships.len())),
                );
            },
        )
        .await?;
        let relationship_inputs = relationship_summary_inputs(&merged_relationships);
        let relationship_summaries = summarize_relationships(
            model.as_ref(),
            &prompt,
            &tokenizer,
            &relationship_inputs,
            summarize_config,
            &|completed, total| {
                context.callbacks.progress(
                    UPDATE_ENTITIES_RELATIONSHIPS_WORKFLOW,
                    entity_inputs.len().saturating_add(completed),
                    Some(entity_inputs.len().saturating_add(total)),
                );
            },
        )
        .await?;
        final_provider
            .write_dataframe(
                "entities",
                summarized_entities_dataframe(&merged_entities, &entity_summaries)?,
            )
            .await?;
        final_provider
            .write_dataframe(
                "relationships",
                summarized_relationships_dataframe(&merged_relationships, &relationship_summaries)?,
            )
            .await?;
        context
            .update_state_mut(UPDATE_ENTITIES_RELATIONSHIPS_WORKFLOW)?
            .entity_id_mapping = Some(mapping);
        context.stats.entity_count = merged_entities.len();
        context.stats.relationship_count = merged_relationships.len();
        Ok(workflow_output(
            old_entities
                .height()
                .saturating_add(delta_entities.height())
                .saturating_add(old_relationships.height())
                .saturating_add(delta_relationships.height()),
            merged_entities
                .len()
                .saturating_add(merged_relationships.len()),
        ))
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct UpdateTextUnitsWorkflow;

#[async_trait]
impl IndexWorkflow for UpdateTextUnitsWorkflow {
    fn name(&self) -> &'static str {
        UPDATE_TEXT_UNITS_WORKFLOW
    }

    async fn run(
        &self,
        _config: &GraphRagConfig,
        context: &mut IndexPipelineContext,
    ) -> Result<IndexWorkflowOutput> {
        let (previous, delta, final_provider) =
            update_providers(context, UPDATE_TEXT_UNITS_WORKFLOW)?;
        let mapping = context
            .update_state(UPDATE_TEXT_UNITS_WORKFLOW)?
            .entity_id_mapping
            .clone()
            .ok_or(GraphLoomError::MissingUpdateIdMapping {
                mapping: "entity",
                workflow: UPDATE_TEXT_UNITS_WORKFLOW,
            })?;
        let old = previous.read_dataframe("text_units").await?;
        let new = delta.read_dataframe("text_units").await?;
        let merged = merge_text_units(&old, &new, &mapping)?;
        final_provider
            .write_dataframe("text_units", merged.clone())
            .await?;
        context.stats.text_unit_count = merged.height();
        Ok(workflow_output(
            old.height() + new.height(),
            merged.height(),
        ))
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct UpdateCovariatesWorkflow;

#[async_trait]
impl IndexWorkflow for UpdateCovariatesWorkflow {
    fn name(&self) -> &'static str {
        UPDATE_COVARIATES_WORKFLOW
    }

    async fn run(
        &self,
        _config: &GraphRagConfig,
        context: &mut IndexPipelineContext,
    ) -> Result<IndexWorkflowOutput> {
        let (previous, delta, final_provider) =
            update_providers(context, UPDATE_COVARIATES_WORKFLOW)?;
        if !previous.has("covariates").await? || !delta.has("covariates").await? {
            return Ok(workflow_output(0, 0));
        }
        let old = previous.read_dataframe("covariates").await?;
        let new = delta.read_dataframe("covariates").await?;
        let merged = concatenate_with_rebased_ids(&old, &new)?;
        final_provider
            .write_dataframe("covariates", merged.clone())
            .await?;
        Ok(workflow_output(
            old.height() + new.height(),
            merged.height(),
        ))
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct UpdateCommunitiesWorkflow;

#[async_trait]
impl IndexWorkflow for UpdateCommunitiesWorkflow {
    fn name(&self) -> &'static str {
        UPDATE_COMMUNITIES_WORKFLOW
    }

    async fn run(
        &self,
        _config: &GraphRagConfig,
        context: &mut IndexPipelineContext,
    ) -> Result<IndexWorkflowOutput> {
        let (previous, delta, final_provider) =
            update_providers(context, UPDATE_COMMUNITIES_WORKFLOW)?;
        let old = previous.read_dataframe("communities").await?;
        let new = delta.read_dataframe("communities").await?;
        let (merged, mapping) = merge_communities(&old, &new)?;
        final_provider
            .write_dataframe("communities", merged.clone())
            .await?;
        context
            .update_state_mut(UPDATE_COMMUNITIES_WORKFLOW)?
            .community_id_mapping = Some(mapping);
        context.stats.community_count = merged.height();
        Ok(workflow_output(
            old.height() + new.height(),
            merged.height(),
        ))
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct UpdateCommunityReportsWorkflow;

#[async_trait]
impl IndexWorkflow for UpdateCommunityReportsWorkflow {
    fn name(&self) -> &'static str {
        UPDATE_COMMUNITY_REPORTS_WORKFLOW
    }

    async fn run(
        &self,
        _config: &GraphRagConfig,
        context: &mut IndexPipelineContext,
    ) -> Result<IndexWorkflowOutput> {
        let (previous, delta, final_provider) =
            update_providers(context, UPDATE_COMMUNITY_REPORTS_WORKFLOW)?;
        let mapping = context
            .update_state(UPDATE_COMMUNITY_REPORTS_WORKFLOW)?
            .community_id_mapping
            .clone()
            .ok_or(GraphLoomError::MissingUpdateIdMapping {
                mapping: "community",
                workflow: UPDATE_COMMUNITY_REPORTS_WORKFLOW,
            })?;
        let old = previous.read_dataframe("community_reports").await?;
        let new = delta.read_dataframe("community_reports").await?;
        let merged = merge_community_reports(&old, &new, &mapping)?;
        final_provider
            .write_dataframe("community_reports", merged.clone())
            .await?;
        context.stats.report_count = merged.height();
        Ok(workflow_output(
            old.height() + new.height(),
            merged.height(),
        ))
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct UpdateTextEmbeddingsWorkflow;

#[async_trait]
impl IndexWorkflow for UpdateTextEmbeddingsWorkflow {
    fn name(&self) -> &'static str {
        UPDATE_TEXT_EMBEDDINGS_WORKFLOW
    }

    fn requirements(&self, config: &GraphRagConfig) -> Result<IndexWorkflowRequirements> {
        GenerateTextEmbeddingsWorkflow.requirements(config)
    }

    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut IndexPipelineContext,
    ) -> Result<IndexWorkflowOutput> {
        let final_provider = Arc::clone(
            &context
                .update_state(UPDATE_TEXT_EMBEDDINGS_WORKFLOW)?
                .final_table_provider,
        );
        let active = context.replace_output_table_provider(final_provider);
        let result = run_text_embeddings(UPDATE_TEXT_EMBEDDINGS_WORKFLOW, config, context).await;
        context.replace_output_table_provider(active);
        result.map_err(|source| GraphLoomError::UpdateVector {
            source: Box::new(source),
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct UpdateCleanStateWorkflow;

#[async_trait]
impl IndexWorkflow for UpdateCleanStateWorkflow {
    fn name(&self) -> &'static str {
        UPDATE_CLEAN_STATE_WORKFLOW
    }

    async fn run(
        &self,
        _config: &GraphRagConfig,
        context: &mut IndexPipelineContext,
    ) -> Result<IndexWorkflowOutput> {
        context
            .update_state_mut(UPDATE_CLEAN_STATE_WORKFLOW)?
            .clear_temporary_state();
        Ok(workflow_output(0, 0))
    }
}

type UpdateProviders = (
    Arc<dyn TableProvider>,
    Arc<dyn TableProvider>,
    Arc<dyn TableProvider>,
);

fn update_providers(
    context: &IndexPipelineContext,
    workflow: &'static str,
) -> Result<UpdateProviders> {
    let state = context.update_state(workflow)?;
    Ok((
        Arc::clone(&state.previous_table_provider),
        Arc::clone(&state.delta_table_provider),
        Arc::clone(&state.final_table_provider),
    ))
}

fn workflow_output(input_rows: usize, output_rows: usize) -> IndexWorkflowOutput {
    IndexWorkflowOutput {
        result: Vec::new(),
        stop: false,
        input_rows,
        output_rows,
    }
}

#[cfg(test)]
mod tests {
    use std::{pin::Pin, sync::Arc};

    use futures_util::{Stream, stream};
    use graphloom_input::{DocumentStream, InputReader, TextDocument};
    use graphloom_storage::{MemoryTableProvider, TableProvider};

    use super::*;
    use crate::{
        context::UpdateRuntimeState,
        workflows::input_documents::{DocumentRow, documents_dataframe},
    };

    #[derive(Debug)]
    struct StaticInputReader {
        documents: Vec<TextDocument>,
    }

    impl InputReader for StaticInputReader {
        fn read_documents(&self) -> DocumentStream<'_> {
            Box::pin(stream::iter(self.documents.clone().into_iter().map(Ok)))
                as Pin<Box<dyn Stream<Item = _> + Send + '_>>
        }
    }

    #[tokio::test]
    async fn test_should_ignore_modified_and_deleted_titles_and_preserve_input_row_ids() {
        let previous = Arc::new(MemoryTableProvider::new());
        previous
            .write_dataframe(
                "documents",
                documents_dataframe(&[
                    document_row("old-a", 0, "A", "old body"),
                    document_row("old-deleted", 1, "DELETED", "removed input"),
                ])
                .expect("previous documents"),
            )
            .await
            .expect("write previous");
        let delta = Arc::new(MemoryTableProvider::new());
        let final_provider = Arc::new(MemoryTableProvider::new());
        let reader = Arc::new(StaticInputReader {
            documents: vec![
                document("new-a", "A", "changed body", None),
                document(
                    "new-b",
                    "B",
                    "new body",
                    Some(serde_json::json!({"source": "fixture"})),
                ),
                document("new-c", "C", "another body", None),
            ],
        });
        let mut context = IndexPipelineContext::for_test(delta.clone())
            .with_input_reader(reader)
            .with_update_state(UpdateRuntimeState {
                timestamp: "20260724-120000".to_owned(),
                previous_table_provider: previous,
                delta_table_provider: delta.clone(),
                final_table_provider: final_provider,
                entity_id_mapping: None,
                community_id_mapping: None,
            });

        let output = LoadUpdateDocumentsWorkflow
            .run(&GraphRagConfig::default(), &mut context)
            .await
            .expect("load update documents");

        assert!(!output.stop);
        assert_eq!(output.input_rows, 3);
        assert_eq!(output.output_rows, 2);
        assert_eq!(context.stats.update_document_count, 2);
        let rows = delta
            .read_dataframe("documents")
            .await
            .expect("delta documents");
        assert_eq!(
            document_titles(&rows).expect("titles"),
            BTreeSet::from(["B".to_owned(), "C".to_owned(),])
        );
        assert_eq!(
            rows.column("human_readable_id")
                .expect("human IDs")
                .i64()
                .expect("i64")
                .into_no_null_iter()
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(rows.column("raw_data").is_ok());
    }

    #[tokio::test]
    async fn test_should_stop_without_writing_delta_when_titles_are_unchanged() {
        let previous = Arc::new(MemoryTableProvider::new());
        previous
            .write_dataframe(
                "documents",
                documents_dataframe(&[document_row("old", 0, "A", "old")])
                    .expect("previous documents"),
            )
            .await
            .expect("write previous");
        let delta = Arc::new(MemoryTableProvider::new());
        let mut context = update_context(
            previous,
            delta.clone(),
            vec![document("new-id", "A", "changed", None)],
        );

        let output = LoadUpdateDocumentsWorkflow
            .run(&GraphRagConfig::default(), &mut context)
            .await
            .expect("unchanged title is a no-op");

        assert!(output.stop);
        assert_eq!(context.stats.update_document_count, 0);
        assert!(!delta.has("documents").await.expect("delta lookup"));
    }

    #[tokio::test]
    async fn test_should_fail_when_previous_documents_are_missing() {
        let previous = Arc::new(MemoryTableProvider::new());
        let delta = Arc::new(MemoryTableProvider::new());
        let mut context =
            update_context(previous, delta, vec![document("new-id", "A", "body", None)]);

        let error = LoadUpdateDocumentsWorkflow
            .run(&GraphRagConfig::default(), &mut context)
            .await
            .expect_err("missing previous documents must not become a full index");

        assert!(error.to_string().contains("table documents does not exist"));
    }

    #[tokio::test]
    async fn test_should_reject_update_workflow_in_standard_context() {
        let mut context = IndexPipelineContext::for_test(Arc::new(MemoryTableProvider::new()));

        let error = UpdateCleanStateWorkflow
            .run(&GraphRagConfig::default(), &mut context)
            .await
            .expect_err("standard context must reject update state access");

        assert!(matches!(
            error,
            GraphLoomError::MissingUpdateContext {
                workflow: UPDATE_CLEAN_STATE_WORKFLOW,
            }
        ));
    }

    fn update_context(
        previous: Arc<MemoryTableProvider>,
        delta: Arc<MemoryTableProvider>,
        documents: Vec<TextDocument>,
    ) -> IndexPipelineContext {
        IndexPipelineContext::for_test(delta.clone())
            .with_input_reader(Arc::new(StaticInputReader { documents }))
            .with_update_state(UpdateRuntimeState {
                timestamp: "20260724-120000".to_owned(),
                previous_table_provider: previous,
                delta_table_provider: delta,
                final_table_provider: Arc::new(MemoryTableProvider::new()),
                entity_id_mapping: None,
                community_id_mapping: None,
            })
    }

    fn document(
        id: &str,
        title: &str,
        text: &str,
        raw_data: Option<serde_json::Value>,
    ) -> TextDocument {
        TextDocument::new(
            id.to_owned(),
            text.to_owned(),
            title.to_owned(),
            None,
            raw_data,
        )
    }

    fn document_row(id: &str, human_readable_id: i64, title: &str, text: &str) -> DocumentRow {
        DocumentRow {
            id: id.to_owned(),
            human_readable_id,
            title: Some(title.to_owned()),
            text: text.to_owned(),
            text_unit_ids: Vec::new(),
            creation_date: None,
            raw_data: None,
        }
    }
}
