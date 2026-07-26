//! Canonical completion response wire types.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ChatRole, ModelCallMetadata};
use crate::{LlmError, Result};

/// Completion token usage.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CompletionUsage {
    /// Output tokens.
    pub completion_tokens: u64,
    /// Input tokens.
    pub prompt_tokens: u64,
    /// Total tokens.
    pub total_tokens: u64,
    /// Provider usage extensions.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Canonical assistant message in a completion choice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionMessage {
    /// Assistant role.
    pub role: ChatRole,
    /// Text response, if present.
    pub content: Option<String>,
    /// Provider refusal text.
    pub refusal: Option<String>,
    /// Provider reasoning text.
    pub reasoning_content: Option<String>,
    /// Tool calls.
    pub tool_calls: Option<Vec<Value>>,
    /// Legacy function call.
    pub function_call: Option<Value>,
    /// Provider annotations.
    pub annotations: Option<Value>,
    /// Provider audio payload.
    pub audio: Option<Value>,
    /// Unknown provider fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// One canonical completion choice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionChoice {
    /// Choice index.
    pub index: u32,
    /// Assistant message.
    pub message: CompletionMessage,
    /// Provider finish reason.
    pub finish_reason: Option<String>,
    /// Provider log probabilities.
    pub logprobs: Option<Value>,
    /// Unknown provider fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Provider-neutral completion response compatible with `GraphRAG` cache JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// Provider response id.
    pub id: String,
    /// Completion choices.
    pub choices: Vec<CompletionChoice>,
    /// Unix creation timestamp.
    pub created: u64,
    /// Provider model name.
    pub model: String,
    /// Wire object kind.
    pub object: String,
    /// Provider usage counters.
    pub usage: Option<CompletionUsage>,
    /// Provider service tier.
    pub service_tier: Option<String>,
    /// Provider system fingerprint.
    pub system_fingerprint: Option<String>,
    /// Unknown and computed GraphRAG/LiteLLM fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
    /// Local metadata excluded from cache JSON.
    #[serde(skip)]
    pub metadata: ModelCallMetadata,
}

impl CompletionResponse {
    /// Return the first completion choice.
    #[must_use]
    pub fn first_choice(&self) -> Option<&CompletionChoice> {
        self.choices.first()
    }

    /// Return the first choice's content using `GraphRAG` semantics.
    ///
    /// A present choice whose message content is either `None` or an empty
    /// string produces an empty string. Provider reasoning content remains
    /// available through [`Self::reasoning_content`] and is never substituted
    /// for ordinary completion content.
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::InvalidResponse`] when the response has no choices.
    pub fn content(&self) -> Result<&str> {
        self.first_choice()
            .map(|choice| choice.message.content.as_deref().unwrap_or_default())
            .ok_or_else(|| LlmError::InvalidResponse {
                model_instance: self.model.clone(),
                operation: "completion",
                message: "missing choices[0]".to_owned(),
            })
    }

    /// Return the first choice's reasoning content.
    #[must_use]
    pub fn reasoning_content(&self) -> Option<&str> {
        self.first_choice()
            .and_then(|choice| choice.message.reasoning_content.as_deref())
    }

    /// Construct a canonical text response for tests and mock providers.
    #[must_use]
    pub fn text_for_test(model: impl Into<String>, content: impl Into<String>) -> Self {
        let model = model.into();
        Self {
            id: "mock-completion".to_owned(),
            choices: vec![CompletionChoice {
                index: 0,
                message: CompletionMessage {
                    role: ChatRole::Assistant,
                    content: Some(content.into()),
                    refusal: None,
                    reasoning_content: None,
                    tool_calls: None,
                    function_call: None,
                    annotations: None,
                    audio: None,
                    extra: BTreeMap::new(),
                },
                finish_reason: Some("stop".to_owned()),
                logprobs: None,
                extra: BTreeMap::new(),
            }],
            created: 0,
            model,
            object: "chat.completion".to_owned(),
            usage: None,
            service_tier: None,
            system_fingerprint: None,
            extra: BTreeMap::new(),
            metadata: ModelCallMetadata::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CompletionResponse;
    use crate::LlmError;

    #[test]
    fn test_should_return_non_empty_completion_content_verbatim() {
        let response = CompletionResponse::text_for_test("model", "answer");

        assert_eq!(response.content().expect("content should exist"), "answer");
    }

    #[test]
    fn test_should_accept_explicit_empty_completion_content() {
        let response = CompletionResponse::text_for_test("model", "");

        assert_eq!(response.content().expect("empty content is valid"), "");
    }

    #[test]
    fn test_should_treat_null_completion_content_as_empty() {
        let mut response = CompletionResponse::text_for_test("model", "ignored");
        response.choices[0].message.content = None;

        assert_eq!(response.content().expect("null content is valid"), "");
    }

    #[test]
    fn test_should_reject_completion_without_choices() {
        let mut response = CompletionResponse::text_for_test("model", "ignored");
        response.choices.clear();

        let error = response.content().expect_err("missing choice must fail");
        assert!(matches!(
            error,
            LlmError::InvalidResponse { message, .. } if message == "missing choices[0]"
        ));
    }

    #[test]
    fn test_should_not_substitute_reasoning_for_empty_completion_content() {
        let mut response = CompletionResponse::text_for_test("model", "");
        response.choices[0].message.reasoning_content = Some("private reasoning".to_owned());

        assert_eq!(response.content().expect("empty content is valid"), "");
        assert_eq!(response.reasoning_content(), Some("private reasoning"));
    }
}
