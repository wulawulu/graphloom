//! Entity and relationship description summarization operations.

use std::collections::BTreeSet;

use futures_util::{StreamExt, stream};
use graphloom_llm::{ChatMessage, CompletionModel, CompletionRequest, Tokenizer};
use serde::Serialize;

use super::{
    EntityRow, ExtractedGraph, RelationshipRow, SummarizedEntityRow, SummarizedGraph,
    SummarizedRelationshipRow,
};
use crate::{Result, prompts::PromptTemplate};

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
    prompt_template: &'a PromptTemplate,
    tokenizer: &'a dyn Tokenizer,
    max_length: usize,
    max_input_tokens: usize,
}

pub(crate) async fn summarize_graph(
    model: &dyn CompletionModel,
    prompt_template: &PromptTemplate,
    tokenizer: &dyn Tokenizer,
    graph: &ExtractedGraph,
    config: DescriptionSummarizeConfig,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<SummarizedGraph> {
    let total = graph
        .entities
        .len()
        .saturating_add(graph.relationships.len());
    let entities = summarize_entities(
        model,
        prompt_template,
        tokenizer,
        &graph.entities,
        config,
        &|completed, _| progress(completed, total),
    )
    .await?;
    let entity_count = entities.len();
    let relationships = summarize_relationships(
        model,
        prompt_template,
        tokenizer,
        &graph.relationships,
        config,
        &|completed, _| progress(entity_count.saturating_add(completed), total),
    )
    .await?;
    Ok(SummarizedGraph {
        entities,
        relationships,
    })
}

pub(crate) async fn summarize_entities(
    model: &dyn CompletionModel,
    prompt_template: &PromptTemplate,
    tokenizer: &dyn Tokenizer,
    rows: &[EntityRow],
    config: DescriptionSummarizeConfig,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Vec<SummarizedEntityRow>> {
    let context = SummarizeContext {
        model,
        prompt_template,
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
    prompt_template: &PromptTemplate,
    tokenizer: &dyn Tokenizer,
    rows: &[RelationshipRow],
    config: DescriptionSummarizeConfig,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Vec<SummarizedRelationshipRow>> {
    let context = SummarizeContext {
        model,
        prompt_template,
        tokenizer,
        max_length: config.max_length,
        max_input_tokens: config.max_input_tokens,
    };
    let mut stream = stream::iter(rows.iter().cloned().enumerate())
        .map(|(index, row)| {
            let summarize_context = context;
            async move {
                let id = python_json_string_array([row.source.as_str(), row.target.as_str()])?;
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

fn render_summarization_prompt(
    template: &PromptTemplate,
    entity_name_json: &str,
    descriptions: &[String],
    max_length: usize,
) -> Result<String> {
    template
        .bind(&SummarizePromptValues {
            entity_name: entity_name_json.to_owned(),
            description_list: python_json_string_array(descriptions.iter().map(String::as_str))?,
            max_length,
        })?
        .render()
}

fn python_json_string_array<'a>(values: impl IntoIterator<Item = &'a str>) -> Result<String> {
    let values = values
        .into_iter()
        .map(serde_json::to_string)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(format!("[{}]", values.join(", ")))
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
        context.prompt_template,
        entity_name_json,
        &[],
        context.max_length,
    )?;
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
                context.prompt_template,
                entity_name_json,
                &collected,
                context.max_length,
            )?;
            context
                .model
                .complete(CompletionRequest::new(vec![ChatMessage::user(prompt)]))
                .await?
                .content()?
                .clone_into(&mut result);
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
        let template = crate::prompts::PromptRepository::new(".")
            .load(crate::prompts::PromptKind::SummarizeDescriptions, None)
            .await
            .expect("summary template");
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
            &template,
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

    #[test]
    fn test_should_serialize_summary_lists_like_python_json_dumps() {
        assert_eq!(
            python_json_string_array(["西门庆", "quoted \"name\""])
                .expect("serialize description list"),
            "[\"西门庆\", \"quoted \\\"name\\\"\"]",
        );
    }

    #[tokio::test]
    async fn test_should_summarize_graph_with_cumulative_progress() {
        let template = crate::prompts::PromptRepository::new(".")
            .load(crate::prompts::PromptKind::SummarizeDescriptions, None)
            .await
            .expect("summary template");
        let graph = ExtractedGraph {
            entities: vec![entity("B", "second", "tu-2"), entity("A", "first", "tu-1")],
            relationships: vec![
                relationship("B", "A", "second edge", "tu-2"),
                relationship("A", "B", "first edge", "tu-1"),
            ],
        };
        let progress = Arc::new(std::sync::Mutex::new(Vec::new()));
        let progress_ref = Arc::clone(&progress);

        let summarized = summarize_graph(
            &UnusedModel,
            &template,
            &WhitespaceTokenizer,
            &graph,
            DescriptionSummarizeConfig {
                max_length: 100,
                max_input_tokens: 100,
                concurrency: 2,
            },
            &|completed, total| {
                progress_ref
                    .lock()
                    .expect("progress lock")
                    .push((completed, total));
            },
        )
        .await
        .expect("graph summarization");

        assert_eq!(
            progress.lock().expect("progress lock").as_slice(),
            &[(1, 4), (2, 4), (3, 4), (4, 4)]
        );
        assert_eq!(
            summarized
                .entities
                .iter()
                .map(|row| row.title.as_str())
                .collect::<Vec<_>>(),
            vec!["B", "A"]
        );
        assert_eq!(
            summarized
                .relationships
                .iter()
                .map(|row| (row.source.as_str(), row.target.as_str()))
                .collect::<Vec<_>>(),
            vec![("B", "A"), ("A", "B")]
        );
    }

    fn entity(title: &str, description: &str, text_unit_id: &str) -> EntityRow {
        EntityRow {
            title: title.to_owned(),
            entity_type: "person".to_owned(),
            description: vec![description.to_owned()],
            text_unit_ids: vec![text_unit_id.to_owned()],
            frequency: 1,
        }
    }

    fn relationship(
        source: &str,
        target: &str,
        description: &str,
        text_unit_id: &str,
    ) -> RelationshipRow {
        RelationshipRow {
            source: source.to_owned(),
            target: target.to_owned(),
            description: vec![description.to_owned()],
            text_unit_ids: vec![text_unit_id.to_owned()],
            weight: 1.0,
        }
    }
}
