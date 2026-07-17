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

    /// Apply provider call arguments to their canonical request fields.
    ///
    /// Unknown arguments are retained in [`Self::extra`]. The provider model
    /// name is deliberately ignored because model selection belongs to
    /// [`crate::ModelConfig`], while `stream` is parsed so orchestration can
    /// explicitly override it when streaming is mandatory.
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::InvalidRequest`] when a canonical argument has an
    /// incompatible JSON type or is outside the corresponding Rust integer
    /// range.
    pub fn apply_call_args(&mut self, call_args: &BTreeMap<String, Value>) -> Result<()> {
        for (field, value) in call_args {
            match field.as_str() {
                "temperature" => self.temperature = Some(finite_number(field, value)?),
                "top_p" => self.top_p = Some(finite_number(field, value)?),
                "max_tokens" => self.max_tokens = Some(unsigned_integer(field, value)?),
                "max_completion_tokens" => {
                    self.max_completion_tokens = Some(unsigned_integer(field, value)?);
                }
                "n" => self.n = Some(unsigned_integer(field, value)?),
                "seed" => self.seed = Some(signed_integer(field, value)?),
                "stop" => self.stop = Some(value.clone()),
                "tools" => {
                    self.tools = Some(
                        value
                            .as_array()
                            .cloned()
                            .ok_or_else(|| invalid_call_arg(field, "must be a JSON array"))?,
                    );
                }
                "tool_choice" => self.tool_choice = Some(value.clone()),
                "presence_penalty" => {
                    self.presence_penalty = Some(finite_number(field, value)?);
                }
                "frequency_penalty" => {
                    self.frequency_penalty = Some(finite_number(field, value)?);
                }
                "response_format" => self.response_format = Some(value.clone()),
                "stream" => {
                    self.stream = Some(
                        value
                            .as_bool()
                            .ok_or_else(|| invalid_call_arg(field, "must be a JSON boolean"))?,
                    );
                }
                "model" => {}
                _ if COMPLETION_RESERVED_FIELDS.contains(&field.as_str()) => {
                    return Err(invalid_call_arg(
                        field,
                        "is supplied by the completion orchestration",
                    ));
                }
                _ => {
                    self.extra.insert(field.clone(), value.clone());
                }
            }
        }
        Ok(())
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

fn finite_number(field: &str, value: &Value) -> Result<f64> {
    value
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| invalid_call_arg(field, "must be a finite JSON number"))
}

fn unsigned_integer(field: &str, value: &Value) -> Result<u32> {
    let raw = value
        .as_u64()
        .ok_or_else(|| invalid_call_arg(field, "must be a non-negative JSON integer"))?;
    u32::try_from(raw).map_err(|_| invalid_call_arg(field, "exceeds u32"))
}

fn signed_integer(field: &str, value: &Value) -> Result<i64> {
    value
        .as_i64()
        .ok_or_else(|| invalid_call_arg(field, "must be a JSON integer within i64"))
}

fn invalid_call_arg(field: &str, message: &str) -> LlmError {
    LlmError::InvalidRequest {
        operation: "apply completion call_args",
        message: format!("completion call_args field {field:?} {message}"),
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
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{ChatMessage, CompletionRequest};

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

    #[test]
    fn test_should_apply_canonical_completion_call_args_without_reserved_extras() {
        let mut request = CompletionRequest::new(vec![ChatMessage::user("query")]);
        request
            .apply_call_args(&BTreeMap::from([
                ("temperature".to_owned(), json!(0.2)),
                ("top_p".to_owned(), json!(0.8)),
                ("n".to_owned(), json!(1)),
                ("max_tokens".to_owned(), json!(100)),
                ("max_completion_tokens".to_owned(), json!(120)),
                ("seed".to_owned(), json!(42)),
                ("stop".to_owned(), json!(["END"])),
                (
                    "tools".to_owned(),
                    json!([{"type": "function", "function": {"name": "lookup"}}]),
                ),
                ("tool_choice".to_owned(), json!("auto")),
                ("presence_penalty".to_owned(), json!(0.1)),
                ("frequency_penalty".to_owned(), json!(0.2)),
                ("response_format".to_owned(), json!({"type": "json_object"})),
                ("stream".to_owned(), json!(false)),
                ("model".to_owned(), json!("configured-elsewhere")),
                ("parallel_tool_calls".to_owned(), json!(false)),
            ]))
            .expect("valid call_args");

        assert_eq!(request.temperature, Some(0.2));
        assert_eq!(request.top_p, Some(0.8));
        assert_eq!(request.n, Some(1));
        assert_eq!(request.max_tokens, Some(100));
        assert_eq!(request.max_completion_tokens, Some(120));
        assert_eq!(request.seed, Some(42));
        assert_eq!(request.stop, Some(json!(["END"])));
        assert_eq!(
            request.tools,
            Some(vec![
                json!({"type": "function", "function": {"name": "lookup"}})
            ])
        );
        assert_eq!(request.tool_choice, Some(json!("auto")));
        assert_eq!(request.presence_penalty, Some(0.1));
        assert_eq!(request.frequency_penalty, Some(0.2));
        assert_eq!(
            request.response_format,
            Some(json!({"type": "json_object"}))
        );
        assert_eq!(request.stream, Some(false));
        assert_eq!(
            request.extra,
            BTreeMap::from([("parallel_tool_calls".to_owned(), json!(false))])
        );
        request.validate().expect("request should validate");
    }

    #[test]
    fn test_should_reject_invalid_canonical_completion_call_args() {
        for (field, value, expected) in [
            ("seed", json!(1.5), "within i64"),
            ("seed", json!(u64::MAX), "within i64"),
            ("max_tokens", json!(-1), "non-negative"),
            ("max_tokens", json!(u64::MAX), "exceeds u32"),
            ("tools", json!({"type": "function"}), "JSON array"),
            ("temperature", json!("0.2"), "finite JSON number"),
            ("presence_penalty", json!("NaN"), "finite JSON number"),
        ] {
            let mut request = CompletionRequest::new(vec![ChatMessage::user("query")]);
            let error = request
                .apply_call_args(&BTreeMap::from([(field.to_owned(), value)]))
                .expect_err("invalid call_arg must fail");
            let message = error.to_string();
            assert!(message.contains(field), "{message}");
            assert!(message.contains(expected), "{message}");
        }
    }
}
