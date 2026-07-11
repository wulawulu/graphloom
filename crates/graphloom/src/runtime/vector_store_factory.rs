//! Vector store construction for the indexing runtime lifecycle.

use std::sync::Arc;

use async_trait::async_trait;
use graphloom_vectors::{LanceDbVectorStore, VectorStore, VectorStoreConfig};

use crate::{GraphLoomError, Result};

#[async_trait]
pub(crate) trait IndexVectorStoreFactory: Send + Sync + std::fmt::Debug {
    async fn create(&self, config: &VectorStoreConfig) -> Result<Arc<dyn VectorStore>>;
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DefaultIndexVectorStoreFactory;

#[async_trait]
impl IndexVectorStoreFactory for DefaultIndexVectorStoreFactory {
    async fn create(&self, config: &VectorStoreConfig) -> Result<Arc<dyn VectorStore>> {
        Ok(Arc::new(
            LanceDbVectorStore::connect(config)
                .await
                .map_err(|source| GraphLoomError::RuntimeBuild {
                    source: Box::new(source),
                })?,
        ))
    }
}
