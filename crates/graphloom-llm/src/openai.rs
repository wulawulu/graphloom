//! OpenAI-compatible model adapters backed by `async-openai`.

use std::time::Duration;

use async_openai::{
    Client,
    config::OpenAIConfig,
    error::OpenAIError,
    middleware::{ReqwestService, retry::OpenAIRetryLayer},
    types::{
        chat::{
            ChatCompletionRequestAssistantMessage, ChatCompletionRequestAssistantMessageContent,
            ChatCompletionRequestDeveloperMessage, ChatCompletionRequestDeveloperMessageContent,
            ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
            ChatCompletionRequestSystemMessageContent, ChatCompletionRequestUserMessage,
            ChatCompletionRequestUserMessageContent, CompletionUsage, CreateChatCompletionRequest,
            CreateChatCompletionRequestArgs, CreateChatCompletionResponse, ResponseFormat,
        },
        embeddings::{
            CreateEmbeddingRequest, CreateEmbeddingRequestArgs, CreateEmbeddingResponse,
            EmbeddingInput, EmbeddingUsage,
        },
    },
};
use async_trait::async_trait;
use tower::ServiceBuilder;

use crate::{
    ChatMessage, ChatRole, CompletionModel, CompletionRequest, CompletionResponse, EmbeddingModel,
    EmbeddingRequest, EmbeddingResponse, LlmError, ModelConfig, Result, Usage,
};

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
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        let request = OpenAiCompletionRequest {
            model: self.config.model.clone(),
            request,
        }
        .try_into()
        .map_err(|source| {
            provider_error(
                &self.model_instance,
                "completion",
                self.config.effective_max_retries(),
                source,
            )
        })?;
        let response = self.client.chat().create(request).await.map_err(|source| {
            provider_error(
                &self.model_instance,
                "completion",
                self.config.effective_max_retries(),
                source,
            )
        })?;
        OpenAiCompletionResponse {
            model_instance: self.model_instance.clone(),
            response,
        }
        .try_into()
    }
}

#[async_trait]
impl EmbeddingModel for OpenAiEmbeddingModel {
    async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse> {
        let request = OpenAiEmbeddingRequest {
            model: self.config.model.clone(),
            request,
        }
        .try_into()
        .map_err(|source| {
            provider_error(
                &self.model_instance,
                "embedding",
                self.config.effective_max_retries(),
                source,
            )
        })?;
        let response = self
            .client
            .embeddings()
            .create(request)
            .await
            .map_err(|source| {
                provider_error(
                    &self.model_instance,
                    "embedding",
                    self.config.effective_max_retries(),
                    source,
                )
            })?;
        Ok(OpenAiEmbeddingResponse { response }.into())
    }
}

#[derive(Debug)]
struct OpenAiCompletionRequest {
    model: String,
    request: CompletionRequest,
}

impl TryFrom<OpenAiCompletionRequest> for CreateChatCompletionRequest {
    type Error = OpenAIError;

    fn try_from(value: OpenAiCompletionRequest) -> std::result::Result<Self, Self::Error> {
        let mut builder = CreateChatCompletionRequestArgs::default();
        builder
            .model(value.model)
            .messages(
                value
                    .request
                    .messages
                    .into_iter()
                    .map(ChatCompletionRequestMessage::from)
                    .collect::<Vec<_>>(),
            )
            .stream(false);
        if let Some(temperature) = value.request.temperature {
            builder.temperature(temperature);
        }
        if let Some(top_p) = value.request.top_p {
            builder.top_p(top_p);
        }
        if let Some(max_tokens) = value.request.max_tokens {
            #[allow(deprecated)]
            builder.max_tokens(max_tokens);
        }
        if let Some(response_format) = value.request.response_format {
            builder.response_format(ResponseFormat::try_from(OpenAiResponseFormat(
                response_format,
            ))?);
        }
        builder.build()
    }
}

impl From<ChatMessage> for ChatCompletionRequestMessage {
    fn from(value: ChatMessage) -> Self {
        match value.role {
            ChatRole::System => ChatCompletionRequestSystemMessage {
                content: ChatCompletionRequestSystemMessageContent::Text(value.content),
                name: None,
            }
            .into(),
            ChatRole::User => ChatCompletionRequestUserMessage {
                content: ChatCompletionRequestUserMessageContent::Text(value.content),
                name: None,
            }
            .into(),
            ChatRole::Assistant => ChatCompletionRequestAssistantMessage {
                content: Some(ChatCompletionRequestAssistantMessageContent::Text(
                    value.content,
                )),
                refusal: None,
                name: None,
                audio: None,
                tool_calls: None,
                #[allow(deprecated)]
                function_call: None,
            }
            .into(),
            ChatRole::Developer => ChatCompletionRequestDeveloperMessage {
                content: ChatCompletionRequestDeveloperMessageContent::Text(value.content),
                name: None,
            }
            .into(),
        }
    }
}

#[derive(Debug)]
struct OpenAiResponseFormat(String);

impl TryFrom<OpenAiResponseFormat> for ResponseFormat {
    type Error = OpenAIError;

    fn try_from(value: OpenAiResponseFormat) -> std::result::Result<Self, Self::Error> {
        match value.0.as_str() {
            "json_object" => Ok(Self::JsonObject),
            "text" => Ok(Self::Text),
            _ => Err(OpenAIError::InvalidArgument(format!(
                "unsupported response_format: {}",
                value.0,
            ))),
        }
    }
}

#[derive(Debug)]
struct OpenAiEmbeddingRequest {
    model: String,
    request: EmbeddingRequest,
}

impl TryFrom<OpenAiEmbeddingRequest> for CreateEmbeddingRequest {
    type Error = OpenAIError;

    fn try_from(value: OpenAiEmbeddingRequest) -> std::result::Result<Self, Self::Error> {
        let mut builder = CreateEmbeddingRequestArgs::default();
        builder
            .model(value.model)
            .input(EmbeddingInput::StringArray(value.request.input));
        if let Some(dimensions) = value.request.dimensions {
            builder.dimensions(dimensions);
        }
        builder.build()
    }
}

#[derive(Debug)]
struct OpenAiCompletionResponse {
    model_instance: String,
    response: CreateChatCompletionResponse,
}

impl TryFrom<OpenAiCompletionResponse> for CompletionResponse {
    type Error = LlmError;

    fn try_from(value: OpenAiCompletionResponse) -> Result<Self> {
        let content = value
            .response
            .choices
            .first()
            .and_then(|choice| choice.message.content.clone())
            .ok_or_else(|| LlmError::InvalidResponse {
                model_instance: value.model_instance.clone(),
                operation: "completion",
                message: "missing choices[0].message.content".to_owned(),
            })?;
        if content.is_empty() {
            return Err(LlmError::InvalidResponse {
                model_instance: value.model_instance,
                operation: "completion",
                message: "missing choices[0].message.content".to_owned(),
            });
        }

        Ok(Self {
            content,
            usage: value.response.usage.map(Usage::from),
            request_id: Some(value.response.id),
        })
    }
}

#[derive(Debug)]
struct OpenAiEmbeddingResponse {
    response: CreateEmbeddingResponse,
}

impl From<OpenAiEmbeddingResponse> for EmbeddingResponse {
    fn from(value: OpenAiEmbeddingResponse) -> Self {
        Self {
            embeddings: value
                .response
                .data
                .into_iter()
                .map(|embedding| embedding.embedding)
                .collect(),
            usage: Some(Usage::from(value.response.usage)),
            request_id: None,
        }
    }
}

impl From<CompletionUsage> for Usage {
    fn from(value: CompletionUsage) -> Self {
        Self {
            prompt_tokens: value.prompt_tokens,
            completion_tokens: value.completion_tokens,
            total_tokens: value.total_tokens,
        }
    }
}

impl From<EmbeddingUsage> for Usage {
    fn from(value: EmbeddingUsage) -> Self {
        Self {
            prompt_tokens: value.prompt_tokens,
            completion_tokens: 0,
            total_tokens: value.total_tokens,
        }
    }
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
        let mut openai =
            OpenAIConfig::new().with_api_key(config.api_key.clone().unwrap_or_default());
        if let Some(api_base) = &config.api_base {
            openai = openai.with_api_base(api_base);
        }
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
    use async_openai::types::{
        chat::{
            ChatChoice, ChatCompletionRequestMessage, ChatCompletionResponseMessage,
            CompletionUsage, CreateChatCompletionResponse, Role,
        },
        embeddings::{CreateEmbeddingRequest, EmbeddingInput},
    };

    use super::{OpenAiCompletionRequest, OpenAiCompletionResponse, OpenAiEmbeddingRequest};
    use crate::{ChatMessage, CompletionRequest, EmbeddingRequest};

    #[test]
    fn test_should_convert_completion_request_to_openai_type() {
        let request: async_openai::types::chat::CreateChatCompletionRequest =
            OpenAiCompletionRequest {
                model: "gpt-test".to_owned(),
                request: CompletionRequest {
                    messages: vec![ChatMessage::user("hello"), ChatMessage::assistant("world")],
                    temperature: Some(0.2),
                    top_p: Some(0.9),
                    max_tokens: Some(32),
                    response_format: Some("json_object".to_owned()),
                    cache_namespace: None,
                },
            }
            .try_into()
            .expect("completion request should convert");

        assert_eq!(request.model, "gpt-test");
        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.temperature, Some(0.2));
        assert_eq!(request.top_p, Some(0.9));
        #[allow(deprecated)]
        {
            assert_eq!(request.max_tokens, Some(32));
        }
        assert!(matches!(
            request.messages.first(),
            Some(ChatCompletionRequestMessage::User(_))
        ));
    }

    #[test]
    fn test_should_convert_embedding_request_to_openai_type() {
        let request: CreateEmbeddingRequest = OpenAiEmbeddingRequest {
            model: "embed-test".to_owned(),
            request: EmbeddingRequest {
                input: vec!["a".to_owned(), "b".to_owned()],
                dimensions: Some(8),
                cache_namespace: None,
            },
        }
        .try_into()
        .expect("embedding request should convert");

        assert_eq!(request.model, "embed-test");
        assert_eq!(request.dimensions, Some(8));
        assert_eq!(
            request.input,
            EmbeddingInput::StringArray(vec!["a".to_owned(), "b".to_owned()])
        );
    }

    #[test]
    fn test_should_convert_openai_completion_response_to_domain_type() {
        let response: crate::CompletionResponse = OpenAiCompletionResponse {
            model_instance: "chat".to_owned(),
            response: CreateChatCompletionResponse {
                id: "request-id".to_owned(),
                choices: vec![ChatChoice {
                    index: 0,
                    message: ChatCompletionResponseMessage {
                        content: Some("answer".to_owned()),
                        refusal: None,
                        tool_calls: None,
                        annotations: None,
                        role: Role::Assistant,
                        #[allow(deprecated)]
                        function_call: None,
                        audio: None,
                    },
                    finish_reason: None,
                    logprobs: None,
                }],
                created: 0,
                model: "gpt-test".to_owned(),
                service_tier: None,
                #[allow(deprecated)]
                system_fingerprint: None,
                object: "chat.completion".to_owned(),
                usage: Some(CompletionUsage {
                    prompt_tokens: 2,
                    completion_tokens: 3,
                    total_tokens: 5,
                    prompt_tokens_details: None,
                    completion_tokens_details: None,
                }),
            },
        }
        .try_into()
        .expect("completion response should convert");

        assert_eq!(response.content, "answer");
        assert_eq!(response.request_id.as_deref(), Some("request-id"));
        assert_eq!(response.usage.expect("usage should exist").total_tokens, 5);
    }
}
