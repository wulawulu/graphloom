//! Runtime context shared by workflows.

use std::{path::Path, sync::Arc};

use graphloom_cache::Cache;
use graphloom_input::InputReader;
use graphloom_storage::{Storage, TableProvider};
use graphloom_vectors::VectorStore;

use crate::{IndexRuntimeServices, ModelRegistry, PipelineRunStats, WorkflowCallbacks};

/// Pipeline runtime state backed by a complete set of indexing services.
#[derive(Debug)]
#[non_exhaustive]
pub struct PipelineRunContext {
    services: IndexRuntimeServices,
    /// Current run statistics.
    pub stats: PipelineRunStats,
    /// Workflow callbacks.
    pub callbacks: Arc<dyn WorkflowCallbacks>,
}

impl PipelineRunContext {
    /// Create a context from fully initialized indexing services.
    #[must_use]
    pub fn new(services: IndexRuntimeServices, callbacks: Arc<dyn WorkflowCallbacks>) -> Self {
        Self {
            services,
            stats: PipelineRunStats::default(),
            callbacks,
        }
    }

    /// Return the prepared input reader.
    #[must_use]
    pub fn input_reader(&self) -> Arc<dyn InputReader> {
        Arc::clone(&self.services.input_reader)
    }

    /// Return the prepared input storage.
    #[must_use]
    pub fn input_storage(&self) -> &dyn Storage {
        self.services.input_storage.as_ref()
    }

    /// Return the prepared output storage.
    #[must_use]
    pub fn output_storage(&self) -> &dyn Storage {
        self.services.output_storage.as_ref()
    }

    /// Return the prepared output table provider.
    #[must_use]
    pub fn output_table_provider(&self) -> &dyn TableProvider {
        self.services.output_table_provider.as_ref()
    }

    /// Return the cache provider when caching is enabled.
    #[must_use]
    pub fn cache(&self) -> Option<&dyn Cache> {
        self.services.cache.provider().map(AsRef::as_ref)
    }

    /// Clone the prepared vector store handle.
    #[must_use]
    pub fn vector_store(&self) -> Arc<dyn VectorStore> {
        Arc::clone(&self.services.vector_store)
    }

    /// Return the prepared model registry.
    #[must_use]
    pub fn models(&self) -> &ModelRegistry {
        &self.services.models
    }

    /// Return the project root used to resolve prompts.
    #[must_use]
    pub fn project_root(&self) -> &Path {
        self.services.project_root()
    }

    /// Return an owned prompt root path.
    #[must_use]
    pub fn prompt_root(&self) -> std::path::PathBuf {
        self.project_root().to_path_buf()
    }
}

#[cfg(test)]
mod test_support {
    use std::{pin::Pin, sync::Arc};

    use async_trait::async_trait;
    use futures_util::{Stream, stream};
    use graphloom_input::{DocumentStream, InputReader};
    use graphloom_llm::{CompletionModel, EmbeddingModel};
    use graphloom_storage::{MemoryStorage, TableProvider};
    use graphloom_vectors::{
        Result as VectorResult, VectorDocument, VectorIndexSchema, VectorStore,
    };

    use super::PipelineRunContext;
    use crate::{
        CacheService, IndexRuntimeIo, IndexRuntimeServices, ModelRegistry, NoopWorkflowCallbacks,
    };

    #[derive(Debug, Default)]
    struct EmptyInputReader;

    impl InputReader for EmptyInputReader {
        fn read_documents(&self) -> DocumentStream<'_> {
            Box::pin(stream::empty()) as Pin<Box<dyn Stream<Item = _> + Send + '_>>
        }
    }

    #[derive(Debug, Default)]
    struct EmptyVectorStore;

    #[async_trait]
    impl VectorStore for EmptyVectorStore {
        async fn ensure_index(&self, _schema: &VectorIndexSchema) -> VectorResult<()> {
            Ok(())
        }

        async fn reset_index(&self, _schema: &VectorIndexSchema) -> VectorResult<()> {
            Ok(())
        }

        async fn upsert_documents(
            &self,
            _schema: &VectorIndexSchema,
            _documents: &[VectorDocument],
        ) -> VectorResult<()> {
            Ok(())
        }

        async fn count(&self, _schema: &VectorIndexSchema) -> VectorResult<usize> {
            Ok(0)
        }

        async fn ids(&self, _schema: &VectorIndexSchema) -> VectorResult<Vec<String>> {
            Ok(Vec::new())
        }

        async fn get_by_id(
            &self,
            _schema: &VectorIndexSchema,
            _id: &str,
        ) -> VectorResult<Option<VectorDocument>> {
            Ok(None)
        }
    }

    impl PipelineRunContext {
        pub(crate) fn for_test(output_table_provider: Arc<dyn TableProvider>) -> Self {
            let storage = Arc::new(MemoryStorage::new());
            let services = IndexRuntimeServices::new(
                IndexRuntimeIo::new(
                    Arc::new(EmptyInputReader),
                    storage.clone(),
                    storage,
                    output_table_provider,
                ),
                CacheService::Disabled,
                Arc::new(EmptyVectorStore),
                ModelRegistry::default(),
                ".",
            );
            Self::new(services, Arc::new(NoopWorkflowCallbacks))
        }

        pub(crate) fn with_input_reader(mut self, input_reader: Arc<dyn InputReader>) -> Self {
            self.services.input_reader = input_reader;
            self
        }

        pub(crate) fn with_callbacks(
            mut self,
            callbacks: Arc<dyn crate::WorkflowCallbacks>,
        ) -> Self {
            self.callbacks = callbacks;
            self
        }

        pub(crate) fn with_completion_model(
            mut self,
            id: impl Into<String>,
            model: Arc<dyn CompletionModel>,
        ) -> Self {
            let _result = self.services.models.insert_completion(id, model);
            self
        }

        pub(crate) fn with_embedding_model(
            mut self,
            id: impl Into<String>,
            model: Arc<dyn EmbeddingModel>,
        ) -> Self {
            let _result = self.services.models.insert_embedding(id, model);
            self
        }

        pub(crate) fn with_vector_store(mut self, vector_store: Arc<dyn VectorStore>) -> Self {
            self.services.vector_store = vector_store;
            self
        }

        pub(crate) fn with_output_storage(
            mut self,
            output_storage: Arc<dyn graphloom_storage::Storage>,
        ) -> Self {
            self.services.output_storage = output_storage;
            self
        }
    }
}
