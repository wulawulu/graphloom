//! Graph extraction workflow orchestration.

use std::path::Path;

use async_trait::async_trait;
use futures_util::{StreamExt, stream};
use graphloom_llm::{CompletionModel, TiktokenTokenizer};

use super::common::resolve_completion_model;
use crate::{
    GraphLoomError, GraphRagConfig, PipelineRunContext, Result, Workflow, WorkflowFunctionOutput,
    operations::graph::{
        DescriptionSummarizeConfig, EntityRow, RelationshipRow, SummarizedEntityRow,
        SummarizedRelationshipRow, TextUnitInput, entity_intermediate_dataframe,
        extract_graph_sample, extract_text_unit_graph, filter_orphan_relationships, merge_entities,
        merge_relationships, raw_entity_dataframe, raw_relationship_dataframe, read_text_units,
        relationship_intermediate_dataframe, summarize_entities, summarize_relationships,
    },
    prompts::{PromptKind, PromptRepository},
};

/// Workflow name.
pub const EXTRACT_GRAPH_WORKFLOW: &str = "extract_graph";

/// Extract entity and relationship graph rows from text units.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExtractGraphWorkflow;

#[derive(Debug)]
struct MergedGraph {
    entities: Vec<EntityRow>,
    relationships: Vec<RelationshipRow>,
}

#[derive(Debug)]
struct SummarizedGraph {
    entities: Vec<SummarizedEntityRow>,
    relationships: Vec<SummarizedRelationshipRow>,
}

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
        let prompt_repository = PromptRepository::new(context.prompt_root());
        let concurrency = config.concurrent_requests.max(1);
        let graph = extract_rows(
            config,
            context,
            &text_units,
            extractor.as_ref(),
            &prompt_repository,
            concurrency,
        )
        .await?;
        let summarized_graph = summarize_rows(
            config,
            context,
            summarizer.as_ref(),
            &prompt_repository,
            &tokenizer,
            &graph,
            concurrency,
        )
        .await?;
        write_graph_tables(config, context, &graph, &summarized_graph).await?;

        context.stats.entity_count = summarized_graph.entities.len();
        context.stats.relationship_count = summarized_graph.relationships.len();
        Ok(WorkflowFunctionOutput {
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

async fn extract_rows(
    config: &GraphRagConfig,
    context: &PipelineRunContext,
    text_units: &[TextUnitInput],
    extractor: &dyn CompletionModel,
    prompt_repository: &PromptRepository,
    concurrency: usize,
) -> Result<MergedGraph> {
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
    let entity_types = config.extract_graph.entity_types.clone();
    let max_gleanings = config.extract_graph.max_gleanings;
    let mut extraction_results = stream::iter(text_units.iter().cloned().enumerate())
        .map(|(index, text_unit)| {
            let extraction_template = extraction_template.clone();
            let continue_template = continue_template.clone();
            let loop_template = loop_template.clone();
            let entity_types = entity_types.clone();
            async move {
                extract_text_unit_graph(
                    extractor,
                    &extraction_template,
                    &continue_template,
                    &loop_template,
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
            message: "Graph Extraction failed. No entities detected during extraction.".to_owned(),
        });
    }
    if relationships.is_empty() {
        return Err(GraphLoomError::InvalidData {
            workflow: EXTRACT_GRAPH_WORKFLOW,
            message: "Graph Extraction failed. No relationships detected during extraction."
                .to_owned(),
        });
    }

    Ok(MergedGraph {
        entities,
        relationships,
    })
}

async fn summarize_rows(
    config: &GraphRagConfig,
    context: &PipelineRunContext,
    summarizer: &dyn CompletionModel,
    prompt_repository: &PromptRepository,
    tokenizer: &TiktokenTokenizer,
    graph: &MergedGraph,
    concurrency: usize,
) -> Result<SummarizedGraph> {
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
    let summary_total = graph
        .entities
        .len()
        .saturating_add(graph.relationships.len());
    let summarized_entities = summarize_entities(
        summarizer,
        &summary_template,
        tokenizer,
        &graph.entities,
        DescriptionSummarizeConfig {
            max_length: config.summarize_descriptions.max_length,
            max_input_tokens: config.summarize_descriptions.max_input_tokens,
            concurrency,
        },
        &|completed, _total| {
            context
                .callbacks
                .progress(EXTRACT_GRAPH_WORKFLOW, completed, Some(summary_total));
        },
    )
    .await?;
    let summarized_entity_count = summarized_entities.len();
    let summarized_relationships = summarize_relationships(
        summarizer,
        &summary_template,
        tokenizer,
        &graph.relationships,
        DescriptionSummarizeConfig {
            max_length: config.summarize_descriptions.max_length,
            max_input_tokens: config.summarize_descriptions.max_input_tokens,
            concurrency,
        },
        &|completed, _total| {
            let cumulative_completed = summarized_entity_count.saturating_add(completed);
            context.callbacks.progress(
                EXTRACT_GRAPH_WORKFLOW,
                cumulative_completed,
                Some(summary_total),
            );
        },
    )
    .await?;

    Ok(SummarizedGraph {
        entities: summarized_entities,
        relationships: summarized_relationships,
    })
}

async fn write_graph_tables(
    config: &GraphRagConfig,
    context: &mut PipelineRunContext,
    graph: &MergedGraph,
    summarized: &SummarizedGraph,
) -> Result<()> {
    context
        .output_table_provider
        .write_dataframe(
            "entities",
            entity_intermediate_dataframe(&summarized.entities)?,
        )
        .await?;
    context
        .output_table_provider
        .write_dataframe(
            "relationships",
            relationship_intermediate_dataframe(&summarized.relationships)?,
        )
        .await?;

    if config.snapshots.raw_graph {
        context
            .output_table_provider
            .write_dataframe("raw_entities", raw_entity_dataframe(&graph.entities)?)
            .await?;
        context
            .output_table_provider
            .write_dataframe(
                "raw_relationships",
                raw_relationship_dataframe(&graph.relationships)?,
            )
            .await?;
    }

    Ok(())
}
