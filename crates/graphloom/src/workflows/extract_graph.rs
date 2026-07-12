//! Graph extraction workflow orchestration.

use std::path::Path;

use async_trait::async_trait;
use graphloom_llm::TiktokenTokenizer;

use super::common::resolve_completion_model;
use crate::{
    GraphRagConfig, IndexPipelineContext, IndexWorkflow, IndexWorkflowOutput,
    IndexWorkflowRequirements, Result,
    operations::graph::{
        DescriptionSummarizeConfig, ExtractedGraph, GraphExtractionConfig, SummarizedGraph,
        entity_intermediate_dataframe, extract_graph, extract_graph_sample, raw_entity_dataframe,
        raw_relationship_dataframe, read_text_units, relationship_intermediate_dataframe,
        summarize_graph,
    },
    prompts::{PromptKind, PromptRepository},
};

/// IndexWorkflow name.
pub const EXTRACT_GRAPH_WORKFLOW: &str = "extract_graph";

/// Extract entity and relationship graph rows from text units.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExtractGraphWorkflow;

#[async_trait]
impl IndexWorkflow for ExtractGraphWorkflow {
    fn name(&self) -> &'static str {
        EXTRACT_GRAPH_WORKFLOW
    }

    fn requirements(&self, config: &GraphRagConfig) -> Result<IndexWorkflowRequirements> {
        let mut requirements = IndexWorkflowRequirements::default();
        requirements.require_completion_model(&config.extract_graph.completion_model_id);
        requirements.require_completion_model(&config.summarize_descriptions.completion_model_id);
        requirements.require_tokenizer("chunking.encoding_model", &config.chunking.encoding_model);
        Ok(requirements)
    }

    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut IndexPipelineContext,
    ) -> Result<IndexWorkflowOutput> {
        let text_units = read_text_units(
            &context
                .output_table_provider()
                .read_dataframe("text_units")
                .await?,
        )?;
        let extractor = resolve_completion_model(
            context,
            &config.extract_graph.completion_model_id,
            EXTRACT_GRAPH_WORKFLOW,
        )?;
        let summarizer = resolve_completion_model(
            context,
            &config.summarize_descriptions.completion_model_id,
            EXTRACT_GRAPH_WORKFLOW,
        )?;
        let tokenizer = TiktokenTokenizer::new(&config.chunking.encoding_model)?;
        let prompt_repository = PromptRepository::new(context.prompt_root());
        let concurrency = config.concurrent_requests.max(1);
        let extraction_template = prompt_repository
            .load(
                PromptKind::ExtractGraph,
                config.extract_graph.prompt.as_deref().map(Path::new),
            )
            .await?;
        let continue_template = prompt_repository
            .load(PromptKind::ExtractGraphContinue, None)
            .await?;
        let loop_template = prompt_repository
            .load(PromptKind::ExtractGraphLoop, None)
            .await?;
        let graph = extract_graph(
            extractor.as_ref(),
            &extraction_template,
            &continue_template,
            &loop_template,
            &text_units,
            GraphExtractionConfig {
                entity_types: &config.extract_graph.entity_types,
                max_gleanings: config.extract_graph.max_gleanings,
                concurrency,
            },
            &|completed, total| {
                context
                    .callbacks
                    .progress(EXTRACT_GRAPH_WORKFLOW, completed, Some(total));
            },
        )
        .await?;
        let summary_template = prompt_repository
            .load(
                PromptKind::SummarizeDescriptions,
                config
                    .summarize_descriptions
                    .prompt
                    .as_deref()
                    .map(Path::new),
            )
            .await?;
        let summarized_graph = summarize_graph(
            summarizer.as_ref(),
            &summary_template,
            &tokenizer,
            &graph,
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
        write_graph_tables(config, context, &graph, &summarized_graph).await?;

        context.stats.entity_count = summarized_graph.entities.len();
        context.stats.relationship_count = summarized_graph.relationships.len();
        Ok(IndexWorkflowOutput {
            result: extract_graph_sample(
                &summarized_graph.entities,
                &summarized_graph.relationships,
            ),
            stop: false,
            input_rows: text_units.len(),
            output_rows: summarized_graph
                .entities
                .len()
                .saturating_add(summarized_graph.relationships.len()),
        })
    }
}

async fn write_graph_tables(
    config: &GraphRagConfig,
    context: &mut IndexPipelineContext,
    graph: &ExtractedGraph,
    summarized: &SummarizedGraph,
) -> Result<()> {
    context
        .output_table_provider()
        .write_dataframe(
            "entities",
            entity_intermediate_dataframe(&summarized.entities)?,
        )
        .await?;
    context
        .output_table_provider()
        .write_dataframe(
            "relationships",
            relationship_intermediate_dataframe(&summarized.relationships)?,
        )
        .await?;

    if config.snapshots.raw_graph {
        context
            .output_table_provider()
            .write_dataframe("raw_entities", raw_entity_dataframe(&graph.entities)?)
            .await?;
        context
            .output_table_provider()
            .write_dataframe(
                "raw_relationships",
                raw_relationship_dataframe(&graph.relationships)?,
            )
            .await?;
    }

    Ok(())
}
