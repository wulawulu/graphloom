//! Graph extraction workflow orchestration.

use async_trait::async_trait;
use futures_util::{StreamExt, stream};
use graphloom_llm::{PromptLoader, TiktokenTokenizer};

use super::common::resolve_completion_model;
use crate::{
    GraphLoomError, GraphRagConfig, PipelineRunContext, Result, Workflow, WorkflowFunctionOutput,
    operations::graph::{
        DescriptionSummarizeConfig, entity_intermediate_dataframe, extract_graph_sample,
        extract_text_unit_graph, filter_orphan_relationships, merge_entities, merge_relationships,
        raw_entity_dataframe, raw_relationship_dataframe, read_text_units,
        relationship_intermediate_dataframe, summarize_entities, summarize_relationships,
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
        let concurrency = config.concurrent_requests.max(1);

        let extraction_prompt = config.extract_graph.prompt.clone();
        let entity_types = config.extract_graph.entity_types.clone();
        let max_gleanings = config.extract_graph.max_gleanings;
        let mut extraction_results = stream::iter(text_units.iter().cloned().enumerate())
            .map(|(index, text_unit)| {
                let extractor = extractor.clone();
                let prompt_loader = prompt_loader.clone();
                let extraction_prompt = extraction_prompt.clone();
                let entity_types = entity_types.clone();
                async move {
                    extract_text_unit_graph(
                        extractor.as_ref(),
                        &prompt_loader,
                        extraction_prompt.as_deref(),
                        &entity_types,
                        &text_unit,
                        max_gleanings,
                    )
                    .await
                    .map(|(entities, relationships)| (index, entities, relationships))
                }
            })
            .buffer_unordered(concurrency);

        let mut completed_extractions = 0usize;
        let mut extracted = Vec::with_capacity(text_units.len());
        while let Some(result) = extraction_results.next().await {
            let result = result?;
            completed_extractions = completed_extractions.saturating_add(1);
            context.callbacks.progress(
                EXTRACT_GRAPH_WORKFLOW,
                completed_extractions,
                Some(text_units.len()),
            );
            extracted.push(result);
        }
        extracted.sort_by_key(|(index, _, _)| *index);

        let mut extracted_entities = Vec::new();
        let mut extracted_relationships = Vec::new();
        for (_, entities, relationships) in extracted {
            extracted_entities.extend(entities);
            extracted_relationships.extend(relationships);
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
            DescriptionSummarizeConfig {
                max_length: config.summarize_descriptions.max_length,
                max_input_tokens: config.summarize_descriptions.max_input_tokens,
                concurrency,
            },
            &|completed, total| {
                context
                    .callbacks
                    .progress(EXTRACT_GRAPH_WORKFLOW, completed, Some(total));
            },
        )
        .await?;
        let summarized_relationships = summarize_relationships(
            summarizer.as_ref(),
            &prompt_loader,
            config.summarize_descriptions.prompt.as_deref(),
            &tokenizer,
            &relationships,
            DescriptionSummarizeConfig {
                max_length: config.summarize_descriptions.max_length,
                max_input_tokens: config.summarize_descriptions.max_input_tokens,
                concurrency,
            },
            &|completed, total| {
                context
                    .callbacks
                    .progress(EXTRACT_GRAPH_WORKFLOW, completed, Some(total));
            },
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
                .write_dataframe("raw_entities", raw_entity_dataframe(&entities)?)
                .await?;
            context
                .output_table_provider
                .write_dataframe(
                    "raw_relationships",
                    raw_relationship_dataframe(&relationships)?,
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
