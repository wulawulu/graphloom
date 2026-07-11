//! Entity and relationship extraction operations.

use graphloom_llm::{ChatMessage, CompletionModel, CompletionRequest, parse_graph_tuples};
use serde::Serialize;

use super::{RawEntityRow, RawRelationshipRow, TextUnitInput};
use crate::{
    Result,
    prompts::{Prompt, PromptTemplate},
};

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
    use graphloom_llm::MockCompletionModel;

    use super::*;

    #[tokio::test]
    async fn test_should_append_gleaned_graph_records() {
        let repository = crate::prompts::PromptRepository::new(".");
        let extraction_template = repository
            .load(crate::prompts::PromptKind::ExtractGraph, None)
            .await
            .expect("extraction template");
        let continue_template = repository
            .load(crate::prompts::PromptKind::ExtractGraphContinue, None)
            .await
            .expect("continue template");
        let loop_template = repository
            .load(crate::prompts::PromptKind::ExtractGraphLoop, None)
            .await
            .expect("loop template");
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
}
