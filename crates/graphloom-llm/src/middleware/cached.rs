//! GraphRAG-compatible completion and embedding cache middleware.

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use bytes::Bytes;
use graphloom_cache::Cache;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    CacheStatus, CompletionModel, CompletionRequest, CompletionResponse, EmbeddingModel,
    EmbeddingRequest, EmbeddingResponse, Result, completion_request_cache_key,
    embedding_request_cache_key,
};

/// GraphRAG-compatible per-call metrics payload.
pub type CacheMetrics = BTreeMap<String, Value>;

/// Shared cache payload stored inside GraphRAG's outer `result` envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedModelResult<R> {
    /// Full canonical provider response.
    pub response: R,
    /// Metrics captured for the provider call.
    #[serde(default)]
    pub metrics: CacheMetrics,
}

/// Cached completion model using GraphRAG v4 keys and payloads.
#[derive(Debug, Clone)]
pub struct CachedCompletionModel {
    inner: Arc<dyn CompletionModel>,
    cache: Arc<dyn Cache>,
}

impl CachedCompletionModel {
    /// Wrap a raw completion provider with a namespaced cache.
    #[must_use]
    pub fn new(inner: Arc<dyn CompletionModel>, cache: Arc<dyn Cache>) -> Self {
        Self { inner, cache }
    }
}

#[async_trait]
impl CompletionModel for CachedCompletionModel {
    fn validate_request(&self, request: &CompletionRequest) -> Result<()> {
        self.inner.validate_request(request)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        self.validate_request(&request)?;
        if request.stream == Some(true) || has_truthy_extra(&request.extra, "mock_response") {
            return self.inner.complete(request).await;
        }
        let key = completion_request_cache_key(&request)?;
        if let Some(mut cached) = read_cached::<CompletionResponse>(&*self.cache, &key).await? {
            tracing::debug!(key, "completion cache hit");
            cached.response.metadata.cache_status = CacheStatus::Hit;
            return Ok(cached.response);
        }

        tracing::debug!(key, "completion cache miss");
        let mut response = self.inner.complete(request).await?;
        response.metadata.cache_status = CacheStatus::Miss;
        write_cached(&*self.cache, &key, &response).await?;
        Ok(response)
    }
}

/// Cached embedding model using GraphRAG v4 keys and payloads.
#[derive(Debug, Clone)]
pub struct CachedEmbeddingModel {
    inner: Arc<dyn EmbeddingModel>,
    cache: Arc<dyn Cache>,
}

impl CachedEmbeddingModel {
    /// Wrap a raw embedding provider with a namespaced cache.
    #[must_use]
    pub fn new(inner: Arc<dyn EmbeddingModel>, cache: Arc<dyn Cache>) -> Self {
        Self { inner, cache }
    }
}

#[async_trait]
impl EmbeddingModel for CachedEmbeddingModel {
    fn validate_request(&self, request: &EmbeddingRequest) -> Result<()> {
        self.inner.validate_request(request)
    }

    async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse> {
        self.validate_request(&request)?;
        if has_truthy_extra(&request.extra, "mock_response") {
            return self.inner.embed(request).await;
        }
        let key = embedding_request_cache_key(&request)?;
        if let Some(mut cached) = read_cached::<EmbeddingResponse>(&*self.cache, &key).await? {
            tracing::debug!(key, "embedding cache hit");
            cached.response.metadata.cache_status = CacheStatus::Hit;
            return Ok(cached.response);
        }

        tracing::debug!(key, "embedding cache miss");
        let mut response = self.inner.embed(request).await?;
        response.metadata.cache_status = CacheStatus::Miss;
        write_cached(&*self.cache, &key, &response).await?;
        Ok(response)
    }
}

async fn read_cached<R>(cache: &dyn Cache, key: &str) -> Result<Option<CachedModelResult<R>>>
where
    R: DeserializeOwned,
{
    let Some(bytes) = cache.get(key).await? else {
        return Ok(None);
    };
    match serde_json::from_slice(&bytes) {
        Ok(cached) => Ok(Some(cached)),
        Err(source) => {
            tracing::debug!(key, error = %source, "invalid model cache entry; deleting and retrying provider");
            cache.delete(key).await?;
            Ok(None)
        }
    }
}

async fn write_cached<R>(cache: &dyn Cache, key: &str, response: &R) -> Result<()>
where
    R: Serialize,
{
    let value = CachedModelResult {
        response,
        metrics: CacheMetrics::new(),
    };
    let bytes = serde_json::to_vec(&value)
        .map(Bytes::from)
        .map_err(|source| graphloom_cache::CacheError::Json {
            key: key.to_owned(),
            source,
        })?;
    cache.set(key, bytes).await?;
    Ok(())
}

fn has_truthy_extra(extra: &BTreeMap<String, Value>, key: &str) -> bool {
    extra.get(key).is_some_and(is_truthy)
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::is_truthy;

    #[test]
    fn test_should_match_python_json_truthiness() {
        for value in [
            json!(null),
            json!(false),
            json!(0),
            json!(0.0),
            json!(""),
            json!([]),
            json!({}),
        ] {
            assert!(!is_truthy(&value), "expected false for {value}");
        }
        for value in [
            json!(true),
            json!(1),
            json!(-0.1),
            json!("mock"),
            json!([0]),
            json!({"value": false}),
        ] {
            assert!(is_truthy(&value), "expected true for {value}");
        }
    }
}
