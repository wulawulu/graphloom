//! OpenAI-compatible model adapters backed by `async-openai`.

use std::{sync::Arc, time::Duration};

use async_openai::{Client, config::OpenAIConfig};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::{
    sync::Semaphore,
    time::{sleep, timeout},
};

use crate::{
    ChatRole, CompletionModel, CompletionRequest, CompletionResponse, EmbeddingModel,
    EmbeddingRequest, EmbeddingResponse, LlmError, ModelConfig, Result, Usage,
};

/// OpenAI-compatible completion model.
#[derive(Debug, Clone)]
pub struct OpenAiCompletionModel {
    model_instance: String,
    config: ModelConfig,
    client: Client<OpenAIConfig>,
    semaphore: Arc<Semaphore>,
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
        let client = Client::with_config(openai_config(&config));
        Ok(Self {
            model_instance,
            config,
            client,
            semaphore: Arc::new(Semaphore::new(concurrent_requests.max(1))),
        })
    }
}

/// OpenAI-compatible embedding model.
#[derive(Debug, Clone)]
pub struct OpenAiEmbeddingModel {
    model_instance: String,
    config: ModelConfig,
    client: Client<OpenAIConfig>,
    semaphore: Arc<Semaphore>,
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
        let client = Client::with_config(openai_config(&config));
        Ok(Self {
            model_instance,
            config,
            client,
            semaphore: Arc::new(Semaphore::new(concurrent_requests.max(1))),
        })
    }
}

#[async_trait]
impl CompletionModel for OpenAiCompletionModel {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        let _permit =
            self.semaphore
                .acquire()
                .await
                .map_err(|source| LlmError::InvalidResponse {
                    model_instance: self.model_instance.clone(),
                    operation: "completion",
                    message: source.to_string(),
                })?;
        let payload = completion_payload(&self.config, &request);
        let value = retry_provider_call(&self.model_instance, "completion", &self.config, || {
            let client = self.client.clone();
            let payload = payload.clone();
            async move { client.chat().create_byot::<Value, Value>(payload).await }
        })
        .await?;
        parse_completion_response(&self.model_instance, value)
    }
}

#[async_trait]
impl EmbeddingModel for OpenAiEmbeddingModel {
    async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse> {
        let _permit =
            self.semaphore
                .acquire()
                .await
                .map_err(|source| LlmError::InvalidResponse {
                    model_instance: self.model_instance.clone(),
                    operation: "embedding",
                    message: source.to_string(),
                })?;
        let payload = embedding_payload(&self.config, &request);
        let value = retry_provider_call(&self.model_instance, "embedding", &self.config, || {
            let client = self.client.clone();
            let payload = payload.clone();
            async move {
                client
                    .embeddings()
                    .create_byot::<Value, Value>(payload)
                    .await
            }
        })
        .await?;
        parse_embedding_response(&self.model_instance, value)
    }
}

fn openai_config(config: &ModelConfig) -> OpenAIConfig {
    let mut openai = OpenAIConfig::new().with_api_key(config.api_key.clone().unwrap_or_default());
    if let Some(api_base) = &config.api_base {
        openai = openai.with_api_base(api_base);
    }
    if let Some(organization) = &config.organization {
        openai = openai.with_org_id(organization);
    }
    openai
}

fn completion_payload(config: &ModelConfig, request: &CompletionRequest) -> Value {
    let mut payload = json!({
        "model": config.model,
        "messages": request.messages.iter().map(message_json).collect::<Vec<_>>(),
    });
    insert_optional(&mut payload, "temperature", request.temperature);
    insert_optional(&mut payload, "top_p", request.top_p);
    insert_optional(&mut payload, "max_tokens", request.max_tokens);
    if let Some(response_format) = &request.response_format {
        payload["response_format"] = json!({ "type": response_format });
    }
    payload
}

fn embedding_payload(config: &ModelConfig, request: &EmbeddingRequest) -> Value {
    let mut payload = json!({
        "model": config.model,
        "input": request.input,
    });
    insert_optional(&mut payload, "dimensions", request.dimensions);
    payload
}

fn message_json(message: &crate::ChatMessage) -> Value {
    let role = match message.role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
        ChatRole::Developer => "developer",
    };
    json!({ "role": role, "content": message.content })
}

fn insert_optional<T>(payload: &mut Value, key: &str, value: Option<T>)
where
    T: serde::Serialize,
{
    if let Some(value) = value {
        payload[key] = json!(value);
    }
}

async fn retry_provider_call<F, Fut>(
    model_instance: &str,
    operation: &'static str,
    config: &ModelConfig,
    mut call: F,
) -> Result<Value>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = std::result::Result<Value, async_openai::error::OpenAIError>>,
{
    let max_attempts = config.max_retries.unwrap_or(1).max(1);
    let mut attempt = 1;
    loop {
        let result = if let Some(seconds) = config.timeout {
            match timeout(Duration::from_secs(seconds), call()).await {
                Ok(result) => result,
                Err(_) if attempt < max_attempts => {
                    sleep(retry_delay(config, attempt)).await;
                    attempt += 1;
                    continue;
                }
                Err(_) => {
                    return Err(LlmError::Timeout {
                        model_instance: model_instance.to_owned(),
                        operation,
                        attempts: attempt,
                    });
                }
            }
        } else {
            call().await
        };

        match result {
            Ok(value) => return Ok(value),
            Err(source) if should_retry(&source) && attempt < max_attempts => {
                sleep(retry_delay(config, attempt)).await;
                attempt += 1;
            }
            Err(source) => {
                return Err(LlmError::Provider {
                    model_instance: model_instance.to_owned(),
                    operation,
                    attempts: attempt,
                    request_id: None,
                    source: Box::new(source),
                });
            }
        }
    }
}

fn should_retry(error: &async_openai::error::OpenAIError) -> bool {
    match error {
        async_openai::error::OpenAIError::Reqwest(_)
        | async_openai::error::OpenAIError::StreamError(_) => true,
        async_openai::error::OpenAIError::ApiError(response) => {
            let status = response.status_code.as_u16();
            status == 429 || (500..=599).contains(&status)
        }
        _ => false,
    }
}

fn retry_delay(config: &ModelConfig, attempt: u32) -> Duration {
    if config.retry_strategy.as_deref() == Some("immediate") {
        return Duration::from_millis(0);
    }
    let shift = attempt.saturating_sub(1).min(10);
    let base = 100_u64.saturating_mul(1_u64 << shift);
    let jitter = u64::from(attempt).saturating_mul(37) % 50;
    Duration::from_millis(base.saturating_add(jitter))
}

fn parse_completion_response(model_instance: &str, value: Value) -> Result<CompletionResponse> {
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Ok(CompletionResponse {
        content,
        usage: parse_usage(value.get("usage")),
        request_id: value
            .get("id")
            .and_then(Value::as_str)
            .map(std::borrow::ToOwned::to_owned),
    })
    .and_then(|response| {
        if response.content.is_empty() {
            Err(LlmError::InvalidResponse {
                model_instance: model_instance.to_owned(),
                operation: "completion",
                message: "missing choices[0].message.content".to_owned(),
            })
        } else {
            Ok(response)
        }
    })
}

fn parse_embedding_response(model_instance: &str, value: Value) -> Result<EmbeddingResponse> {
    let data =
        value
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| LlmError::InvalidResponse {
                model_instance: model_instance.to_owned(),
                operation: "embedding",
                message: "missing data array".to_owned(),
            })?;
    let mut embeddings = Vec::with_capacity(data.len());
    for item in data {
        let values = item
            .get("embedding")
            .and_then(Value::as_array)
            .ok_or_else(|| LlmError::InvalidResponse {
                model_instance: model_instance.to_owned(),
                operation: "embedding",
                message: "missing embedding vector".to_owned(),
            })?;
        let mut embedding = Vec::with_capacity(values.len());
        for value in values {
            let number = value.as_f64().ok_or_else(|| LlmError::InvalidResponse {
                model_instance: model_instance.to_owned(),
                operation: "embedding",
                message: "embedding value is not numeric".to_owned(),
            })?;
            embedding.push(number as f32);
        }
        embeddings.push(embedding);
    }
    Ok(EmbeddingResponse {
        embeddings,
        usage: parse_usage(value.get("usage")),
        request_id: value
            .get("id")
            .and_then(Value::as_str)
            .map(std::borrow::ToOwned::to_owned),
    })
}

fn parse_usage(value: Option<&Value>) -> Option<Usage> {
    let usage = value?;
    Some(Usage {
        prompt_tokens: usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .and_then(|value| value.try_into().ok())
            .unwrap_or_default(),
        completion_tokens: usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .and_then(|value| value.try_into().ok())
            .unwrap_or_default(),
        total_tokens: usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .and_then(|value| value.try_into().ok())
            .unwrap_or_default(),
    })
}
