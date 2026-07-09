//! CLI error types and redaction helpers.

use std::{io, path::PathBuf};

use thiserror::Error;

/// CLI result type.
pub type Result<T> = std::result::Result<T, CliError>;

/// Errors raised by project initialization and indexing.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CliError {
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
    /// Unsupported provider.
    #[error("unsupported provider {provider}; only openai is implemented")]
    UnsupportedProvider {
        /// Provider name.
        provider: String,
    },
    /// Unsupported authentication method.
    #[error("unsupported auth method {auth_method}; only api_key is implemented")]
    UnsupportedAuthMethod {
        /// Auth method.
        auth_method: String,
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
    /// Unsupported indexing method.
    #[error("unsupported indexing method {method}; only standard is implemented")]
    UnsupportedMethod {
        /// Method.
        method: String,
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
