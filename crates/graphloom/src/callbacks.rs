//! Workflow callback hooks.

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
