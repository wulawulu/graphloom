//! Entity type discovery for GraphRAG prompt tuning.

use std::sync::Arc;

use graphloom_llm::{ChatMessage, CompletionModel, CompletionRequest};
use serde::Deserialize;

use super::meta_prompts::{
    DEFAULT_TASK, ENTITY_TYPE_GENERATION_JSON_PROMPT, ENTITY_TYPE_GENERATION_PROMPT,
};
use crate::{GraphLoomError, Result};

#[derive(Debug, Deserialize)]
struct EntityTypesResponse {
    entity_types: Vec<String>,
}

pub(crate) async fn generate_entity_types(
    model: &Arc<dyn CompletionModel>,
    domain: &str,
    persona: &str,
    docs: &[String],
    json_mode: bool,
    consumer: &'static str,
) -> Result<Vec<String>> {
    let formatted_task = DEFAULT_TASK.replace("{domain}", domain);
    let docs_str = docs.join("\n");

    let entity_types_prompt = if json_mode {
        ENTITY_TYPE_GENERATION_JSON_PROMPT
            .replace("{task}", &formatted_task)
            .replace("{input_text}", &docs_str)
    } else {
        ENTITY_TYPE_GENERATION_PROMPT
            .replace("{task}", &formatted_task)
            .replace("{input_text}", &docs_str)
    };

    let messages = vec![
        ChatMessage::system(persona.to_owned()),
        ChatMessage::user(entity_types_prompt),
    ];

    if json_mode {
        let mut request = CompletionRequest::new(messages);
        request.response_format = Some(serde_json::json!({"type": "json_object"}));
        let response = model.complete(request).await.map_err(GraphLoomError::Llm)?;
        let content = response.content().map_err(GraphLoomError::Llm)?;

        if let Ok(parsed) = serde_json::from_str::<EntityTypesResponse>(content) {
            if parsed.entity_types.is_empty() {
                return Err(GraphLoomError::InvalidData {
                    workflow: consumer,
                    message: "entity types JSON parse returned empty list".to_owned(),
                });
            }
            return Ok(parsed.entity_types);
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(content)
            && let Some(types) = value.get("entity_types").and_then(|v| v.as_array())
        {
            let types: Vec<String> = types
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            if types.is_empty() {
                return Err(GraphLoomError::InvalidData {
                    workflow: consumer,
                    message: "entity types JSON array is empty".to_owned(),
                });
            }
            return Ok(types);
        }
        return Err(GraphLoomError::InvalidData {
            workflow: consumer,
            message: format!(
                "failed to parse entity types from JSON response: {:.200}",
                content
            ),
        });
    }

    let request = CompletionRequest::new(messages);
    let response = model.complete(request).await.map_err(GraphLoomError::Llm)?;
    let content = response.content().map_err(GraphLoomError::Llm)?;
    let trimmed = content.trim().to_owned();

    if trimmed.is_empty() {
        return Err(GraphLoomError::InvalidData {
            workflow: consumer,
            message: "entity types generation returned empty content".to_owned(),
        });
    }

    let types: Vec<String> = trimmed
        .split(',')
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .collect();

    if types.is_empty() {
        return Err(GraphLoomError::InvalidData {
            workflow: consumer,
            message: "entity types generation produced no valid types".to_owned(),
        });
    }

    Ok(types)
}
