//! Canonical embedding response wire types.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ModelCallMetadata;

/// Embedding token usage.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingUsage {
    /// Input tokens.
    pub prompt_tokens: u64,
    /// Total tokens.
    pub total_tokens: u64,
    /// Provider usage extensions.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// One embedding vector in provider order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingData {
    /// Wire object kind.
    pub object: String,
    /// Input index.
    pub index: usize,
    /// Embedding vector.
    pub embedding: Vec<f64>,
    /// Unknown provider fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Provider-neutral embedding response compatible with GraphRAG cache JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingResponse {
    /// Wire object kind.
    pub object: String,
    /// Embedding vectors.
    pub data: Vec<EmbeddingData>,
    /// Provider model name.
    pub model: String,
    /// Usage counters.
    pub usage: EmbeddingUsage,
    /// Unknown and computed GraphRAG/LiteLLM fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
    /// Local metadata excluded from cache JSON.
    #[serde(skip)]
    pub metadata: ModelCallMetadata,
}

impl EmbeddingResponse {
    /// Iterate over embedding vectors without cloning.
    pub fn embeddings(&self) -> impl ExactSizeIterator<Item = &[f64]> {
        self.data.iter().map(|item| item.embedding.as_slice())
    }

    /// Consume the response and return embedding vectors in provider order.
    #[must_use]
    pub fn into_embeddings(self) -> Vec<Vec<f32>> {
        self.data
            .into_iter()
            .map(|item| {
                item.embedding
                    .into_iter()
                    .map(|value| value as f32)
                    .collect()
            })
            .collect()
    }

    /// Construct a canonical response for mock providers.
    #[must_use]
    pub fn vectors_for_test(model: impl Into<String>, vectors: Vec<Vec<f32>>) -> Self {
        Self {
            object: "list".to_owned(),
            data: vectors
                .into_iter()
                .enumerate()
                .map(|(index, embedding)| EmbeddingData {
                    object: "embedding".to_owned(),
                    index,
                    embedding: embedding.into_iter().map(f64::from).collect(),
                    extra: BTreeMap::new(),
                })
                .collect(),
            model: model.into(),
            usage: EmbeddingUsage::default(),
            extra: BTreeMap::new(),
            metadata: ModelCallMetadata::default(),
        }
    }
}
