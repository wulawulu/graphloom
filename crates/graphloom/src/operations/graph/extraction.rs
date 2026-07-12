//! Entity and relationship extraction operations.

use futures_util::{StreamExt, stream};
use graphloom_llm::{ChatMessage, CompletionModel, CompletionRequest, parse_graph_tuples};
use serde::Serialize;

use super::{
    ExtractedGraph, RawEntityRow, RawRelationshipRow, TextUnitInput, filter_orphan_relationships,
    merge_entities, merge_relationships,
};
use crate::{
    GraphLoomError, Result,
    prompts::{Prompt, PromptTemplate},
};

const EXTRACT_GRAPH_CONTEXT: &str = "extract_graph";

#[derive(Debug, Clone, Copy)]
pub(crate) struct GraphExtractionConfig<'a> {
    pub(crate) entity_types: &'a [String],
    pub(crate) max_gleanings: usize,
    pub(crate) concurrency: usize,
}

pub(crate) async fn extract_graph(
    model: &dyn CompletionModel,
    extraction_template: &PromptTemplate,
    continue_template: &PromptTemplate,
    loop_template: &PromptTemplate,
    text_units: &[TextUnitInput],
    config: GraphExtractionConfig<'_>,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<ExtractedGraph> {
    let mut extraction_results = stream::iter(text_units.iter().cloned().enumerate())
        .map(|(index, text_unit)| async move {
            extract_text_unit_graph(
                model,
                extraction_template,
                continue_template,
                loop_template,
                config.entity_types,
                &text_unit,
                config.max_gleanings,
            )
            .await
            .map(|(entities, relationships)| (index, entities, relationships))
        })
        .buffer_unordered(config.concurrency.max(1));

    let mut completed = 0usize;
    let mut extracted = Vec::with_capacity(text_units.len());
    while let Some(result) = extraction_results.next().await {
        let result = result?;
        completed = completed.saturating_add(1);
        progress(completed, text_units.len());
        extracted.push(result);
    }
    extracted.sort_by_key(|(index, _, _)| *index);

    let mut raw_entities = Vec::new();
    let mut raw_relationships = Vec::new();
    for (_, entities, relationships) in extracted {
        raw_entities.extend(entities);
        raw_relationships.extend(relationships);
    }

    let entities = merge_entities(&raw_entities);
    let relationships =
        filter_orphan_relationships(merge_relationships(&raw_relationships), &entities);
    if entities.is_empty() {
        return Err(GraphLoomError::InvalidData {
            workflow: EXTRACT_GRAPH_CONTEXT,
            message: "Graph Extraction failed. No entities detected during extraction.".to_owned(),
        });
    }
    if relationships.is_empty() {
        return Err(GraphLoomError::InvalidData {
            workflow: EXTRACT_GRAPH_CONTEXT,
            message: "Graph Extraction failed. No relationships detected during extraction."
                .to_owned(),
        });
    }

    Ok(ExtractedGraph {
        entities,
        relationships,
    })
}

pub(crate) async fn extract_text_unit_graph(
    model: &dyn CompletionModel,
    extraction_template: &PromptTemplate,
    continue_template: &PromptTemplate,
    loop_template: &PromptTemplate,
    entity_types: &[String],
    text_unit: &TextUnitInput,
    max_gleanings: usize,
) -> Result<(Vec<RawEntityRow>, Vec<RawRelationshipRow>)> {
    let mut messages = vec![ChatMessage::user(
        bind_extraction_prompt(extraction_template, &text_unit.text, entity_types)?.render()?,
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

    let continue_prompt = bind_empty_prompt(continue_template)?.render()?;
    let loop_prompt = bind_empty_prompt(loop_template)?.render()?;
    for glean_index in 0..max_gleanings {
        messages.push(ChatMessage::user(continue_prompt.clone()));
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

        messages.push(ChatMessage::user(loop_prompt.clone()));
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

#[derive(Debug, Serialize)]
struct EmptyPromptValues {}

fn bind_extraction_prompt(
    template: &PromptTemplate,
    input_text: &str,
    entity_types: &[String],
) -> Result<Prompt> {
    template.bind(&ExtractPromptValues {
        entity_types: entity_types.join(","),
        input_text,
    })
}

fn bind_empty_prompt(template: &PromptTemplate) -> Result<Prompt> {
    template.bind(&EmptyPromptValues {})
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use async_trait::async_trait;
    use graphloom_llm::{CompletionResponse, LlmError, MockCompletionModel};
    use tokio::time::sleep;

    use super::*;

    #[tokio::test]
    async fn test_should_append_gleaned_graph_records() {
        let (extraction_template, continue_template, loop_template) = extraction_templates().await;
        let model = MockCompletionModel::new(
            "mock",
            vec![
                "(\"entity\"<|>Alice<|>person<|>Alice)##".to_owned(),
                "(\"entity\"<|>Bob<|>person<|>Bob)##(\"relationship\"<|>Alice<|>Bob<|>knows<|>1)##\
                 <|COMPLETE|>"
                    .to_owned(),
            ],
        );
        let text_unit = TextUnitInput {
            id: "tu-1".to_owned(),
            text: "Alice knows Bob.".to_owned(),
        };

        let (entities, relationships) = extract_text_unit_graph(
            &model,
            &extraction_template,
            &continue_template,
            &loop_template,
            &[String::from("person")],
            &text_unit,
            1,
        )
        .await
        .expect("graph extraction should succeed");

        assert_eq!(entities.len(), 2);
        assert_eq!(relationships.len(), 1);
        assert_eq!(relationships[0].source, "ALICE");
        assert_eq!(relationships[0].target, "BOB");
    }

    #[tokio::test]
    async fn test_should_extract_merge_and_filter_orphan_relationships() {
        let (extraction_template, continue_template, loop_template) = extraction_templates().await;
        let model = MockCompletionModel::new(
            "mock",
            vec![
                graph_records(
                    &[("Alice", "first"), ("Bob", "second")],
                    &[("Alice", "Bob", "knows")],
                ),
                graph_records(
                    &[("Alice", "third"), ("Carol", "fourth")],
                    &[
                        ("Alice", "Carol", "mentors"),
                        ("Alice", "Missing", "orphan"),
                    ],
                ),
            ],
        );
        let text_units = text_units();
        let progress = Arc::new(Mutex::new(Vec::new()));
        let progress_ref = Arc::clone(&progress);

        let graph = extract_graph(
            &model,
            &extraction_template,
            &continue_template,
            &loop_template,
            &text_units,
            extraction_config(1),
            &|completed, total| {
                progress_ref
                    .lock()
                    .expect("progress lock")
                    .push((completed, total));
            },
        )
        .await
        .expect("batch extraction");

        let alice = graph
            .entities
            .iter()
            .find(|entity| entity.title == "ALICE")
            .expect("Alice entity");
        assert_eq!(alice.frequency, 2);
        assert_eq!(alice.text_unit_ids, vec!["tu-1", "tu-2"]);
        assert_eq!(graph.relationships.len(), 2);
        assert!(
            graph
                .relationships
                .iter()
                .all(|relationship| relationship.target != "MISSING")
        );
        assert_eq!(
            progress.lock().expect("progress lock").last(),
            Some(&(2, 2))
        );
    }

    #[tokio::test]
    async fn test_should_preserve_input_order_when_extractions_finish_out_of_order() {
        let (extraction_template, continue_template, loop_template) = extraction_templates().await;
        let graph = extract_graph(
            &DelayedGraphModel,
            &extraction_template,
            &continue_template,
            &loop_template,
            &text_units(),
            extraction_config(2),
            &|_, _| {},
        )
        .await
        .expect("concurrent extraction");

        let alice = graph
            .entities
            .iter()
            .find(|entity| entity.title == "ALICE")
            .expect("Alice entity");
        assert_eq!(alice.description, vec!["slow-first", "fast-second"]);
        assert_eq!(alice.text_unit_ids, vec!["tu-1", "tu-2"]);
    }

    #[tokio::test]
    async fn test_should_reject_batch_extraction_without_entities() {
        let error = extract_with_response("<|COMPLETE|>")
            .await
            .expect_err("empty entities must fail");
        assert!(matches!(error, GraphLoomError::InvalidData { .. }));
        assert!(
            error
                .to_string()
                .contains("No entities detected during extraction")
        );
    }

    #[tokio::test]
    async fn test_should_reject_batch_extraction_without_relationships() {
        let error = extract_with_response("(\"entity\"<|>Alice<|>person<|>alone)##<|COMPLETE|>")
            .await
            .expect_err("empty relationships must fail");
        assert!(matches!(error, GraphLoomError::InvalidData { .. }));
        assert!(
            error
                .to_string()
                .contains("No relationships detected during extraction")
        );
    }

    #[derive(Debug)]
    struct DelayedGraphModel;

    #[async_trait]
    impl CompletionModel for DelayedGraphModel {
        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> graphloom_llm::Result<CompletionResponse> {
            let prompt = request
                .messages
                .first()
                .map(|message| message.content.as_str())
                .unwrap_or_default();
            let description = if prompt.contains("tu-1") {
                sleep(Duration::from_millis(50)).await;
                "slow-first"
            } else if prompt.contains("tu-2") {
                sleep(Duration::from_millis(1)).await;
                "fast-second"
            } else {
                return Err(LlmError::InvalidResponse {
                    model_instance: "delayed-graph".to_owned(),
                    operation: "completion",
                    message: "unknown text unit".to_owned(),
                });
            };
            Ok(CompletionResponse {
                content: graph_records(
                    &[("Alice", description), ("Bob", "shared")],
                    &[("Alice", "Bob", description)],
                ),
                usage: None,
                request_id: None,
            })
        }
    }

    async fn extraction_templates() -> (PromptTemplate, PromptTemplate, PromptTemplate) {
        let repository = crate::prompts::PromptRepository::new(".");
        let extraction = repository
            .load(crate::prompts::PromptKind::ExtractGraph, None)
            .await
            .expect("extraction template");
        let continuation = repository
            .load(crate::prompts::PromptKind::ExtractGraphContinue, None)
            .await
            .expect("continue template");
        let loop_template = repository
            .load(crate::prompts::PromptKind::ExtractGraphLoop, None)
            .await
            .expect("loop template");
        (extraction, continuation, loop_template)
    }

    fn text_units() -> Vec<TextUnitInput> {
        vec![
            TextUnitInput {
                id: "tu-1".to_owned(),
                text: "tu-1".to_owned(),
            },
            TextUnitInput {
                id: "tu-2".to_owned(),
                text: "tu-2".to_owned(),
            },
        ]
    }

    fn extraction_config(concurrency: usize) -> GraphExtractionConfig<'static> {
        GraphExtractionConfig {
            entity_types: &[],
            max_gleanings: 0,
            concurrency,
        }
    }

    async fn extract_with_response(response: &str) -> Result<ExtractedGraph> {
        let (extraction_template, continue_template, loop_template) = extraction_templates().await;
        extract_graph(
            &MockCompletionModel::new("mock", vec![response.to_owned()]),
            &extraction_template,
            &continue_template,
            &loop_template,
            &[TextUnitInput {
                id: "tu-1".to_owned(),
                text: "empty graph".to_owned(),
            }],
            extraction_config(1),
            &|_, _| {},
        )
        .await
    }

    fn graph_records(entities: &[(&str, &str)], relationships: &[(&str, &str, &str)]) -> String {
        let mut records = entities
            .iter()
            .map(|(title, description)| {
                format!("(\"entity\"<|>{title}<|>person<|>{description})##")
            })
            .collect::<String>();
        records.extend(relationships.iter().map(|(source, target, description)| {
            format!("(\"relationship\"<|>{source}<|>{target}<|>{description}<|>1)##")
        }));
        records.push_str("<|COMPLETE|>");
        records
    }
}
