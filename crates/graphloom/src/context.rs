//! Runtime context shared by workflows.

use std::{collections::BTreeMap, sync::Arc};

use graphloom_cache::Cache;
use graphloom_input::InputReader;
use graphloom_storage::{Storage, TableProvider};
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
    /// Workflow callbacks.
    pub callbacks: Arc<dyn WorkflowCallbacks>,
    /// Arbitrary run-local state.
    pub state: BTreeMap<String, Value>,
    /// Input reader used by `load_input_documents`.
    pub input_reader: Option<Arc<dyn InputReader>>,
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
            callbacks: Arc::new(NoopWorkflowCallbacks),
            state: BTreeMap::new(),
            input_reader: None,
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
}
