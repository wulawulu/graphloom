//! Deterministic mock completion and embedding models.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::{
    CompletionChunk, CompletionModel, CompletionRequest, CompletionResponse, CompletionStream,
    EmbeddingModel, EmbeddingRequest, EmbeddingResponse, LlmError, Result,
};

/// Deterministic mock completion model.
#[derive(Debug, Clone)]
pub struct MockCompletionModel {
    model_instance: String,
    responses: Arc<Mutex<Vec<String>>>,
    stream_responses: Option<Arc<Mutex<Vec<Vec<String>>>>>,
}

impl MockCompletionModel {
    /// Create a mock completion model.
    #[must_use]
    pub fn new(model_instance: impl Into<String>, responses: Vec<String>) -> Self {
        Self {
            model_instance: model_instance.into(),
            responses: Arc::new(Mutex::new(responses)),
            stream_responses: None,
        }
    }

    /// Create a mock whose stream calls emit the configured chunk sequences.
    #[must_use]
    pub fn with_streaming_chunks(
        model_instance: impl Into<String>,
        responses: Vec<Vec<String>>,
    ) -> Self {
        let model_instance = model_instance.into();
        let complete_responses = responses
            .iter()
            .map(|chunks| chunks.concat())
            .collect::<Vec<_>>();
        Self {
            model_instance,
            responses: Arc::new(Mutex::new(complete_responses)),
            stream_responses: Some(Arc::new(Mutex::new(responses))),
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

        Ok(CompletionResponse::text_for_test(
            self.model_instance.clone(),
            responses.remove(0),
        ))
    }

    async fn stream(&self, mut request: CompletionRequest) -> Result<CompletionStream> {
        self.validate_request(&request)?;
        let Some(responses) = &self.stream_responses else {
            request.stream = Some(false);
            let response = self.complete(request).await?;
            return Ok(Box::pin(futures_util::stream::once(async move {
                Ok(CompletionChunk::from_response(response))
            })));
        };
        let mut responses = responses.lock().await;
        if responses.is_empty() {
            return Err(LlmError::InvalidResponse {
                model_instance: self.model_instance.clone(),
                operation: "completion stream",
                message: "mock stream response queue is empty".to_owned(),
            });
        }
        let chunks = responses.remove(0);
        let last = chunks.len().saturating_sub(1);
        let model = self.model_instance.clone();
        Ok(Box::pin(futures_util::stream::iter(
            chunks.into_iter().enumerate().map(move |(index, content)| {
                Ok(CompletionChunk::text_for_test(
                    model.clone(),
                    content,
                    (index == last).then(|| "stop".to_owned()),
                ))
            }),
        )))
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

        Ok(EmbeddingResponse::vectors_for_test(
            self.model_instance.clone(),
            vec![self.embedding.clone(); request.input.len()],
        ))
    }
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;

    use super::*;
    use crate::ChatMessage;

    #[tokio::test]
    async fn test_should_use_single_chunk_fallback_for_non_streaming_mock() {
        let model = MockCompletionModel::new("mock", vec!["answer".to_owned()]);
        let mut stream = model
            .stream(CompletionRequest::new(vec![ChatMessage::user("question")]))
            .await
            .expect("stream");
        let chunk = stream.next().await.expect("chunk").expect("valid chunk");
        assert_eq!(
            chunk
                .choices
                .first()
                .and_then(|choice| choice.delta.content.as_deref()),
            Some("answer")
        );
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn test_should_emit_configured_mock_chunks() {
        let model = MockCompletionModel::with_streaming_chunks(
            "mock",
            vec![vec!["one".to_owned(), "two".to_owned()]],
        );
        let mut stream = model
            .stream(CompletionRequest::new(vec![ChatMessage::user("question")]))
            .await
            .expect("stream");
        let mut content = Vec::new();
        while let Some(chunk) = stream.next().await {
            content.push(
                chunk
                    .expect("chunk")
                    .choices
                    .first()
                    .and_then(|choice| choice.delta.content.clone())
                    .expect("content"),
            );
        }
        assert_eq!(content, vec!["one", "two"]);
    }
}
