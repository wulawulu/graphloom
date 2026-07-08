//! Vector store trait and factory.

use std::{fmt::Debug, sync::Arc};

use async_trait::async_trait;

use crate::{LanceDbVectorStore, Result, VectorIndexSchema, VectorStoreConfig, VectorStoreType};

/// Stored vector document.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorDocument {
    /// Source row id.
    pub id: String,
    /// Embedding vector.
    pub vector: Vec<f32>,
}

/// Vector search result.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorSearchResult {
    /// Matched document.
    pub document: VectorDocument,
    /// Provider score.
    pub score: f32,
}

/// Vector store contract.
#[async_trait]
pub trait VectorStore: Send + Sync + Debug {
    /// Ensure the index/table exists and matches `schema`.
    ///
    /// # Errors
    ///
    /// Returns an error when the provider cannot create or validate the index.
    async fn ensure_index(&self, schema: &VectorIndexSchema) -> Result<()>;

    /// Upsert vector documents into an index.
    ///
    /// # Errors
    ///
    /// Returns an error when validation or provider write fails.
    async fn upsert_documents(
        &self,
        schema: &VectorIndexSchema,
        documents: &[VectorDocument],
    ) -> Result<()>;

    /// Count documents in an index.
    ///
    /// # Errors
    ///
    /// Returns an error when provider read fails.
    async fn count(&self, schema: &VectorIndexSchema) -> Result<usize>;

    /// Fetch one document by id.
    ///
    /// # Errors
    ///
    /// Returns an error when provider read or decoding fails.
    async fn get_by_id(
        &self,
        schema: &VectorIndexSchema,
        id: &str,
    ) -> Result<Option<VectorDocument>>;
}

/// Create the configured vector store.
///
/// # Errors
///
/// Returns an error when configuration is invalid or the provider cannot connect.
pub async fn create_vector_store(config: &VectorStoreConfig) -> Result<Arc<dyn VectorStore>> {
    config.validate()?;
    match config.store_type {
        VectorStoreType::LanceDb => Ok(Arc::new(LanceDbVectorStore::connect(config).await?)),
    }
}
