//! OpenAI-compatible model adapters backed by `async-openai`.

use std::time::Duration;

use async_openai::{
    Client,
    config::OpenAIConfig,
    error::OpenAIError,
    middleware::{ReqwestService, retry::OpenAIRetryLayer},
};
use async_trait::async_trait;
use secrecy::ExposeSecret;
use serde_json::Value;
use tower::ServiceBuilder;

use crate::{
    CompletionModel, CompletionRequest, CompletionResponse, EmbeddingModel, EmbeddingRequest,
    EmbeddingResponse, LlmError, ModelConfig, Result,
};

const OPENAI_CLIENT_ONLY_FIELDS: &[&str] = &[
    "mock_response",
    "timeout",
    "base_url",
    "api_base",
    "api_version",
    "api_key",
    "azure_ad_token_provider",
    "drop_params",
    "stream_options",
    "metrics",
];

/// OpenAI-compatible completion model.
#[derive(Debug, Clone)]
pub struct OpenAiCompletionModel {
    model_instance: String,
    config: ModelConfig,
    client: Client<OpenAIConfig>,
}

impl OpenAiCompletionModel {
    /// Create an OpenAI-compatible completion model.
    ///
    /// # Errors
    ///
    /// Returns an error when the model configuration is invalid.
    pub fn new(
        model_instance: impl Into<String>,
        config: ModelConfig,
        concurrent_requests: usize,
    ) -> Result<Self> {
        let model_instance = model_instance.into();
        config.validate_openai_compatible(&model_instance)?;
        let client = openai_client(&config, concurrent_requests);
        Ok(Self {
            model_instance,
            config,
            client,
        })
    }
}

/// OpenAI-compatible embedding model.
#[derive(Debug, Clone)]
pub struct OpenAiEmbeddingModel {
    model_instance: String,
    config: ModelConfig,
    client: Client<OpenAIConfig>,
}

impl OpenAiEmbeddingModel {
    /// Create an OpenAI-compatible embedding model.
    ///
    /// # Errors
    ///
    /// Returns an error when the model configuration is invalid.
    pub fn new(
        model_instance: impl Into<String>,
        config: ModelConfig,
        concurrent_requests: usize,
    ) -> Result<Self> {
        let model_instance = model_instance.into();
        config.validate_openai_compatible(&model_instance)?;
        let client = openai_client(&config, concurrent_requests);
        Ok(Self {
            model_instance,
            config,
            client,
        })
    }
}

#[async_trait]
impl CompletionModel for OpenAiCompletionModel {
    fn validate_request(&self, request: &CompletionRequest) -> Result<()> {
        validate_openai_completion_request(request)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        let request: Value = OpenAiCompletionRequest {
            model: self.config.model.clone(),
            request,
        }
        .try_into()?;
        let response: CompletionResponse =
            self.client
                .chat()
                .create_byot(request)
                .await
                .map_err(|source| {
                    provider_error(
                        &self.model_instance,
                        "completion",
                        self.config.effective_max_retries(),
                        source,
                    )
                })?;
        response.content().map_err(|source| match source {
            LlmError::InvalidResponse { message, .. } => LlmError::InvalidResponse {
                model_instance: self.model_instance.clone(),
                operation: "completion conversion",
                message,
            },
            source => source,
        })?;
        Ok(response)
    }
}

#[async_trait]
impl EmbeddingModel for OpenAiEmbeddingModel {
    fn validate_request(&self, request: &EmbeddingRequest) -> Result<()> {
        validate_openai_embedding_request(request)
    }

    async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse> {
        let request: Value = OpenAiEmbeddingRequest {
            model: self.config.model.clone(),
            request,
        }
        .try_into()?;
        self.client
            .embeddings()
            .create_byot(request)
            .await
            .map_err(|source| {
                provider_error(
                    &self.model_instance,
                    "embedding",
                    self.config.effective_max_retries(),
                    source,
                )
            })
    }
}

#[derive(Debug)]
struct OpenAiCompletionRequest {
    model: String,
    request: CompletionRequest,
}

impl TryFrom<OpenAiCompletionRequest> for Value {
    type Error = LlmError;

    fn try_from(value: OpenAiCompletionRequest) -> std::result::Result<Self, Self::Error> {
        validate_openai_completion_request(&value.request)?;
        let mut payload = serde_json::to_value(value.request).map_err(provider_request_error)?;
        let object = payload
            .as_object_mut()
            .ok_or_else(|| LlmError::InvalidRequest {
                operation: "build OpenAI completion request",
                message: "completion request must serialize to an object".to_owned(),
            })?;
        object.insert("model".to_owned(), Value::String(value.model));
        object
            .entry("stream".to_owned())
            .or_insert(Value::Bool(false));
        Ok(payload)
    }
}

#[derive(Debug)]
struct OpenAiEmbeddingRequest {
    model: String,
    request: EmbeddingRequest,
}

impl TryFrom<OpenAiEmbeddingRequest> for Value {
    type Error = LlmError;

    fn try_from(value: OpenAiEmbeddingRequest) -> std::result::Result<Self, Self::Error> {
        validate_openai_embedding_request(&value.request)?;
        let mut payload = serde_json::to_value(value.request).map_err(provider_request_error)?;
        let object = payload
            .as_object_mut()
            .ok_or_else(|| LlmError::InvalidRequest {
                operation: "build OpenAI embedding request",
                message: "embedding request must serialize to an object".to_owned(),
            })?;
        object.insert("model".to_owned(), Value::String(value.model));
        Ok(payload)
    }
}

fn provider_request_error(source: serde_json::Error) -> LlmError {
    LlmError::InvalidRequest {
        operation: "build OpenAI provider request",
        message: source.to_string(),
    }
}

fn validate_provider_body_extra(
    extra: &std::collections::BTreeMap<String, Value>,
    operation: &'static str,
) -> Result<()> {
    if let Some(field) = extra
        .keys()
        .find(|field| OPENAI_CLIENT_ONLY_FIELDS.contains(&field.as_str()))
    {
        return Err(LlmError::InvalidRequest {
            operation,
            message: format!("client-only field {field:?} cannot be sent in the provider body"),
        });
    }
    Ok(())
}

fn validate_openai_completion_request(request: &CompletionRequest) -> Result<()> {
    request.validate()?;
    validate_provider_body_extra(&request.extra, "validate OpenAI completion request")
}

fn validate_openai_embedding_request(request: &EmbeddingRequest) -> Result<()> {
    request.validate()?;
    validate_provider_body_extra(&request.extra, "validate OpenAI embedding request")
}

fn openai_client(config: &ModelConfig, concurrent_requests: usize) -> Client<OpenAIConfig> {
    let client = Client::with_config(OpenAiModelConfig(config).into());
    let retry_layer = OpenAIRetryLayer::new(
        usize::try_from(config.effective_max_retries().saturating_sub(1)).unwrap_or(usize::MAX),
    );
    let concurrent_requests = concurrent_requests.max(1);
    let transport = ReqwestService::default();

    if let Some(seconds) = config.timeout {
        let service = ServiceBuilder::new()
            .concurrency_limit(concurrent_requests)
            .timeout(Duration::from_secs(seconds))
            .layer(retry_layer)
            .service(transport);
        return client.with_http_service(service);
    }

    let service = ServiceBuilder::new()
        .concurrency_limit(concurrent_requests)
        .layer(retry_layer)
        .service(transport);
    client.with_http_service(service)
}

#[derive(Debug, Clone, Copy)]
struct OpenAiModelConfig<'a>(&'a ModelConfig);

impl From<OpenAiModelConfig<'_>> for OpenAIConfig {
    fn from(value: OpenAiModelConfig<'_>) -> Self {
        let config = value.0;
        let api_key = config
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret)
            .unwrap_or_default()
            .to_owned();
        let mut openai = OpenAIConfig::new().with_api_key(api_key);
        openai = openai.with_api_base(config.effective_api_base());
        if let Some(organization) = &config.organization {
            openai = openai.with_org_id(organization);
        }
        openai
    }
}

fn provider_error(
    model_instance: &str,
    operation: &'static str,
    attempts: u32,
    source: OpenAIError,
) -> LlmError {
    if is_tower_timeout(&source) {
        return LlmError::Timeout {
            model_instance: model_instance.to_owned(),
            operation,
            attempts,
        };
    }

    LlmError::Provider {
        model_instance: model_instance.to_owned(),
        operation,
        attempts,
        request_id: None,
        source: Box::new(source),
    }
}

fn is_tower_timeout(error: &OpenAIError) -> bool {
    match error {
        OpenAIError::Boxed(source) => source.is::<tower::timeout::error::Elapsed>(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use async_openai::config::{Config, OpenAIConfig};
    use secrecy::ExposeSecret;
    use serde_json::Value;

    use super::{OpenAiCompletionRequest, OpenAiEmbeddingRequest, OpenAiModelConfig};
    use crate::{ChatMessage, CompletionRequest, EmbeddingRequest, ModelConfig};

    #[test]
    fn test_should_convert_completion_request_to_openai_type() {
        let request: Value = OpenAiCompletionRequest {
            model: "gpt-test".to_owned(),
            request: CompletionRequest {
                temperature: Some(0.2),
                top_p: Some(0.9),
                max_tokens: Some(32),
                response_format: Some(serde_json::json!({"type": "json_object"})),
                ..CompletionRequest::new(vec![
                    ChatMessage::user("hello"),
                    ChatMessage::assistant("world"),
                ])
            },
        }
        .try_into()
        .expect("completion request should convert");

        assert_eq!(request["model"], "gpt-test");
        assert_eq!(request["messages"].as_array().map(Vec::len), Some(2));
        assert_eq!(request["temperature"], 0.2);
        assert_eq!(request["top_p"], 0.9);
        assert_eq!(request["max_tokens"], 32);
        assert_eq!(request["stream"], false);
    }

    #[test]
    fn test_should_convert_embedding_request_to_openai_type() {
        let request: Value = OpenAiEmbeddingRequest {
            model: "embed-test".to_owned(),
            request: EmbeddingRequest {
                dimensions: Some(8),
                ..EmbeddingRequest::new(vec!["a".to_owned(), "b".to_owned()])
            },
        }
        .try_into()
        .expect("embedding request should convert");

        assert_eq!(request["model"], "embed-test");
        assert_eq!(request["dimensions"], 8);
        assert_eq!(request["input"], serde_json::json!(["a", "b"]));
    }

    #[test]
    fn test_should_expose_api_key_only_at_provider_config_boundary() {
        let model: ModelConfig = serde_json::from_value(serde_json::json!({
            "model_provider": "openai",
            "model": "gpt-test",
            "api_key": "provider-secret"
        }))
        .expect("model config");

        let provider: OpenAIConfig = OpenAiModelConfig(&model).into();

        assert_eq!(provider.api_key().expose_secret(), "provider-secret");
    }

    #[test]
    fn test_should_apply_provider_specific_api_base_to_openai_transport() {
        let model: ModelConfig = serde_json::from_value(serde_json::json!({
            "model_provider": "ollama",
            "model": "bge-m3",
            "api_key": "ollama",
            "api_base": "http://localhost:11434"
        }))
        .expect("model config");

        let provider: OpenAIConfig = OpenAiModelConfig(&model).into();

        assert_eq!(provider.api_base(), "http://localhost:11434/v1");
    }
}
