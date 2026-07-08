//! Entity and relationship description summarization operations.

use std::{collections::BTreeSet, path::Path};

use futures_util::{StreamExt, stream};
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
pub(crate) struct DescriptionSummarizeConfig {
    pub(crate) max_length: usize,
    pub(crate) max_input_tokens: usize,
    pub(crate) concurrency: usize,
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
    config: DescriptionSummarizeConfig,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Vec<SummarizedEntityRow>> {
    let context = SummarizeContext {
        model,
        prompt_loader,
        prompt_path,
        tokenizer,
        max_length: config.max_length,
        max_input_tokens: config.max_input_tokens,
    };
    let mut stream = stream::iter(rows.iter().cloned().enumerate())
        .map(|(index, row)| {
            let summarize_context = context;
            async move {
                let id = serde_json::to_string(&row.title)?;
                let description =
                    summarize_description_list(&summarize_context, &id, &row.description).await?;
                Ok::<(usize, SummarizedEntityRow), crate::GraphLoomError>((
                    index,
                    SummarizedEntityRow {
                        title: row.title,
                        entity_type: row.entity_type,
                        description,
                        text_unit_ids: row.text_unit_ids,
                        frequency: row.frequency,
                    },
                ))
            }
        })
        .buffer_unordered(config.concurrency.max(1));

    let mut completed = 0usize;
    let mut summarized = Vec::with_capacity(rows.len());
    while let Some(result) = stream.next().await {
        let result = result?;
        completed = completed.saturating_add(1);
        progress(completed, rows.len());
        summarized.push(result);
    }
    summarized.sort_by_key(|(index, _)| *index);
    Ok(summarized.into_iter().map(|(_, row)| row).collect())
}

pub(crate) async fn summarize_relationships(
    model: &dyn CompletionModel,
    prompt_loader: &PromptLoader,
    prompt_path: Option<&str>,
    tokenizer: &dyn Tokenizer,
    rows: &[RelationshipRow],
    config: DescriptionSummarizeConfig,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Vec<SummarizedRelationshipRow>> {
    let context = SummarizeContext {
        model,
        prompt_loader,
        prompt_path,
        tokenizer,
        max_length: config.max_length,
        max_input_tokens: config.max_input_tokens,
    };
    let mut stream = stream::iter(rows.iter().cloned().enumerate())
        .map(|(index, row)| {
            let summarize_context = context;
            async move {
                let id = serde_json::to_string(&[row.source.as_str(), row.target.as_str()])?;
                let description =
                    summarize_description_list(&summarize_context, &id, &row.description).await?;
                Ok::<(usize, SummarizedRelationshipRow), crate::GraphLoomError>((
                    index,
                    SummarizedRelationshipRow {
                        source: row.source,
                        target: row.target,
                        description,
                        text_unit_ids: row.text_unit_ids,
                        weight: row.weight,
                    },
                ))
            }
        })
        .buffer_unordered(config.concurrency.max(1));

    let mut completed = 0usize;
    let mut summarized = Vec::with_capacity(rows.len());
    while let Some(result) = stream.next().await {
        let result = result?;
        completed = completed.saturating_add(1);
        progress(completed, rows.len());
        summarized.push(result);
    }
    summarized.sort_by_key(|(index, _)| *index);
    Ok(summarized.into_iter().map(|(_, row)| row).collect())
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

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use graphloom_llm::{CompletionResponse, LlmError};

    use super::*;

    #[derive(Debug)]
    struct UnusedModel;

    #[async_trait]
    impl CompletionModel for UnusedModel {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> graphloom_llm::Result<CompletionResponse> {
            Err(LlmError::InvalidResponse {
                model_instance: "unused".to_owned(),
                operation: "completion",
                message: "single-description rows should not call the model".to_owned(),
            })
        }
    }

    #[derive(Debug)]
    struct WhitespaceTokenizer;

    impl Tokenizer for WhitespaceTokenizer {
        fn encode(&self, text: &str) -> graphloom_llm::Result<Vec<u32>> {
            Ok(text
                .split_whitespace()
                .enumerate()
                .map(|(index, _)| u32::try_from(index).unwrap_or(u32::MAX))
                .collect())
        }

        fn decode(&self, _tokens: &[u32]) -> graphloom_llm::Result<String> {
            Ok(String::new())
        }
    }

    #[tokio::test]
    async fn test_should_preserve_entity_order_with_concurrent_summarization() {
        let rows = vec![
            EntityRow {
                title: "B".to_owned(),
                entity_type: "person".to_owned(),
                description: vec!["second".to_owned()],
                text_unit_ids: vec!["tu-2".to_owned()],
                frequency: 1,
            },
            EntityRow {
                title: "A".to_owned(),
                entity_type: "person".to_owned(),
                description: vec!["first".to_owned()],
                text_unit_ids: vec!["tu-1".to_owned()],
                frequency: 1,
            },
        ];
        let progress = Arc::new(AtomicUsize::new(0));
        let progress_ref = Arc::clone(&progress);

        let summarized = summarize_entities(
            &UnusedModel,
            &PromptLoader::new("."),
            None,
            &WhitespaceTokenizer,
            &rows,
            DescriptionSummarizeConfig {
                max_length: 100,
                max_input_tokens: 100,
                concurrency: 2,
            },
            &|completed, _| {
                progress_ref.store(completed, Ordering::SeqCst);
            },
        )
        .await
        .expect("summarization should succeed");

        assert_eq!(summarized[0].title, "B");
        assert_eq!(summarized[1].title, "A");
        assert_eq!(progress.load(Ordering::SeqCst), 2);
    }
}
