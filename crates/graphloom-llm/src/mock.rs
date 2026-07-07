//! Deterministic mock completion and embedding models.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::{
    CompletionModel, CompletionRequest, CompletionResponse, EmbeddingModel, EmbeddingRequest,
    EmbeddingResponse, LlmError, Result,
};

/// Deterministic mock completion model.
#[derive(Debug, Clone)]
pub struct MockCompletionModel {
    model_instance: String,
    responses: Arc<Mutex<Vec<String>>>,
}

impl MockCompletionModel {
    /// Create a mock completion model.
    #[must_use]
    pub fn new(model_instance: impl Into<String>, responses: Vec<String>) -> Self {
        Self {
            model_instance: model_instance.into(),
            responses: Arc::new(Mutex::new(responses)),
        }
    }
}

#[async_trait]
impl CompletionModel for MockCompletionModel {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse> {
        let mut responses = self.responses.lock().await;
        if responses.is_empty() {
            return Err(LlmError::InvalidResponse {
                model_instance: self.model_instance.clone(),
                operation: "completion",
                message: "mock response queue is empty".to_owned(),
            });
        }

        Ok(CompletionResponse {
            content: responses.remove(0),
            usage: None,
            request_id: None,
        })
    }
}

/// Deterministic mock embedding model.
#[derive(Debug, Clone)]
pub struct MockEmbeddingModel {
    model_instance: String,
    embedding: Vec<f32>,
}

impl MockEmbeddingModel {
    /// Create a mock embedding model that repeats `embedding` for every input.
    #[must_use]
    pub fn new(model_instance: impl Into<String>, embedding: Vec<f32>) -> Self {
        Self {
            model_instance: model_instance.into(),
            embedding,
        }
    }
}

#[async_trait]
impl EmbeddingModel for MockEmbeddingModel {
    async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse> {
        if self.embedding.is_empty() {
            return Err(LlmError::InvalidResponse {
                model_instance: self.model_instance.clone(),
                operation: "embedding",
                message: "mock embedding must not be empty".to_owned(),
            });
        }

        Ok(EmbeddingResponse {
            embeddings: vec![self.embedding.clone(); request.input.len()],
            usage: None,
            request_id: None,
        })
    }
}
