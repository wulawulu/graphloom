//! Provider-neutral model request and response types.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::Result;

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

/// Chat message used by completion models.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    /// Message role.
    pub role: ChatRole,
    /// Text content.
    pub content: String,
}

impl ChatMessage {
    /// Create a user message.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }

    /// Create an assistant message.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
        }
    }
}

/// OpenAI-compatible model configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    /// Provider type, e.g. `openai`, `mock`, or `azure`.
    #[serde(rename = "type")]
    pub provider_type: String,
    /// Provider model name.
    pub model: String,
    /// API key for OpenAI-compatible providers.
    #[serde(alias = "api_key")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// API base URL for OpenAI-compatible providers.
    #[serde(alias = "api_base")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
    /// Organization id for OpenAI-compatible providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    /// Per-request timeout in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// Maximum retry attempts. The initial request counts as attempt one.
    #[serde(alias = "max_retries")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    /// Retry strategy name. Supported values are `exponential_backoff` and `immediate`.
    #[serde(alias = "retry_strategy")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_strategy: Option<String>,
    /// Token rate limit carried for config compatibility.
    #[serde(alias = "tokens_per_minute")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_per_minute: Option<u32>,
    /// Request rate limit carried for config compatibility.
    #[serde(alias = "requests_per_minute")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requests_per_minute: Option<u32>,
    /// Tokenizer encoding model.
    #[serde(alias = "encoding_model")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding_model: Option<String>,
}

impl ModelConfig {
    /// Validate the configuration for a concrete model instance.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported Azure configuration or missing API key.
    pub fn validate_openai_compatible(&self, model_instance: &str) -> Result<()> {
        if self.provider_type.eq_ignore_ascii_case("azure") {
            return Err(crate::LlmError::InvalidConfig {
                model_instance: model_instance.to_owned(),
                message: "Azure OpenAI is not supported in Phase 1".to_owned(),
            });
        }

        if self.api_key.as_deref().unwrap_or_default().is_empty() {
            return Err(crate::LlmError::InvalidConfig {
                model_instance: model_instance.to_owned(),
                message: "api_key is required".to_owned(),
            });
        }

        Ok(())
    }
}

/// Completion request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionRequest {
    /// Chat messages.
    pub messages: Vec<ChatMessage>,
    /// Sampling temperature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Nucleus sampling threshold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// Maximum generated tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Response format, e.g. `json_object`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<String>,
    /// Business cache namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_namespace: Option<String>,
}

/// Embedding request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingRequest {
    /// Input texts.
    pub input: Vec<String>,
    /// Optional embedding dimensions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<u32>,
    /// Business cache namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_namespace: Option<String>,
}

/// Provider usage counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// Prompt/input tokens.
    pub prompt_tokens: u32,
    /// Completion/output tokens.
    pub completion_tokens: u32,
    /// Total tokens.
    pub total_tokens: u32,
}

/// Completion response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionResponse {
    /// First choice content, matching GraphRAG's `.content` convenience field.
    pub content: String,
    /// Usage counters when the provider returns them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Provider request id when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// Embedding response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingResponse {
    /// Embeddings in provider order.
    pub embeddings: Vec<Vec<f32>>,
    /// Usage counters when the provider returns them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Provider request id when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// Provider-neutral completion model.
#[async_trait]
pub trait CompletionModel: Send + Sync + std::fmt::Debug {
    /// Execute a completion request.
    ///
    /// # Errors
    ///
    /// Returns an error when the provider fails or returns malformed output.
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse>;
}

/// Provider-neutral embedding model.
#[async_trait]
pub trait EmbeddingModel: Send + Sync + std::fmt::Debug {
    /// Execute an embedding request.
    ///
    /// # Errors
    ///
    /// Returns an error when the provider fails or returns malformed output.
    async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse>;
}
