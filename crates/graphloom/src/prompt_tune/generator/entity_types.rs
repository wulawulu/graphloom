//! Entity type discovery for GraphRAG prompt tuning.

use std::sync::Arc;

use graphloom_llm::{ChatMessage, CompletionModel, CompletionRequest, try_parse_json_object};
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
        // Plain CompletionRequest — no provider-specific response_format.
        // JSON extraction and validation are handled on the client side,
        // the same way Index workflows parse community reports.
        let request = CompletionRequest::new(messages);
        let response = model.complete(request).await.map_err(GraphLoomError::Llm)?;
        let content = response.content().map_err(GraphLoomError::Llm)?;

        let (_, value) =
            try_parse_json_object(content).map_err(|source| GraphLoomError::InvalidData {
                workflow: consumer,
                message: format!(
                    "failed to parse entity types JSON response: {source} ({:.200})",
                    content
                ),
            })?;

        let parsed: EntityTypesResponse =
            serde_json::from_value(value).map_err(|source| GraphLoomError::InvalidData {
                workflow: consumer,
                message: format!("invalid entity types JSON schema: {source}"),
            })?;

        // Empty arrays are valid — the upper level converts them to None
        // and falls back to the untyped extract-graph prompt.
        return Ok(parsed.entity_types);
    }

    let request = CompletionRequest::new(messages);
    let response = model.complete(request).await.map_err(GraphLoomError::Llm)?;
    let content = response.content().map_err(GraphLoomError::Llm)?;

    // GraphRAG 3.1.0 returns response.content directly without trimming
    // or validating for whitespace-only content.  Whitespace-only input
    // produces an empty list here; the upper level translates that to
    // None (untyped fallback).
    let types: Vec<String> = content
        .trim()
        .split(',')
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .collect();

    Ok(types)
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use graphloom_llm::CompletionResponse;

    use super::*;

    /// A recording mock that captures every `CompletionRequest` and replies
    /// with pre-configured responses in order.
    #[derive(Debug)]
    struct RecordingModel {
        responses: std::sync::Mutex<Vec<String>>,
        requests: std::sync::Mutex<Vec<CompletionRequest>>,
    }

    impl RecordingModel {
        fn new(responses: Vec<String>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses),
                requests: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl CompletionModel for RecordingModel {
        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> std::result::Result<CompletionResponse, graphloom_llm::LlmError> {
            self.requests.lock().unwrap().push(request);
            let mut responses = self.responses.lock().unwrap();
            let content = if responses.is_empty() {
                "".to_owned()
            } else {
                responses.remove(0)
            };
            Ok(CompletionResponse::text_for_test(
                "test.entity_types".to_owned(),
                content,
            ))
        }
    }

    fn test_model(responses: Vec<&str>) -> (Arc<RecordingModel>, &'static str) {
        (
            Arc::new(RecordingModel::new(
                responses.into_iter().map(String::from).collect(),
            )),
            "test.entity_types",
        )
    }

    // ---- json_mode = true --------------------------------------------------

    #[tokio::test]
    async fn json_request_has_no_response_format() {
        let (model, consumer) = test_model(vec![r#"{"entity_types":["person"]}"#]);
        let _ = generate_entity_types(
            &(model.clone() as Arc<dyn CompletionModel>),
            "SW",
            "Expert.",
            &["doc".into()],
            true,
            consumer,
        )
        .await
        .expect("should succeed");

        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].response_format, None);
    }

    #[tokio::test]
    async fn json_request_messages_count_and_roles() {
        let (model, consumer) = test_model(vec![r#"{"entity_types":["person"]}"#]);
        let _ = generate_entity_types(
            &(model.clone() as Arc<dyn CompletionModel>),
            "SW",
            "Expert.",
            &["doc".into()],
            true,
            consumer,
        )
        .await
        .expect("should succeed");

        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 1, "one request");
        assert_eq!(requests[0].messages.len(), 2, "system + user");
        assert_eq!(
            requests[0].messages[0].role,
            graphloom_llm::ChatRole::System
        );
        assert_eq!(requests[0].messages[1].role, graphloom_llm::ChatRole::User);
    }

    #[tokio::test]
    async fn json_valid_response_parses_entity_types() {
        let (model, consumer) = test_model(vec![r#"{"entity_types":["person","organization"]}"#]);
        let types = generate_entity_types(
            &(model.clone() as Arc<dyn CompletionModel>),
            "SW",
            "Expert.",
            &["doc".into()],
            true,
            consumer,
        )
        .await
        .expect("should succeed");

        assert_eq!(types, vec!["person", "organization"]);
    }

    #[tokio::test]
    async fn json_empty_array_returns_ok_empty_vec() {
        let (model, consumer) = test_model(vec![r#"{"entity_types":[]}"#]);
        let types = generate_entity_types(
            &(model.clone() as Arc<dyn CompletionModel>),
            "SW",
            "Expert.",
            &["doc".into()],
            true,
            consumer,
        )
        .await
        .expect("should succeed");

        assert!(types.is_empty());
    }

    #[tokio::test]
    async fn json_missing_field_returns_error() {
        let (model, consumer) = test_model(vec![r#"{"types":["person"]}"#]);
        let err = generate_entity_types(
            &(model.clone() as Arc<dyn CompletionModel>),
            "SW",
            "Expert.",
            &["doc".into()],
            true,
            consumer,
        )
        .await
        .expect_err("should fail on missing field");

        let msg = format!("{err:?}");
        assert!(
            msg.contains("entity_types") || msg.contains("invalid"),
            "error should mention the missing field: {msg}"
        );
    }

    #[tokio::test]
    async fn json_wrong_field_type_returns_error() {
        let (model, consumer) = test_model(vec![r#"{"entity_types":"person"}"#]);
        let err = generate_entity_types(
            &(model.clone() as Arc<dyn CompletionModel>),
            "SW",
            "Expert.",
            &["doc".into()],
            true,
            consumer,
        )
        .await
        .expect_err("should fail on wrong field type");

        let msg = format!("{err:?}");
        assert!(
            msg.contains("entity_types") || msg.contains("invalid") || msg.contains("schema"),
            "error should mention the type mismatch: {msg}"
        );
    }

    #[tokio::test]
    async fn json_invalid_json_returns_error() {
        let (model, consumer) = test_model(vec!["not-json"]);
        let err = generate_entity_types(
            &(model.clone() as Arc<dyn CompletionModel>),
            "SW",
            "Expert.",
            &["doc".into()],
            true,
            consumer,
        )
        .await
        .expect_err("should fail on invalid JSON");

        let msg = format!("{err:?}");
        assert!(
            msg.contains("not-json") || msg.contains("parse") || msg.contains("json"),
            "error should reference the content: {msg}"
        );
    }

    #[tokio::test]
    async fn json_code_fence_is_handled() {
        // try_parse_json_object strips ```json fences
        let (model, consumer) = test_model(vec!["```json\n{\"entity_types\":[\"person\"]}\n```"]);
        let types = generate_entity_types(
            &(model.clone() as Arc<dyn CompletionModel>),
            "SW",
            "Expert.",
            &["doc".into()],
            true,
            consumer,
        )
        .await
        .expect("should parse code-fenced JSON");

        assert_eq!(types, vec!["person"]);
    }

    // ---- json_mode = false -------------------------------------------------

    #[tokio::test]
    async fn non_json_trims_and_splits_comma_separated() {
        let (model, consumer) = test_model(vec![" person , organization "]);
        let types = generate_entity_types(
            &(model.clone() as Arc<dyn CompletionModel>),
            "SW",
            "Expert.",
            &["doc".into()],
            false,
            consumer,
        )
        .await
        .expect("should succeed");

        assert_eq!(types, vec!["person", "organization"]);
    }

    #[tokio::test]
    async fn non_json_whitespace_only_returns_empty_vec() {
        // GraphRAG 3.1.0 returns response.content directly; whitespace-only
        // produces an empty list so the upper level falls back to untyped.
        let (model, consumer) = test_model(vec!["   "]);
        let types = generate_entity_types(
            &(model.clone() as Arc<dyn CompletionModel>),
            "SW",
            "Expert.",
            &["doc".into()],
            false,
            consumer,
        )
        .await
        .expect("should succeed");

        assert!(types.is_empty());
    }

    #[tokio::test]
    async fn non_json_empty_string_returns_empty_vec() {
        let (model, consumer) = test_model(vec![""]);
        let types = generate_entity_types(
            &(model.clone() as Arc<dyn CompletionModel>),
            "SW",
            "Expert.",
            &["doc".into()],
            false,
            consumer,
        )
        .await
        .expect("should succeed");

        assert!(types.is_empty());
    }
}
