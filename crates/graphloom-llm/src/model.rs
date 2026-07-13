//! Provider-neutral model request and response types.

use std::fmt;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize, Serializer};

use crate::{CompletionRequest, CompletionResponse, EmbeddingRequest, EmbeddingResponse, Result};

fn default_max_retries() -> u32 {
    1
}

fn default_provider_type() -> String {
    "openai".to_owned()
}

fn default_auth_method() -> String {
    "api_key".to_owned()
}

fn default_retry_type() -> String {
    "exponential_backoff".to_owned()
}

fn serialize_optional_secret<S>(
    value: &Option<SecretString>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(_) => serializer.serialize_some("<redacted>"),
        None => serializer.serialize_none(),
    }
}

/// Retry configuration nested under `GraphRAG` 3.1 model settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RetryConfig {
    /// Retry strategy type.
    #[serde(rename = "type", alias = "retry_type")]
    pub retry_type: String,
    /// Maximum retry attempts.
    #[serde(alias = "max_retries", skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            retry_type: default_retry_type(),
            max_retries: None,
        }
    }
}

/// OpenAI-compatible model configuration.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    /// Provider type, e.g. `openai`, `mock`, or `azure`.
    #[serde(
        rename = "model_provider",
        alias = "type",
        default = "default_provider_type"
    )]
    pub provider_type: String,
    /// Provider model name.
    pub model: String,
    /// Authentication method.
    #[serde(alias = "auth_method", default = "default_auth_method")]
    pub auth_method: String,
    /// API key for OpenAI-compatible providers.
    #[serde(
        alias = "api_key",
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_optional_secret"
    )]
    pub api_key: Option<SecretString>,
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
    /// Maximum retry attempts.
    #[serde(alias = "max_retries")]
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Retry strategy name. Supported values are `exponential_backoff` and `immediate`.
    #[serde(alias = "retry_strategy")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_strategy: Option<String>,
    /// `GraphRAG` 3.1 nested retry configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryConfig>,
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

impl fmt::Debug for ModelConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ModelConfig")
            .field("provider_type", &self.provider_type)
            .field("model", &self.model)
            .field("auth_method", &self.auth_method)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("api_base", &self.api_base)
            .field("organization", &self.organization)
            .field("timeout", &self.timeout)
            .field("max_retries", &self.max_retries)
            .field("retry_strategy", &self.retry_strategy)
            .field("retry", &self.retry)
            .field("tokens_per_minute", &self.tokens_per_minute)
            .field("requests_per_minute", &self.requests_per_minute)
            .field("encoding_model", &self.encoding_model)
            .finish()
    }
}

impl ModelConfig {
    /// Return the effective provider type.
    #[must_use]
    pub fn provider_type(&self) -> &str {
        self.provider_type.as_str()
    }

    /// Return the effective retry strategy.
    #[must_use]
    pub fn effective_retry_strategy(&self) -> &str {
        self.retry
            .as_ref()
            .map(|retry| retry.retry_type.as_str())
            .or(self.retry_strategy.as_deref())
            .unwrap_or("exponential_backoff")
    }

    /// Return the effective maximum retry count.
    #[must_use]
    pub fn effective_max_retries(&self) -> u32 {
        self.retry
            .as_ref()
            .and_then(|retry| retry.max_retries)
            .unwrap_or(self.max_retries)
    }

    /// Validate the configuration for a concrete model instance.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported Azure configuration or missing API key.
    pub fn validate_openai_compatible(&self, model_instance: &str) -> Result<()> {
        if !self.provider_type.eq_ignore_ascii_case("openai") {
            return Err(crate::LlmError::InvalidConfig {
                model_instance: model_instance.to_owned(),
                message: format!(
                    "unsupported provider {}; only openai is supported",
                    self.provider_type
                ),
            });
        }

        if !self.auth_method.eq_ignore_ascii_case("api_key") {
            return Err(crate::LlmError::InvalidConfig {
                model_instance: model_instance.to_owned(),
                message: format!(
                    "unsupported auth method {}; only api_key is supported",
                    self.auth_method
                ),
            });
        }

        let api_key = self
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret)
            .unwrap_or_default()
            .trim();
        if api_key.is_empty() || api_key == "<API_KEY>" {
            return Err(crate::LlmError::InvalidConfig {
                model_instance: model_instance.to_owned(),
                message: "api_key is required".to_owned(),
            });
        }

        if self.effective_max_retries() == 0 {
            return Err(crate::LlmError::InvalidConfig {
                model_instance: model_instance.to_owned(),
                message: "max_retries must be greater than zero".to_owned(),
            });
        }

        if !self
            .effective_retry_strategy()
            .eq_ignore_ascii_case("exponential_backoff")
        {
            return Err(crate::LlmError::InvalidConfig {
                model_instance: model_instance.to_owned(),
                message: format!(
                    "unsupported retry strategy {}; only exponential_backoff is supported",
                    self.effective_retry_strategy()
                ),
            });
        }

        Ok(())
    }
}

/// Provider-neutral completion model.
#[async_trait]
pub trait CompletionModel: Send + Sync + std::fmt::Debug {
    /// Validate a completion request without performing I/O.
    ///
    /// # Errors
    ///
    /// Returns an error when canonical request invariants are violated.
    fn validate_request(&self, request: &CompletionRequest) -> Result<()> {
        request.validate()
    }

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
    /// Validate an embedding request without performing I/O.
    ///
    /// # Errors
    ///
    /// Returns an error when canonical request invariants are violated.
    fn validate_request(&self, request: &EmbeddingRequest) -> Result<()> {
        request.validate()
    }

    /// Execute an embedding request.
    ///
    /// # Errors
    ///
    /// Returns an error when the provider fails or returns malformed output.
    async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse>;
}
