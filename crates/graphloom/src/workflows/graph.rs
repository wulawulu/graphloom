//! Graph extraction and finalization workflows.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};

use async_trait::async_trait;
use graphloom_llm::{
    ChatMessage, CompletionModel, CompletionRequest, DefaultPrompt, OpenAiCompletionModel,
    PromptLoader, TiktokenTokenizer, Tokenizer, parse_graph_tuples,
};
use polars_core::{frame::row::Row, prelude::*};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    base_text_units::{optional_string_at, string_at},
    input_documents::list_column,
};
use crate::{
    GraphLoomError, GraphRagConfig, PipelineRunContext, Result, Workflow, WorkflowFunctionOutput,
};

/// Workflow name.
pub const EXTRACT_GRAPH_WORKFLOW: &str = "extract_graph";
/// Workflow name.
pub const FINALIZE_GRAPH_WORKFLOW: &str = "finalize_graph";
const CONTINUE_PROMPT: &str = "MANY entities and relationships were missed in the last \
                               extraction. Remember to ONLY emit entities that match any of the \
                               previously extracted types. Add them below using the same format:\n";
const LOOP_PROMPT: &str = "It appears some entities and relationships may have still been missed. \
                           Answer Y if there are still entities or relationships that need to be \
                           added, or N if there are none. Please answer with a single letter Y or \
                           N.\n";

/// Extract entity and relationship graph rows from text units.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExtractGraphWorkflow;

/// Finalize extracted graph rows.
#[derive(Debug, Clone, Copy, Default)]
pub struct FinalizeGraphWorkflow;

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
        )?;
        let summarizer = resolve_completion_model(
            config,
            context,
            &config.summarize_descriptions.completion_model_id,
            &config.summarize_descriptions.model_instance_name,
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

#[async_trait]
impl Workflow for FinalizeGraphWorkflow {
    fn name(&self) -> &'static str {
        FINALIZE_GRAPH_WORKFLOW
    }

    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut PipelineRunContext,
    ) -> Result<WorkflowFunctionOutput> {
        let entities = read_entity_rows(
            &context
                .output_table_provider
                .read_dataframe("entities")
                .await?,
        )?;
        let relationships = read_relationship_rows(
            &context
                .output_table_provider
                .read_dataframe("relationships")
                .await?,
        )?;
        let degree_map = degree_map(&relationships);
        let final_entities = finalize_entities(&entities, &degree_map)?;
        let final_relationships = finalize_relationships(&relationships, &degree_map)?;

        context
            .output_table_provider
            .write_dataframe("entities", final_entities_dataframe(&final_entities)?)
            .await?;
        context
            .output_table_provider
            .write_dataframe(
                "relationships",
                final_relationships_dataframe(&final_relationships)?,
            )
            .await?;

        if config.snapshots.graphml {
            let storage =
                context
                    .output_storage
                    .as_ref()
                    .ok_or(GraphLoomError::MissingProvider {
                        name: "output_storage",
                    })?;
            storage
                .set_text("graph.graphml", &graphml_snapshot(&final_relationships))
                .await?;
        }

        context.stats.entity_count = final_entities.len();
        context.stats.relationship_count = final_relationships.len();
        Ok(WorkflowFunctionOutput {
            result: finalize_graph_sample(&final_entities, &final_relationships),
            stop: false,
            input_rows: entities.len().saturating_add(relationships.len()),
            output_rows: final_entities
                .len()
                .saturating_add(final_relationships.len()),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TextUnitInput {
    id: String,
    text: String,
}

#[derive(Debug, Clone, PartialEq)]
struct RawEntityRow {
    title: String,
    entity_type: String,
    description: String,
    source_id: String,
}

#[derive(Debug, Clone, PartialEq)]
struct RawRelationshipRow {
    source: String,
    target: String,
    description: String,
    source_id: String,
    weight: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct EntityRow {
    title: String,
    entity_type: String,
    description: Vec<String>,
    text_unit_ids: Vec<String>,
    frequency: i64,
}

#[derive(Debug, Clone, PartialEq)]
struct RelationshipRow {
    source: String,
    target: String,
    description: Vec<String>,
    text_unit_ids: Vec<String>,
    weight: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct SummarizedEntityRow {
    title: String,
    entity_type: String,
    description: String,
    text_unit_ids: Vec<String>,
    frequency: i64,
}

#[derive(Debug, Clone, PartialEq)]
struct SummarizedRelationshipRow {
    source: String,
    target: String,
    description: String,
    text_unit_ids: Vec<String>,
    weight: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct FinalEntityRow {
    id: String,
    human_readable_id: usize,
    title: String,
    entity_type: String,
    description: String,
    text_unit_ids: Vec<String>,
    frequency: i64,
    degree: i64,
}

#[derive(Debug, Clone, PartialEq)]
struct FinalRelationshipRow {
    id: String,
    human_readable_id: usize,
    source: String,
    target: String,
    description: String,
    weight: f64,
    combined_degree: i64,
    text_unit_ids: Vec<String>,
}

fn resolve_completion_model(
    config: &GraphRagConfig,
    context: &PipelineRunContext,
    model_id: &str,
    model_instance_name: &str,
) -> Result<Arc<dyn CompletionModel>> {
    if let Some(model) = context.completion_models.get(model_id) {
        return Ok(Arc::clone(model));
    }
    let model_config =
        config
            .completion_models
            .get(model_id)
            .ok_or_else(|| GraphLoomError::InvalidData {
                workflow: EXTRACT_GRAPH_WORKFLOW,
                message: format!("completion model {model_id} is not configured"),
            })?;
    Ok(Arc::new(OpenAiCompletionModel::new(
        model_instance_name,
        model_config.clone(),
        config.concurrent_requests,
    )?))
}

async fn extract_text_unit_graph(
    model: &dyn CompletionModel,
    prompt_loader: &PromptLoader,
    prompt_path: Option<&str>,
    entity_types: &[String],
    text_unit: &TextUnitInput,
    max_gleanings: usize,
) -> Result<(Vec<RawEntityRow>, Vec<RawRelationshipRow>)> {
    let mut messages = vec![ChatMessage::user(
        render_extraction_prompt(prompt_loader, prompt_path, &text_unit.text, entity_types).await?,
    )];

    let mut output = model
        .complete(CompletionRequest {
            messages: messages.clone(),
            temperature: None,
            top_p: None,
            max_tokens: None,
            response_format: None,
            cache_namespace: None,
        })
        .await?
        .content;
    messages.push(ChatMessage::assistant(output.clone()));

    for glean_index in 0..max_gleanings {
        messages.push(ChatMessage::user(CONTINUE_PROMPT.to_owned()));
        let response = model
            .complete(CompletionRequest {
                messages: messages.clone(),
                temperature: None,
                top_p: None,
                max_tokens: None,
                response_format: None,
                cache_namespace: None,
            })
            .await?
            .content;
        output.push_str(&response);
        messages.push(ChatMessage::assistant(response));

        if glean_index >= max_gleanings.saturating_sub(1) {
            break;
        }

        messages.push(ChatMessage::user(LOOP_PROMPT.to_owned()));
        let response = model
            .complete(CompletionRequest {
                messages: messages.clone(),
                temperature: None,
                top_p: None,
                max_tokens: None,
                response_format: None,
                cache_namespace: None,
            })
            .await?
            .content;
        if response != "Y" {
            break;
        }
    }

    let parsed = parse_graph_tuples(&output, &text_unit.id);
    let entities = parsed
        .entities
        .into_iter()
        .map(|entity| RawEntityRow {
            title: entity.title,
            entity_type: entity.entity_type,
            description: entity.description,
            source_id: entity.source_id,
        })
        .collect::<Vec<_>>();
    let relationships = parsed
        .relationships
        .into_iter()
        .map(|relationship| RawRelationshipRow {
            source: relationship.source,
            target: relationship.target,
            description: relationship.description,
            source_id: relationship.source_id,
            weight: relationship.weight,
        })
        .collect::<Vec<_>>();
    Ok((entities, relationships))
}

#[derive(Debug, Serialize)]
struct ExtractPromptValues<'a> {
    entity_types: String,
    input_text: &'a str,
}

async fn render_extraction_prompt(
    prompt_loader: &PromptLoader,
    prompt_path: Option<&str>,
    input_text: &str,
    entity_types: &[String],
) -> Result<String> {
    prompt_loader
        .render(
            DefaultPrompt::ExtractGraph,
            prompt_path.map(Path::new),
            &ExtractPromptValues {
                entity_types: entity_types.join(","),
                input_text,
            },
        )
        .await
        .map_err(GraphLoomError::from)
}

#[derive(Debug, Serialize)]
struct SummarizePromptValues {
    entity_name: String,
    description_list: String,
    max_length: usize,
}

#[derive(Debug, Clone, Copy)]
struct SummarizeContext<'a> {
    model: &'a dyn CompletionModel,
    prompt_loader: &'a PromptLoader,
    prompt_path: Option<&'a str>,
    tokenizer: &'a dyn Tokenizer,
    max_length: usize,
    max_input_tokens: usize,
}

async fn render_summarization_prompt(
    prompt_loader: &PromptLoader,
    prompt_path: Option<&str>,
    entity_name_json: &str,
    descriptions: &[String],
    max_length: usize,
) -> Result<String> {
    prompt_loader
        .render(
            DefaultPrompt::SummarizeDescriptions,
            prompt_path.map(Path::new),
            &SummarizePromptValues {
                entity_name: entity_name_json.to_owned(),
                description_list: serde_json::to_string(descriptions)?,
                max_length,
            },
        )
        .await
        .map_err(GraphLoomError::from)
}

async fn summarize_entities(
    model: &dyn CompletionModel,
    prompt_loader: &PromptLoader,
    prompt_path: Option<&str>,
    tokenizer: &dyn Tokenizer,
    rows: &[EntityRow],
    max_length: usize,
    max_input_tokens: usize,
) -> Result<Vec<SummarizedEntityRow>> {
    let context = SummarizeContext {
        model,
        prompt_loader,
        prompt_path,
        tokenizer,
        max_length,
        max_input_tokens,
    };
    let mut summarized = Vec::with_capacity(rows.len());
    for row in rows {
        let id = serde_json::to_string(&row.title)?;
        summarized.push(SummarizedEntityRow {
            title: row.title.clone(),
            entity_type: row.entity_type.clone(),
            description: summarize_description_list(&context, &id, &row.description).await?,
            text_unit_ids: row.text_unit_ids.clone(),
            frequency: row.frequency,
        });
    }
    Ok(summarized)
}

async fn summarize_relationships(
    model: &dyn CompletionModel,
    prompt_loader: &PromptLoader,
    prompt_path: Option<&str>,
    tokenizer: &dyn Tokenizer,
    rows: &[RelationshipRow],
    max_length: usize,
    max_input_tokens: usize,
) -> Result<Vec<SummarizedRelationshipRow>> {
    let context = SummarizeContext {
        model,
        prompt_loader,
        prompt_path,
        tokenizer,
        max_length,
        max_input_tokens,
    };
    let mut summarized = Vec::with_capacity(rows.len());
    for row in rows {
        let id = serde_json::to_string(&[row.source.as_str(), row.target.as_str()])?;
        summarized.push(SummarizedRelationshipRow {
            source: row.source.clone(),
            target: row.target.clone(),
            description: summarize_description_list(&context, &id, &row.description).await?,
            text_unit_ids: row.text_unit_ids.clone(),
            weight: row.weight,
        });
    }
    Ok(summarized)
}

async fn summarize_description_list(
    context: &SummarizeContext<'_>,
    entity_name_json: &str,
    descriptions: &[String],
) -> Result<String> {
    let descriptions = descriptions
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if descriptions.is_empty() {
        return Ok(String::new());
    }
    if descriptions.len() == 1 {
        return match descriptions.first() {
            Some(description) => Ok(description.clone()),
            None => Ok(String::new()),
        };
    }

    let prompt_budget = render_summarization_prompt(
        context.prompt_loader,
        context.prompt_path,
        entity_name_json,
        &[],
        context.max_length,
    )
    .await?;
    let mut usable_tokens = context
        .max_input_tokens
        .saturating_sub(context.tokenizer.count(&prompt_budget)?);
    let mut collected = Vec::new();
    let mut result = String::new();
    for (index, description) in descriptions.iter().enumerate() {
        usable_tokens = usable_tokens.saturating_sub(context.tokenizer.count(description)?);
        collected.push(description.clone());
        if (usable_tokens == 0 && collected.len() > 1)
            || index == descriptions.len().saturating_sub(1)
        {
            let prompt = render_summarization_prompt(
                context.prompt_loader,
                context.prompt_path,
                entity_name_json,
                &collected,
                context.max_length,
            )
            .await?;
            result = context
                .model
                .complete(CompletionRequest {
                    messages: vec![ChatMessage::user(prompt)],
                    temperature: None,
                    top_p: None,
                    max_tokens: None,
                    response_format: None,
                    cache_namespace: None,
                })
                .await?
                .content;
            if index != descriptions.len().saturating_sub(1) {
                collected = vec![result.clone()];
                usable_tokens = context
                    .max_input_tokens
                    .saturating_sub(context.tokenizer.count(&prompt_budget)?)
                    .saturating_sub(context.tokenizer.count(&result)?);
            }
        }
    }
    Ok(result)
}

fn merge_entities(rows: &[RawEntityRow]) -> Vec<EntityRow> {
    let mut grouped: BTreeMap<(String, String), EntityRow> = BTreeMap::new();
    for row in rows {
        let key = (row.title.clone(), row.entity_type.clone());
        let entry = grouped.entry(key).or_insert_with(|| EntityRow {
            title: row.title.clone(),
            entity_type: row.entity_type.clone(),
            description: Vec::new(),
            text_unit_ids: Vec::new(),
            frequency: 0,
        });
        entry.description.push(row.description.clone());
        entry.text_unit_ids.push(row.source_id.clone());
        entry.frequency = entry.frequency.saturating_add(1);
    }
    grouped.into_values().collect()
}

fn merge_relationships(rows: &[RawRelationshipRow]) -> Vec<RelationshipRow> {
    let mut grouped: BTreeMap<(String, String), RelationshipRow> = BTreeMap::new();
    for row in rows {
        let key = (row.source.clone(), row.target.clone());
        let entry = grouped.entry(key).or_insert_with(|| RelationshipRow {
            source: row.source.clone(),
            target: row.target.clone(),
            description: Vec::new(),
            text_unit_ids: Vec::new(),
            weight: 0.0,
        });
        entry.description.push(row.description.clone());
        entry.text_unit_ids.push(row.source_id.clone());
        entry.weight += row.weight;
    }
    grouped.into_values().collect()
}

fn filter_orphan_relationships(
    relationships: Vec<RelationshipRow>,
    entities: &[EntityRow],
) -> Vec<RelationshipRow> {
    let titles = entities
        .iter()
        .map(|entity| entity.title.as_str())
        .collect::<BTreeSet<_>>();
    relationships
        .into_iter()
        .filter(|relationship| {
            titles.contains(relationship.source.as_str())
                && titles.contains(relationship.target.as_str())
        })
        .collect()
}

fn degree_map(rows: &[SummarizedRelationshipRow]) -> BTreeMap<String, i64> {
    let mut seen = BTreeSet::new();
    let mut degree = BTreeMap::new();
    for row in rows {
        let (left, right) = sorted_pair(&row.source, &row.target);
        if seen.insert((left.clone(), right.clone())) {
            *degree.entry(left).or_insert(0) += 1;
            *degree.entry(right).or_insert(0) += 1;
        }
    }
    degree
}

fn finalize_entities(
    rows: &[SummarizedEntityRow],
    degree_map: &BTreeMap<String, i64>,
) -> Result<Vec<FinalEntityRow>> {
    let mut seen = BTreeSet::new();
    let mut final_rows = Vec::new();
    for row in rows {
        if !seen.insert(row.title.clone()) {
            continue;
        }
        final_rows.push(FinalEntityRow {
            id: Uuid::new_v4().to_string(),
            human_readable_id: final_rows.len(),
            title: row.title.clone(),
            entity_type: row.entity_type.clone(),
            description: row.description.clone(),
            text_unit_ids: row.text_unit_ids.clone(),
            frequency: row.frequency,
            degree: degree_map
                .get(&row.title)
                .copied()
                .map_or(0, |degree| degree),
        });
    }
    Ok(final_rows)
}

fn finalize_relationships(
    rows: &[SummarizedRelationshipRow],
    degree_map: &BTreeMap<String, i64>,
) -> Result<Vec<FinalRelationshipRow>> {
    let mut seen = BTreeSet::new();
    let mut final_rows = Vec::new();
    for row in rows {
        let key = (row.source.clone(), row.target.clone());
        if !seen.insert(key.clone()) {
            continue;
        }
        final_rows.push(FinalRelationshipRow {
            id: Uuid::new_v4().to_string(),
            human_readable_id: final_rows.len(),
            source: row.source.clone(),
            target: row.target.clone(),
            description: row.description.clone(),
            weight: row.weight,
            combined_degree: degree_map
                .get(&row.source)
                .copied()
                .map_or(0, |degree| degree)
                .saturating_add(
                    degree_map
                        .get(&row.target)
                        .copied()
                        .map_or(0, |degree| degree),
                ),
            text_unit_ids: row.text_unit_ids.clone(),
        });
    }
    Ok(final_rows)
}

fn sorted_pair(left: &str, right: &str) -> (String, String) {
    if left <= right {
        (left.to_owned(), right.to_owned())
    } else {
        (right.to_owned(), left.to_owned())
    }
}

fn read_text_units(dataframe: &DataFrame) -> Result<Vec<TextUnitInput>> {
    let ids = dataframe.column("id")?.str()?;
    let texts = dataframe.column("text")?.str()?;
    let mut rows = Vec::with_capacity(dataframe.height());
    for index in 0..dataframe.height() {
        rows.push(TextUnitInput {
            id: ids
                .get(index)
                .ok_or_else(|| invalid_data("missing text unit id"))?
                .to_owned(),
            text: texts
                .get(index)
                .ok_or_else(|| invalid_data("missing text unit text"))?
                .to_owned(),
        });
    }
    Ok(rows)
}

fn read_entity_rows(dataframe: &DataFrame) -> Result<Vec<SummarizedEntityRow>> {
    let mut rows = Vec::with_capacity(dataframe.height());
    for row_index in 0..dataframe.height() {
        let row = row_to_static(dataframe.get_row(row_index)?);
        rows.push(SummarizedEntityRow {
            title: string_at(&row, 0, "title")?,
            entity_type: string_at(&row, 1, "type")?,
            description: string_list_or_string_at(&row, 2).join("\n"),
            text_unit_ids: list_at(&row, 3)?,
            frequency: i64_at(&row, 4, "frequency")?,
        });
    }
    Ok(rows)
}

fn read_relationship_rows(dataframe: &DataFrame) -> Result<Vec<SummarizedRelationshipRow>> {
    let mut rows = Vec::with_capacity(dataframe.height());
    for row_index in 0..dataframe.height() {
        let row = row_to_static(dataframe.get_row(row_index)?);
        rows.push(SummarizedRelationshipRow {
            source: string_at(&row, 0, "source")?,
            target: string_at(&row, 1, "target")?,
            description: string_list_or_string_at(&row, 2).join("\n"),
            text_unit_ids: list_at(&row, 3)?,
            weight: f64_at(&row, 4, "weight")?,
        });
    }
    Ok(rows)
}

fn raw_entity_dataframe(rows: &[RawEntityRow]) -> Result<DataFrame> {
    Ok(df!(
        "title" => rows.iter().map(|row| row.title.as_str()).collect::<Vec<_>>(),
        "type" => rows.iter().map(|row| row.entity_type.as_str()).collect::<Vec<_>>(),
        "description" => rows.iter().map(|row| row.description.as_str()).collect::<Vec<_>>(),
        "source_id" => rows.iter().map(|row| row.source_id.as_str()).collect::<Vec<_>>(),
    )?)
}

fn raw_relationship_dataframe(rows: &[RawRelationshipRow]) -> Result<DataFrame> {
    Ok(df!(
        "source" => rows.iter().map(|row| row.source.as_str()).collect::<Vec<_>>(),
        "target" => rows.iter().map(|row| row.target.as_str()).collect::<Vec<_>>(),
        "weight" => rows.iter().map(|row| row.weight).collect::<Vec<_>>(),
        "description" => rows.iter().map(|row| row.description.as_str()).collect::<Vec<_>>(),
        "source_id" => rows.iter().map(|row| row.source_id.as_str()).collect::<Vec<_>>(),
    )?)
}

fn entity_intermediate_dataframe(rows: &[SummarizedEntityRow]) -> Result<DataFrame> {
    let mut dataframe = df!(
        "title" => rows.iter().map(|row| row.title.as_str()).collect::<Vec<_>>(),
        "type" => rows.iter().map(|row| row.entity_type.as_str()).collect::<Vec<_>>(),
        "description" => rows.iter().map(|row| row.description.as_str()).collect::<Vec<_>>(),
        "frequency" => rows.iter().map(|row| row.frequency).collect::<Vec<_>>(),
    )?;
    dataframe.insert_column(
        3,
        list_column(
            "text_unit_ids",
            &rows
                .iter()
                .map(|row| row.text_unit_ids.clone())
                .collect::<Vec<_>>(),
        )?,
    )?;
    Ok(dataframe)
}

fn relationship_intermediate_dataframe(rows: &[SummarizedRelationshipRow]) -> Result<DataFrame> {
    let mut dataframe = df!(
        "source" => rows.iter().map(|row| row.source.as_str()).collect::<Vec<_>>(),
        "target" => rows.iter().map(|row| row.target.as_str()).collect::<Vec<_>>(),
        "description" => rows.iter().map(|row| row.description.as_str()).collect::<Vec<_>>(),
        "weight" => rows.iter().map(|row| row.weight).collect::<Vec<_>>(),
    )?;
    dataframe.insert_column(
        3,
        list_column(
            "text_unit_ids",
            &rows
                .iter()
                .map(|row| row.text_unit_ids.clone())
                .collect::<Vec<_>>(),
        )?,
    )?;
    Ok(dataframe)
}

fn final_entities_dataframe(rows: &[FinalEntityRow]) -> Result<DataFrame> {
    let mut dataframe = df!(
        "id" => rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        "human_readable_id" => rows.iter().map(|row| row.human_readable_id as u64).collect::<Vec<_>>(),
        "title" => rows.iter().map(|row| row.title.as_str()).collect::<Vec<_>>(),
        "type" => rows.iter().map(|row| row.entity_type.as_str()).collect::<Vec<_>>(),
        "description" => rows.iter().map(|row| row.description.as_str()).collect::<Vec<_>>(),
        "frequency" => rows.iter().map(|row| row.frequency).collect::<Vec<_>>(),
        "degree" => rows.iter().map(|row| row.degree).collect::<Vec<_>>(),
    )?;
    dataframe.insert_column(
        5,
        list_column(
            "text_unit_ids",
            &rows
                .iter()
                .map(|row| row.text_unit_ids.clone())
                .collect::<Vec<_>>(),
        )?,
    )?;
    Ok(dataframe)
}

fn final_relationships_dataframe(rows: &[FinalRelationshipRow]) -> Result<DataFrame> {
    let mut dataframe = df!(
        "id" => rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        "human_readable_id" => rows.iter().map(|row| row.human_readable_id as u64).collect::<Vec<_>>(),
        "source" => rows.iter().map(|row| row.source.as_str()).collect::<Vec<_>>(),
        "target" => rows.iter().map(|row| row.target.as_str()).collect::<Vec<_>>(),
        "description" => rows.iter().map(|row| row.description.as_str()).collect::<Vec<_>>(),
        "weight" => rows.iter().map(|row| row.weight).collect::<Vec<_>>(),
        "combined_degree" => rows.iter().map(|row| row.combined_degree).collect::<Vec<_>>(),
    )?;
    dataframe.with_column(list_column(
        "text_unit_ids",
        &rows
            .iter()
            .map(|row| row.text_unit_ids.clone())
            .collect::<Vec<_>>(),
    )?)?;
    Ok(dataframe)
}

fn extract_graph_sample(
    entities: &[SummarizedEntityRow],
    relationships: &[SummarizedRelationshipRow],
) -> Vec<Value> {
    vec![
        json!({"entities": entities.iter().take(5).map(entity_value).collect::<Vec<_>>()}),
        json!({"relationships": relationships.iter().take(5).map(relationship_value).collect::<Vec<_>>()}),
    ]
}

fn finalize_graph_sample(
    entities: &[FinalEntityRow],
    relationships: &[FinalRelationshipRow],
) -> Vec<Value> {
    vec![
        json!({"entities": entities.iter().take(5).map(final_entity_value).collect::<Vec<_>>()}),
        json!({"relationships": relationships.iter().take(5).map(final_relationship_value).collect::<Vec<_>>()}),
    ]
}

fn entity_value(row: &SummarizedEntityRow) -> Value {
    json!({
        "title": row.title,
        "type": row.entity_type,
        "description": row.description,
        "text_unit_ids": row.text_unit_ids,
        "frequency": row.frequency,
    })
}

fn relationship_value(row: &SummarizedRelationshipRow) -> Value {
    json!({
        "source": row.source,
        "target": row.target,
        "description": row.description,
        "text_unit_ids": row.text_unit_ids,
        "weight": row.weight,
    })
}

fn final_entity_value(row: &FinalEntityRow) -> Value {
    json!({
        "id": row.id,
        "human_readable_id": row.human_readable_id,
        "title": row.title,
        "type": row.entity_type,
        "description": row.description,
        "text_unit_ids": row.text_unit_ids,
        "frequency": row.frequency,
        "degree": row.degree,
    })
}

fn final_relationship_value(row: &FinalRelationshipRow) -> Value {
    json!({
        "id": row.id,
        "human_readable_id": row.human_readable_id,
        "source": row.source,
        "target": row.target,
        "description": row.description,
        "weight": row.weight,
        "combined_degree": row.combined_degree,
        "text_unit_ids": row.text_unit_ids,
    })
}

fn graphml_snapshot(rows: &[FinalRelationshipRow]) -> String {
    let mut graphml = String::from(
        r#"<?xml version="1.0" encoding="utf-8"?>
<graphml xmlns="http://graphml.graphdrawing.org/xmlns">
<key id="weight" for="edge" attr.name="weight" attr.type="double"/>
<graph edgedefault="undirected">
"#,
    );
    let mut nodes = BTreeSet::new();
    for row in rows {
        nodes.insert(row.source.as_str());
        nodes.insert(row.target.as_str());
    }
    for node in nodes {
        graphml.push_str(&format!(r#"<node id="{}"/>"#, xml_escape(node)));
        graphml.push('\n');
    }
    for row in rows {
        graphml.push_str(&format!(
            r#"<edge source="{}" target="{}"><data key="weight">{}</data></edge>"#,
            xml_escape(&row.source),
            xml_escape(&row.target),
            row.weight,
        ));
        graphml.push('\n');
    }
    graphml.push_str("</graph>\n</graphml>\n");
    graphml
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(crate) fn row_to_static(row: Row<'_>) -> Row<'static> {
    Row::new(row.0.into_iter().map(AnyValue::into_static).collect())
}

pub(crate) fn list_at(row: &Row<'static>, index: usize) -> Result<Vec<String>> {
    let Some(value) = row.0.get(index) else {
        return Ok(Vec::new());
    };
    match value {
        AnyValue::List(series) => {
            let strings = series.str()?;
            Ok((0..series.len())
                .filter_map(|index| strings.get(index).map(str::to_owned))
                .collect())
        }
        AnyValue::Null => Ok(Vec::new()),
        AnyValue::String(value) => Ok(vec![(*value).to_owned()]),
        AnyValue::StringOwned(value) => Ok(vec![value.to_string()]),
        _ => Err(invalid_data("expected string list column")),
    }
}

fn string_list_or_string_at(row: &Row<'static>, index: usize) -> Vec<String> {
    let values = list_at(row, index)
        .ok()
        .filter(|values| !values.is_empty())
        .or_else(|| optional_string_at(row, index).map(|value| vec![value]));
    let Some(values) = values else {
        return Vec::new();
    };
    values
}

fn i64_at(row: &Row<'static>, index: usize, column: &'static str) -> Result<i64> {
    row.0
        .get(index)
        .and_then(|value| match value {
            AnyValue::Int64(value) => Some(*value),
            AnyValue::Int32(value) => Some(i64::from(*value)),
            AnyValue::UInt32(value) => Some(i64::from(*value)),
            _ => None,
        })
        .ok_or_else(|| invalid_data(&format!("missing integer column {column}")))
}

fn f64_at(row: &Row<'static>, index: usize, column: &'static str) -> Result<f64> {
    row.0
        .get(index)
        .and_then(|value| match value {
            AnyValue::Float64(value) => Some(*value),
            AnyValue::Float32(value) => Some(f64::from(*value)),
            _ => None,
        })
        .ok_or_else(|| invalid_data(&format!("missing float column {column}")))
}

fn invalid_data(message: &str) -> GraphLoomError {
    GraphLoomError::InvalidData {
        workflow: EXTRACT_GRAPH_WORKFLOW,
        message: message.to_owned(),
    }
}
