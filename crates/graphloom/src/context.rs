//! Runtime context for `GraphRAG` indexing workflows.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use graphloom_cache::Cache;
use graphloom_input::InputReader;
use graphloom_storage::{Storage, TableProvider};
use graphloom_vectors::VectorStore;

use crate::{IndexRunStats, IndexRuntimeServices, IndexWorkflowCallbacks, ModelRegistry};

/// Runtime context for `GraphLoom` indexing workflows.
///
/// This type is specific to the `GraphRAG` indexing pipeline. Query execution
/// uses a separate runtime and does not share this pipeline abstraction.
#[derive(Debug)]
#[non_exhaustive]
pub struct IndexPipelineContext {
    services: IndexRuntimeServices,
    update: Option<UpdateRuntimeState>,
    /// Current run statistics.
    pub stats: IndexRunStats,
    /// `IndexWorkflow` callbacks.
    pub callbacks: Arc<dyn IndexWorkflowCallbacks>,
}

impl IndexPipelineContext {
    /// Create a context from fully initialized indexing services.
    #[must_use]
    pub(crate) fn new(
        services: IndexRuntimeServices,
        callbacks: Arc<dyn IndexWorkflowCallbacks>,
    ) -> Self {
        Self {
            services,
            update: None,
            stats: IndexRunStats::default(),
            callbacks,
        }
    }

    pub(crate) fn with_update_state(mut self, update: UpdateRuntimeState) -> Self {
        self.update = Some(update);
        self
    }

    pub(crate) fn update_state(
        &self,
        workflow: &'static str,
    ) -> crate::Result<&UpdateRuntimeState> {
        self.update
            .as_ref()
            .ok_or(crate::GraphLoomError::MissingUpdateContext { workflow })
    }

    pub(crate) fn update_state_mut(
        &mut self,
        workflow: &'static str,
    ) -> crate::Result<&mut UpdateRuntimeState> {
        self.update
            .as_mut()
            .ok_or(crate::GraphLoomError::MissingUpdateContext { workflow })
    }

    /// Return the prepared input reader.
    #[must_use]
    pub fn input_reader(&self) -> Arc<dyn InputReader> {
        Arc::clone(&self.services.input_reader)
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

    pub(crate) fn replace_output_table_provider(
        &mut self,
        provider: Arc<dyn TableProvider>,
    ) -> Arc<dyn TableProvider> {
        std::mem::replace(&mut self.services.output_table_provider, provider)
    }

    /// Return the cache provider when caching is enabled.
    #[must_use]
    pub fn cache(&self) -> Option<&dyn Cache> {
        self.services.cache.provider().map(AsRef::as_ref)
    }

    /// Clone the prepared vector store handle.
    /// # Errors
    ///
    /// Returns an error when the active indexing pipeline did not request vector storage.
    pub fn vector_store(&self) -> crate::Result<Arc<dyn VectorStore>> {
        self.services.vector_store.provider()
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

/// Typed provider and mapping state for one incremental-update run.
#[derive(Debug, Clone)]
pub(crate) struct UpdateRuntimeState {
    pub(crate) timestamp: String,
    pub(crate) previous_table_provider: Arc<dyn TableProvider>,
    pub(crate) delta_table_provider: Arc<dyn TableProvider>,
    pub(crate) final_table_provider: Arc<dyn TableProvider>,
    pub(crate) entity_id_mapping: Option<BTreeMap<String, String>>,
    pub(crate) community_id_mapping: Option<BTreeMap<i64, i64>>,
}

impl UpdateRuntimeState {
    pub(crate) fn clear_temporary_state(&mut self) {
        self.entity_id_mapping = None;
        self.community_id_mapping = None;
    }
}

#[cfg(test)]
mod test_support {
    use std::{pin::Pin, sync::Arc};

    use futures_util::{Stream, stream};
    use graphloom_input::{DocumentStream, InputReader};
    use graphloom_llm::{CompletionModel, EmbeddingModel};
    use graphloom_storage::{MemoryStorage, TableProvider};
    use graphloom_vectors::VectorStore;

    use super::IndexPipelineContext;
    use crate::{
        ModelRegistry, NoopIndexWorkflowCallbacks,
        runtime::{CacheService, IndexRuntimeIo, IndexRuntimeServices, VectorStoreService},
    };

    #[derive(Debug, Default)]
    struct EmptyInputReader;

    impl InputReader for EmptyInputReader {
        fn read_documents(&self) -> DocumentStream<'_> {
            Box::pin(stream::empty()) as Pin<Box<dyn Stream<Item = _> + Send + '_>>
        }
    }

    impl IndexPipelineContext {
        pub(crate) fn for_test(output_table_provider: Arc<dyn TableProvider>) -> Self {
            let storage = Arc::new(MemoryStorage::new());
            let services = IndexRuntimeServices::new(
                IndexRuntimeIo::new(Arc::new(EmptyInputReader), storage, output_table_provider),
                CacheService::Disabled,
                VectorStoreService::Disabled,
                ModelRegistry::default(),
                ".",
            );
            Self::new(services, Arc::new(NoopIndexWorkflowCallbacks))
        }

        pub(crate) fn with_input_reader(mut self, input_reader: Arc<dyn InputReader>) -> Self {
            self.services.input_reader = input_reader;
            self
        }

        pub(crate) fn with_callbacks(
            mut self,
            callbacks: Arc<dyn crate::IndexWorkflowCallbacks>,
        ) -> Self {
            self.callbacks = callbacks;
            self
        }

        pub(crate) fn with_completion_model(
            mut self,
            id: impl Into<String>,
            model: Arc<dyn CompletionModel>,
        ) -> crate::Result<Self> {
            self.services.models.insert_completion(id, model)?;
            Ok(self)
        }

        pub(crate) fn with_cache(mut self, cache: Arc<dyn graphloom_cache::Cache>) -> Self {
            self.services.cache = CacheService::Enabled(cache);
            self
        }

        pub(crate) fn with_embedding_model(
            mut self,
            id: impl Into<String>,
            model: Arc<dyn EmbeddingModel>,
        ) -> crate::Result<Self> {
            self.services.models.insert_embedding(id, model)?;
            Ok(self)
        }

        pub(crate) fn with_vector_store(mut self, vector_store: Arc<dyn VectorStore>) -> Self {
            self.services.vector_store = VectorStoreService::Enabled(vector_store);
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
