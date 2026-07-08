//! Graph extraction workflow orchestration.

use async_trait::async_trait;
use graphloom_llm::{PromptLoader, TiktokenTokenizer};

use super::common::resolve_completion_model;
use crate::{
    GraphLoomError, GraphRagConfig, PipelineRunContext, Result, Workflow, WorkflowFunctionOutput,
    operations::graph::{
        entity_intermediate_dataframe, extract_graph_sample, extract_text_unit_graph,
        filter_orphan_relationships, merge_entities, merge_relationships, raw_entity_dataframe,
        raw_relationship_dataframe, read_text_units, relationship_intermediate_dataframe,
        summarize_entities, summarize_relationships,
    },
};

/// Workflow name.
pub const EXTRACT_GRAPH_WORKFLOW: &str = "extract_graph";

/// Extract entity and relationship graph rows from text units.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExtractGraphWorkflow;

#[async_trait]
impl Workflow for ExtractGraphWorkflow {
    fn name(&self) -> &'static str {
        EXTRACT_GRAPH_WORKFLOW
    }

    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut PipelineRunContext,
    ) -> Result<WorkflowFunctionOutput> {
        let text_units = read_text_units(
            &context
                .output_table_provider
                .read_dataframe("text_units")
                .await?,
        )?;
        let extractor = resolve_completion_model(
            config,
            context,
            &config.extract_graph.completion_model_id,
            &config.extract_graph.model_instance_name,
            EXTRACT_GRAPH_WORKFLOW,
        )?;
        let summarizer = resolve_completion_model(
            config,
            context,
            &config.summarize_descriptions.completion_model_id,
            &config.summarize_descriptions.model_instance_name,
            EXTRACT_GRAPH_WORKFLOW,
        )?;
        let tokenizer = TiktokenTokenizer::new(&config.chunking.encoding_model)?;
        let prompt_loader = PromptLoader::new(".");

        let mut extracted_entities = Vec::new();
        let mut extracted_relationships = Vec::new();
        for (index, text_unit) in text_units.iter().enumerate() {
            let (entities, relationships) = extract_text_unit_graph(
                extractor.as_ref(),
                &prompt_loader,
                config.extract_graph.prompt.as_deref(),
                &config.extract_graph.entity_types,
                text_unit,
                config.extract_graph.max_gleanings,
            )
            .await?;
            extracted_entities.extend(entities);
            extracted_relationships.extend(relationships);
            context.callbacks.progress(
                EXTRACT_GRAPH_WORKFLOW,
                index.saturating_add(1),
                Some(text_units.len()),
            );
        }

        let entities = merge_entities(&extracted_entities);
        let relationships =
            filter_orphan_relationships(merge_relationships(&extracted_relationships), &entities);
        if entities.is_empty() {
            return Err(GraphLoomError::InvalidData {
                workflow: EXTRACT_GRAPH_WORKFLOW,
                message: "Graph Extraction failed. No entities detected during extraction."
                    .to_owned(),
            });
        }
        if relationships.is_empty() {
            return Err(GraphLoomError::InvalidData {
                workflow: EXTRACT_GRAPH_WORKFLOW,
                message: "Graph Extraction failed. No relationships detected during extraction."
                    .to_owned(),
            });
        }

        let summarized_entities = summarize_entities(
            summarizer.as_ref(),
            &prompt_loader,
            config.summarize_descriptions.prompt.as_deref(),
            &tokenizer,
            &entities,
            config.summarize_descriptions.max_length,
            config.summarize_descriptions.max_input_tokens,
        )
        .await?;
        let summarized_relationships = summarize_relationships(
            summarizer.as_ref(),
            &prompt_loader,
            config.summarize_descriptions.prompt.as_deref(),
            &tokenizer,
            &relationships,
            config.summarize_descriptions.max_length,
            config.summarize_descriptions.max_input_tokens,
        )
        .await?;

        context
            .output_table_provider
            .write_dataframe(
                "entities",
                entity_intermediate_dataframe(&summarized_entities)?,
            )
            .await?;
        context
            .output_table_provider
            .write_dataframe(
                "relationships",
                relationship_intermediate_dataframe(&summarized_relationships)?,
            )
            .await?;

        if config.snapshots.raw_graph {
            context
                .output_table_provider
                .write_dataframe("raw_entities", raw_entity_dataframe(&extracted_entities)?)
                .await?;
            context
                .output_table_provider
                .write_dataframe(
                    "raw_relationships",
                    raw_relationship_dataframe(&extracted_relationships)?,
                )
                .await?;
        }

        context.stats.entity_count = summarized_entities.len();
        context.stats.relationship_count = summarized_relationships.len();
        Ok(WorkflowFunctionOutput {
            result: extract_graph_sample(&summarized_entities, &summarized_relationships),
            stop: false,
            input_rows: text_units.len(),
            output_rows: summarized_entities
                .len()
                .saturating_add(summarized_relationships.len()),
        })
    }
}
