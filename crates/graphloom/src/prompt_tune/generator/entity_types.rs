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
                message: format!(
                    "entity types JSON response does not match the expected structure: {source}"
                ),
            })?;

        // Empty arrays are valid — the upper level converts them to None
        // and falls back to the untyped extract-graph prompt.
        return Ok(parsed.entity_types);
    }

    let request = CompletionRequest::new(messages);
    let response = model.complete(request).await.map_err(GraphLoomError::Llm)?;
    let content = response.content().map_err(GraphLoomError::Llm)?;

    // The non-JSON path is an internal GraphLoom normalization path.
    // It converts a comma-separated response into entity type values.
    // Empty or whitespace-only content produces an empty list so the
    // caller can use the untyped fallback.
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

    use super::*;
    use crate::prompt_tune::test_support::RecordingModel;

    /// Independent GraphRAG v3.1.0 request golden.
    ///
    /// Source:
    /// - repository: microsoft/graphrag
    /// - commit: 7fc6607edda3d387d23e52ededbf8a75b6730f97
    /// - DEFAULT_TASK: packages/graphrag/graphrag/prompt_tune/defaults.py
    /// - prompt: packages/graphrag/graphrag/prompt_tune/prompt/entity_types.py
    /// - inputs: domain="SW", input_text="doc"
    ///
    /// Generated with Python `.format()` without trimming or newline normalization.
    const GRAPH_RAG_ENTITY_TYPES_JSON_REQUEST: &str =
        include_str!("../fixtures/entity_types_json_request.txt");

    /// UTF-8 length of the GraphRAG golden.
    const EXPECTED_USER_CONTENT_LEN: usize = 2812;

    fn test_model(responses: Vec<&str>) -> (Arc<RecordingModel>, &'static str) {
        (
            Arc::new(RecordingModel::new(
                responses.into_iter().map(String::from).collect(),
            )),
            "test.entity_types",
        )
    }

    // ---- json_mode = true: request contract -------------------------------

    #[tokio::test]
    async fn json_request_matches_graphrag_golden() {
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

        let request = &requests[0];

        // No provider-specific response_format
        assert_eq!(request.response_format, None);

        // Messages: system + user
        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.messages[0].role, graphloom_llm::ChatRole::System);
        assert_eq!(request.messages[1].role, graphloom_llm::ChatRole::User);

        // System content — exact bytes
        assert_eq!(request.messages[0].content.as_str().as_bytes(), b"Expert.");

        // User content — independent GraphRAG 3.1.0 golden.
        // This does NOT depend on the current GraphLoom prompt resources.
        let content = request.messages[1].content.as_str();
        let actual = content.as_bytes();
        assert_eq!(actual.len(), EXPECTED_USER_CONTENT_LEN);
        assert_eq!(actual, GRAPH_RAG_ENTITY_TYPES_JSON_REQUEST.as_bytes());
        assert!(content.contains(
            "The user's task is to \nIdentify the relations and structure of the community of \
             interest, specifically within the SW domain.\n.",
        ));
        assert!(content.contains(
            "Task: \nIdentify the relations and structure of the community of interest, \
             specifically within the SW domain.\n\nText: doc",
        ));
    }

    // ---- json_mode = true: JSON parsing -----------------------------------

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
    async fn json_missing_entity_types_field_returns_error() {
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
            msg.contains("does not match the expected structure"),
            "error should mention structure mismatch: {msg}"
        );
        assert!(
            msg.contains("entity_types"),
            "error should mention the missing field name: {msg}"
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
            msg.contains("does not match the expected structure"),
            "error should mention structure mismatch: {msg}"
        );
        assert!(
            msg.contains("entity_types"),
            "error should mention the field name: {msg}"
        );
    }

    #[tokio::test]
    async fn json_invalid_json_returns_parse_error() {
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
            msg.contains("failed to parse entity types JSON response"),
            "error should mention JSON parse failure: {msg}"
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

    // ---- json_mode = false: GraphLoom normalization -----------------------

    #[tokio::test]
    async fn non_json_normalizes_comma_separated_types() {
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
    async fn non_json_whitespace_only_produces_empty_list() {
        // GraphLoom normalizes the non-JSON response into a list;
        // an empty list allows the caller to select the untyped path.
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
    async fn non_json_empty_content_produces_empty_list() {
        // GraphLoom normalizes the non-JSON response into a list;
        // an empty list allows the caller to select the untyped path.
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
