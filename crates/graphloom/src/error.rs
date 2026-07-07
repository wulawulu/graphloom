//! Error types for the top-level pipeline.

use thiserror::Error;

/// Result type used by the top-level `graphloom` crate.
pub type Result<T> = std::result::Result<T, GraphLoomError>;

/// Errors raised by pipeline configuration and workflow orchestration.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum GraphLoomError {
    /// Storage operation failed.
    #[error(transparent)]
    Storage(#[from] graphloom_storage::StorageError),

    /// Input operation failed.
    #[error(transparent)]
    Input(#[from] graphloom_input::InputError),

    /// Chunking operation failed.
    #[error(transparent)]
    Chunking(#[from] graphloom_chunking::ChunkingError),

    /// LLM/tokenizer operation failed.
    #[error(transparent)]
    Llm(#[from] graphloom_llm::LlmError),

    /// DataFrame operation failed.
    #[error(transparent)]
    Polars(#[from] polars_core::error::PolarsError),

    /// JSON serialization failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// A workflow name is not registered.
    #[error("workflow {name} is not registered")]
    UnknownWorkflow {
        /// Workflow name.
        name: String,
    },

    /// A workflow failed.
    #[error("workflow {name} failed: {source}")]
    WorkflowFailed {
        /// Workflow name.
        name: String,
        /// Underlying failure.
        #[source]
        source: Box<GraphLoomError>,
    },

    /// A required provider is missing.
    #[error("missing provider: {name}")]
    MissingProvider {
        /// Provider name.
        name: &'static str,
    },

    /// A workflow encountered invalid data.
    #[error("invalid data in workflow {workflow}: {message}")]
    InvalidData {
        /// Workflow name.
        workflow: &'static str,
        /// Failure details.
        message: String,
    },

    /// Numeric conversion failed.
    #[error("numeric conversion failed in workflow {workflow}: {message}")]
    NumericConversion {
        /// Workflow name.
        workflow: &'static str,
        /// Failure details.
        message: String,
    },
}
