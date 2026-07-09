//! Runtime context shared by workflows.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use graphloom_cache::Cache;
use graphloom_input::InputReader;
use graphloom_llm::{CompletionModel, EmbeddingModel};
use graphloom_storage::{Storage, TableProvider};
use graphloom_vectors::VectorStore;
use serde_json::Value;

use crate::{NoopWorkflowCallbacks, PipelineRunStats, WorkflowCallbacks};

/// Pipeline runtime context.
#[derive(Debug)]
#[non_exhaustive]
pub struct PipelineRunContext {
    /// Current run statistics.
    pub stats: PipelineRunStats,
    /// Input document storage.
    pub input_storage: Option<Arc<dyn Storage>>,
    /// Output storage for snapshots and long-lived files.
    pub output_storage: Option<Arc<dyn Storage>>,
    /// Current output table provider.
    pub output_table_provider: Arc<dyn TableProvider>,
    /// Previous output table provider for update workflows.
    pub previous_table_provider: Option<Arc<dyn TableProvider>>,
    /// Cache provider for LLM and operation results.
    pub cache: Option<Arc<dyn Cache>>,
    /// Completion model instances keyed by model id.
    pub completion_models: BTreeMap<String, Arc<dyn CompletionModel>>,
    /// Embedding model instances keyed by model id.
    pub embedding_models: BTreeMap<String, Arc<dyn EmbeddingModel>>,
    /// Optional caller-provided vector store.
    pub vector_store: Option<Arc<dyn VectorStore>>,
    /// Workflow callbacks.
    pub callbacks: Arc<dyn WorkflowCallbacks>,
    /// Arbitrary run-local state.
    pub state: BTreeMap<String, Value>,
    /// Input reader used by `load_input_documents`.
    pub input_reader: Option<Arc<dyn InputReader>>,
    /// Project root used to resolve prompt paths.
    pub project_root: Option<PathBuf>,
}

impl PipelineRunContext {
    /// Create a context with the required table provider.
    #[must_use]
    pub fn new(output_table_provider: Arc<dyn TableProvider>) -> Self {
        Self {
            stats: PipelineRunStats::default(),
            input_storage: None,
            output_storage: None,
            output_table_provider,
            previous_table_provider: None,
            cache: None,
            completion_models: BTreeMap::new(),
            embedding_models: BTreeMap::new(),
            vector_store: None,
            callbacks: Arc::new(NoopWorkflowCallbacks),
            state: BTreeMap::new(),
            input_reader: None,
            project_root: None,
        }
    }

    /// Attach an input reader.
    #[must_use]
    pub fn with_input_reader(mut self, input_reader: Arc<dyn InputReader>) -> Self {
        self.input_reader = Some(input_reader);
        self
    }

    /// Attach callbacks.
    #[must_use]
    pub fn with_callbacks(mut self, callbacks: Arc<dyn WorkflowCallbacks>) -> Self {
        self.callbacks = callbacks;
        self
    }

    /// Attach a completion model by model id.
    #[must_use]
    pub fn with_completion_model(
        mut self,
        model_id: impl Into<String>,
        model: Arc<dyn CompletionModel>,
    ) -> Self {
        self.completion_models.insert(model_id.into(), model);
        self
    }

    /// Attach an embedding model by model id.
    #[must_use]
    pub fn with_embedding_model(
        mut self,
        model_id: impl Into<String>,
        model: Arc<dyn EmbeddingModel>,
    ) -> Self {
        self.embedding_models.insert(model_id.into(), model);
        self
    }

    /// Attach a custom vector store.
    #[must_use]
    pub fn with_vector_store(mut self, vector_store: Arc<dyn VectorStore>) -> Self {
        self.vector_store = Some(vector_store);
        self
    }

    /// Attach a project root for prompt path resolution.
    #[must_use]
    pub fn with_project_root(mut self, project_root: impl Into<PathBuf>) -> Self {
        self.project_root = Some(project_root.into());
        self
    }

    /// Return the prompt root, defaulting to current-directory semantics for library callers.
    #[must_use]
    pub fn prompt_root(&self) -> PathBuf {
        self.project_root
            .clone()
            .unwrap_or_else(|| PathBuf::from("."))
    }
}
