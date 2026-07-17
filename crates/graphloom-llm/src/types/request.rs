//! Canonical completion and embedding requests.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{LlmError, Result};

const COMPLETION_RESERVED_FIELDS: &[&str] = &[
    "messages",
    "response_format",
    "temperature",
    "top_p",
    "max_tokens",
    "max_completion_tokens",
    "n",
    "seed",
    "stop",
    "tools",
    "tool_choice",
    "presence_penalty",
    "frequency_penalty",
    "stream",
    "model",
];
const EMBEDDING_RESERVED_FIELDS: &[&str] =
    &["input", "dimensions", "encoding_format", "user", "model"];

/// OpenAI-compatible chat role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    /// System instruction.
    System,
    /// User message.
    User,
    /// Assistant message.
    Assistant,
    /// Developer instruction.
    Developer,
}

/// Text or multipart message content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Plain text content.
    Text(String),
    /// Provider-neutral multipart content.
    Parts(Vec<Value>),
}

impl Default for MessageContent {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

impl MessageContent {
    /// Return plain text when this content is textual.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Parts(_) => None,
        }
    }

    /// Return text content or an empty string for multipart content.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.as_text().unwrap_or_default()
    }

    /// Return whether textual content contains a pattern.
    #[must_use]
    pub fn contains(&self, pattern: &str) -> bool {
        self.as_str().contains(pattern)
    }
}

/// Chat message used by completion models.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Message role.
    pub role: ChatRole,
    /// Message content.
    pub content: MessageContent,
    /// Provider extensions preserved in the canonical request.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl ChatMessage {
    /// Create a system message.
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self::text(ChatRole::System, content)
    }

    /// Create a user message.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self::text(ChatRole::User, content)
    }

    /// Create an assistant message.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        let mut message = Self::text(ChatRole::Assistant, content);
        // GraphRAG's CompletionMessagesBuilder includes this nullable OpenAI field when it
        // reconstructs assistant history. It is part of the v4 cache key for gleaning calls.
        message.extra.insert("refusal".to_owned(), Value::Null);
        message
    }

    fn text(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: MessageContent::Text(content.into()),
            extra: BTreeMap::new(),
        }
    }
}

/// Completion request kwargs used for cache keys and provider adapters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// Chat messages.
    pub messages: Vec<ChatMessage>,
    /// Structured response format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Value>,
    /// Sampling temperature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Nucleus sampling threshold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    /// Legacy maximum generated tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Maximum generated completion tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    /// Number of choices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    /// Deterministic sampling seed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    /// Stop sequence configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<Value>,
    /// Tool definitions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
    /// Tool-selection configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    /// Presence penalty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f64>,
    /// Frequency penalty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f64>,
    /// Streaming mode. Cached middleware bypasses true values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    /// Additional provider-neutral kwargs.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl CompletionRequest {
    /// Create a request containing only messages.
    #[must_use]
    pub fn new(messages: Vec<ChatMessage>) -> Self {
        Self {
            messages,
            response_format: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            max_completion_tokens: None,
            n: None,
            seed: None,
            stop: None,
            tools: None,
            tool_choice: None,
            presence_penalty: None,
            frequency_penalty: None,
            stream: None,
            extra: BTreeMap::new(),
        }
    }

    /// Validate that provider extensions cannot override canonical fields.
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::InvalidRequest`] when `extra` contains a reserved field.
    pub fn validate(&self) -> Result<()> {
        validate_extra_fields(
            &self.extra,
            COMPLETION_RESERVED_FIELDS,
            "validate completion request",
        )
    }
}

/// Embedding request kwargs used for cache keys and provider adapters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    /// Input texts in provider order.
    pub input: Vec<String>,
    /// Optional embedding dimensions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<u32>,
    /// Optional wire encoding format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding_format: Option<String>,
    /// Optional provider user identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Additional provider-neutral kwargs.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl EmbeddingRequest {
    /// Create an embedding request for text inputs.
    #[must_use]
    pub fn new(input: Vec<String>) -> Self {
        Self {
            input,
            dimensions: None,
            encoding_format: None,
            user: None,
            extra: BTreeMap::new(),
        }
    }

    /// Validate that provider extensions cannot override canonical fields.
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::InvalidRequest`] when `extra` contains a reserved field.
    pub fn validate(&self) -> Result<()> {
        validate_extra_fields(
            &self.extra,
            EMBEDDING_RESERVED_FIELDS,
            "validate embedding request",
        )
    }
}

fn validate_extra_fields(
    extra: &BTreeMap<String, Value>,
    reserved: &[&str],
    operation: &'static str,
) -> Result<()> {
    if let Some(field) = extra
        .keys()
        .find(|field| reserved.contains(&field.as_str()))
    {
        return Err(LlmError::InvalidRequest {
            operation,
            message: format!("provider extra field {field:?} conflicts with a canonical field"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::ChatMessage;

    #[test]
    fn test_should_match_graphrag_assistant_message_shape() {
        assert_eq!(
            serde_json::to_value(ChatMessage::assistant("response"))
                .expect("serialize assistant message"),
            json!({
                "role": "assistant",
                "content": "response",
                "refusal": null,
            }),
        );
    }

    #[test]
    fn test_should_not_add_refusal_to_user_messages() {
        assert_eq!(
            serde_json::to_value(ChatMessage::user("prompt")).expect("serialize user message"),
            json!({
                "role": "user",
                "content": "prompt",
            }),
        );
    }
}
