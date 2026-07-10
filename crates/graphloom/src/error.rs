//! Error types for the top-level pipeline, project loading, and public API.

use std::{io, path::PathBuf};

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

    /// Cache operation failed.
    #[error(transparent)]
    Cache(#[from] graphloom_cache::CacheError),

    /// LLM/tokenizer operation failed.
    #[error(transparent)]
    Llm(#[from] graphloom_llm::LlmError),

    /// Vector store operation failed.
    #[error(transparent)]
    Vector(#[from] graphloom_vectors::VectorError),

    /// `DataFrame` operation failed.
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

    /// Invalid root path.
    #[error("invalid project root {path}: {message}")]
    InvalidRoot {
        /// Root path.
        path: PathBuf,
        /// Failure message.
        message: String,
    },

    /// Project already initialized.
    #[error("project {root} is already initialized; use --force to overwrite managed files")]
    AlreadyInitialized {
        /// Project root.
        root: PathBuf,
    },

    /// Missing settings file.
    #[error("no settings.[yaml|yml|json] found under {root}")]
    MissingSettings {
        /// Project root.
        root: PathBuf,
    },

    /// Unsupported config format.
    #[error("unsupported config format for {path}; supported formats are yaml, yml, json")]
    UnsupportedConfigFormat {
        /// Config path.
        path: PathBuf,
    },

    /// Malformed dotenv line.
    #[error(".env line {line} is malformed")]
    DotenvParse {
        /// Line number.
        line: usize,
    },

    /// Missing environment variable.
    #[error("environment variable {name} is not defined")]
    MissingEnvironmentVariable {
        /// Variable name.
        name: String,
    },

    /// Configuration parse failed.
    #[error("failed to parse {path}: {source}")]
    ConfigParse {
        /// Config path.
        path: PathBuf,
        /// Underlying parser error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Unsupported storage.
    #[error("unsupported {kind} storage {storage_type}; only file is implemented")]
    UnsupportedStorage {
        /// Storage kind.
        kind: &'static str,
        /// Storage type.
        storage_type: String,
    },

    /// Unsupported input type.
    #[error("unsupported input type {input_type}; only text and file are implemented")]
    UnsupportedInput {
        /// Input type.
        input_type: String,
    },

    /// Unsafe destructive output path.
    #[error("unsafe output path {path}: {message}")]
    UnsafeOutputPath {
        /// Output path.
        path: PathBuf,
        /// Reason.
        message: String,
    },

    /// Missing prompt file.
    #[error("missing prompt file {path}")]
    MissingPrompt {
        /// Prompt path.
        path: PathBuf,
    },

    /// Prompt template loading failed.
    #[error("failed to load prompt template {path}: {source}")]
    PromptLoad {
        /// Template path.
        path: PathBuf,
        /// Original I/O error.
        #[source]
        source: io::Error,
    },

    /// Prompt template rendering or syntax validation failed.
    #[error("failed to render {kind} prompt template {name} from {prompt_source}: {message}")]
    PromptRender {
        /// Prompt kind.
        kind: &'static str,
        /// Canonical prompt filename.
        name: &'static str,
        /// Built-in or filesystem template source.
        prompt_source: String,
        /// Rendering or validation failure.
        message: String,
    },

    /// Missing input.
    #[error("missing input: {message}")]
    MissingInput {
        /// Message.
        message: String,
    },

    /// Invalid model configuration.
    #[error("invalid model {model_id}: {message}")]
    InvalidModel {
        /// Model id.
        model_id: String,
        /// Message.
        message: String,
    },

    /// Runtime build failed.
    #[error("failed to build indexing runtime: {source}")]
    RuntimeBuild {
        /// Source error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Indexing failed.
    #[error("index failed: {source}")]
    IndexFailed {
        /// Source error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A transaction failed and its recovery also failed.
    #[error("{operation} failed: {source}; rollback also failed: {rollback}")]
    RollbackFailed {
        /// Transaction being recovered.
        operation: &'static str,
        /// Primary transaction failure.
        #[source]
        source: Box<GraphLoomError>,
        /// Recovery failure.
        rollback: Box<GraphLoomError>,
    },

    /// I/O failed.
    #[error("{operation} failed for {path}: {source}")]
    Io {
        /// Operation.
        operation: &'static str,
        /// Path.
        path: PathBuf,
        /// Source.
        #[source]
        source: io::Error,
    },
}

/// Redact sensitive key/value pairs in a JSON value.
pub fn redact_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                if is_secret_key(key) {
                    *value = serde_json::Value::String("<redacted>".to_owned());
                } else {
                    redact_json(value);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_json(value);
            }
        }
        _ => {}
    }
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    let compact = key.replace('_', "");
    [
        "apikey",
        "authorization",
        "connectionstring",
        "token",
        "secret",
        "password",
    ]
    .iter()
    .any(|secret| compact.contains(secret))
}
