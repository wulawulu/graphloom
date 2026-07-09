//! Workflow callback hooks.

use std::sync::Arc;

use crate::PipelineRunStats;

/// Callback hooks used by pipeline workflows.
pub trait WorkflowCallbacks: Send + Sync + std::fmt::Debug {
    /// Called when a workflow starts.
    fn workflow_started(&self, _workflow_name: &str) {}

    /// Called when a workflow completes successfully.
    fn workflow_completed(&self, _workflow_name: &str, _stats: &PipelineRunStats) {}

    /// Called for progress updates.
    fn progress(&self, _workflow_name: &str, _completed: usize, _total: Option<usize>) {}

    /// Called for non-fatal warnings.
    fn warning(&self, _workflow_name: &str, _message: &str) {}

    /// Called when a workflow fails.
    fn error(&self, _workflow_name: &str, _message: &str) {}

    /// Called when an LLM retry occurs.
    fn llm_retry(&self, _model_instance: &str, _attempt: u32) {}

    /// Called when LLM usage is recorded.
    fn llm_usage(&self, _model_instance: &str, _input_tokens: usize, _output_tokens: usize) {}
}

/// No-op callbacks implementation.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopWorkflowCallbacks;

impl WorkflowCallbacks for NoopWorkflowCallbacks {}

/// Callback fan-out implementation.
#[derive(Debug, Clone)]
pub struct CallbackChain {
    callbacks: Vec<Arc<dyn WorkflowCallbacks>>,
}

impl CallbackChain {
    /// Create a callback chain.
    #[must_use]
    pub fn new(callbacks: Vec<Arc<dyn WorkflowCallbacks>>) -> Self {
        Self { callbacks }
    }
}

impl WorkflowCallbacks for CallbackChain {
    fn workflow_started(&self, workflow_name: &str) {
        for callback in &self.callbacks {
            callback.workflow_started(workflow_name);
        }
    }

    fn workflow_completed(&self, workflow_name: &str, stats: &PipelineRunStats) {
        for callback in &self.callbacks {
            callback.workflow_completed(workflow_name, stats);
        }
    }

    fn progress(&self, workflow_name: &str, completed: usize, total: Option<usize>) {
        for callback in &self.callbacks {
            callback.progress(workflow_name, completed, total);
        }
    }

    fn warning(&self, workflow_name: &str, message: &str) {
        for callback in &self.callbacks {
            callback.warning(workflow_name, message);
        }
    }

    fn error(&self, workflow_name: &str, message: &str) {
        for callback in &self.callbacks {
            callback.error(workflow_name, message);
        }
    }

    fn llm_retry(&self, model_instance: &str, attempt: u32) {
        for callback in &self.callbacks {
            callback.llm_retry(model_instance, attempt);
        }
    }

    fn llm_usage(&self, model_instance: &str, input_tokens: usize, output_tokens: usize) {
        for callback in &self.callbacks {
            callback.llm_usage(model_instance, input_tokens, output_tokens);
        }
    }
}

pub(crate) fn callback_chain(
    callbacks: Vec<Arc<dyn WorkflowCallbacks>>,
) -> Arc<dyn WorkflowCallbacks> {
    if callbacks.is_empty() {
        Arc::new(NoopWorkflowCallbacks)
    } else if callbacks.len() == 1 {
        callbacks
            .into_iter()
            .next()
            .expect("checked callback length")
    } else {
        Arc::new(CallbackChain::new(callbacks))
    }
}
