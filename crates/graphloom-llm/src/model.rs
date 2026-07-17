//! Provider-neutral model request and response types.

use std::{fmt, pin::Pin};

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

use crate::{
    CompletionChunk, CompletionRequest, CompletionResponse, EmbeddingRequest, EmbeddingResponse,
    Result,
};

/// Provider-neutral completion stream.
pub type CompletionStream =
    Pin<Box<dyn futures_util::Stream<Item = Result<CompletionChunk>> + Send>>;

fn default_max_retries() -> u32 {
    1
}

fn default_provider_type() -> String {
    "openai".to_owned()
}

const DEFAULT_OPENAI_API_BASE: &str = "https://api.openai.com/v1";
const DEFAULT_DEEPSEEK_API_BASE: &str = "https://api.deepseek.com";
const DEFAULT_OLLAMA_API_BASE: &str = "http://localhost:11434/v1";
const GRAPHRAG_LITELLM_FALLBACK_ENCODING: &str = "cl100k_base";

fn default_auth_method() -> String {
    "api_key".to_owned()
}

fn default_retry_type() -> String {
    "exponential_backoff".to_owned()
}

#[allow(
    clippy::ref_option,
    reason = "serde serialize_with callbacks receive a reference to the declared Option field"
)]
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
    /// Base provider call arguments used by Query and indexing operations.
    #[serde(alias = "call_args", default)]
    pub call_args: std::collections::BTreeMap<String, Value>,
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
            .field("call_args_keys", &self.call_args.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ModelConfig {
    /// Return the effective provider type.
    #[must_use]
    pub fn provider_type(&self) -> &str {
        self.provider_type.as_str()
    }

    /// Return the OpenAI-compatible API base resolved from `GraphRAG` provider semantics.
    ///
    /// `DeepSeek` and `Ollama` expose OpenAI-compatible APIs but use provider-specific defaults.
    /// `Ollama`'s compatibility routes live below `/v1`, so a missing suffix is added here rather
    /// than requiring a Graphloom-only configuration change.
    #[must_use]
    pub fn effective_api_base(&self) -> String {
        match self.provider_type.to_ascii_lowercase().as_str() {
            "deepseek" => self
                .api_base
                .clone()
                .unwrap_or_else(|| DEFAULT_DEEPSEEK_API_BASE.to_owned()),
            "ollama" => self.api_base.as_deref().map_or_else(
                || DEFAULT_OLLAMA_API_BASE.to_owned(),
                normalize_ollama_api_base,
            ),
            _ => self
                .api_base
                .clone()
                .unwrap_or_else(|| DEFAULT_OPENAI_API_BASE.to_owned()),
        }
    }

    /// Return the tokenizer encoding used for GraphRAG-compatible model input accounting.
    ///
    /// `GraphRAG`'s `LiteLLM` tokenizer currently falls back to `cl100k_base` when a model has no
    /// registered Hugging Face tokenizer, including custom `DeepSeek` model names. An explicit
    /// `encoding_model` always takes precedence.
    #[must_use]
    pub fn effective_tokenizer_encoding(&self) -> &str {
        self.encoding_model
            .as_deref()
            .unwrap_or(GRAPHRAG_LITELLM_FALLBACK_ENCODING)
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
    /// Returns an error for an unsupported provider or missing API key.
    pub fn validate_openai_compatible(&self, model_instance: &str) -> Result<()> {
        if !matches!(
            self.provider_type.to_ascii_lowercase().as_str(),
            "openai" | "deepseek" | "ollama"
        ) {
            return Err(crate::LlmError::InvalidConfig {
                model_instance: model_instance.to_owned(),
                message: format!(
                    "unsupported provider {}; supported OpenAI-compatible providers are openai, \
                     deepseek, and ollama",
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

fn normalize_ollama_api_base(api_base: &str) -> String {
    let trimmed = api_base.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/v1")
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

    /// Stream a completion request.
    ///
    /// Custom models receive a correct single-chunk fallback. Provider adapters
    /// should override this method when their transport supports true streaming.
    ///
    /// # Errors
    ///
    /// Returns an error when request validation or completion fails.
    async fn stream(&self, mut request: CompletionRequest) -> Result<CompletionStream> {
        self.validate_request(&request)?;
        request.stream = Some(false);
        let response = self.complete(request).await?;
        Ok(Box::pin(futures_util::stream::once(async move {
            Ok(CompletionChunk::from_response(response))
        })))
    }
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
