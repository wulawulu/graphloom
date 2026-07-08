//! Entity and relationship description summarization operations.

use std::{collections::BTreeSet, path::Path};

use graphloom_llm::{
    ChatMessage, CompletionModel, CompletionRequest, DefaultPrompt, PromptLoader, Tokenizer,
};
use serde::Serialize;

use super::{EntityRow, RelationshipRow, SummarizedEntityRow, SummarizedRelationshipRow};
use crate::{GraphLoomError, Result};

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

pub(crate) async fn summarize_entities(
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

pub(crate) async fn summarize_relationships(
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
