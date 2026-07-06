use std::path::PathBuf;

use thiserror::Error;

/// Result type used by storage and table-provider operations.
pub type Result<T> = std::result::Result<T, StorageError>;

/// Errors raised by storage and table-provider operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StorageError {
    /// A logical path or table name is invalid.
    #[error("invalid logical path {path:?}: {reason}")]
    InvalidPath {
        /// Rejected logical path.
        path: String,
        /// Human-readable rejection reason.
        reason: &'static str,
    },

    /// A filesystem operation failed.
    #[error("filesystem operation failed for {path}: {source}")]
    Filesystem {
        /// Path involved in the failed operation.
        path: PathBuf,
        /// Original IO error.
        #[source]
        source: std::io::Error,
    },

    /// A regular expression pattern cannot be compiled.
    #[error("invalid regex pattern {pattern:?}: {source}")]
    Regex {
        /// Rejected regular expression pattern.
        pattern: String,
        /// Original regex compilation error.
        #[source]
        source: regex::Error,
    },

    /// Stored bytes were not valid UTF-8.
    #[error("object {name} is not valid UTF-8: {source}")]
    Utf8 {
        /// Object name being decoded.
        name: String,
        /// UTF-8 conversion error.
        #[source]
        source: std::string::FromUtf8Error,
    },

    /// Arrow schema conversion failed.
    #[error("arrow conversion failed: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    /// Polars dataframe operation failed.
    #[error("polars dataframe operation failed: {0}")]
    Polars(#[from] polars_core::prelude::PolarsError),

    /// A blocking table task failed before returning its domain result.
    #[error("blocking task failed while {operation}: {source}")]
    BlockingTask {
        /// Operation being performed.
        operation: &'static str,
        /// Join failure from Tokio.
        #[source]
        source: tokio::task::JoinError,
    },

    /// A dataframe cannot be appended to an existing table because its schema
    /// differs.
    #[error("schema mismatch for column {column}: {reason}")]
    SchemaMismatch {
        /// Column name.
        column: String,
        /// Human-readable mismatch reason.
        reason: String,
    },

    /// A streaming table handle is already closed.
    #[error("table writer {name} is already closed")]
    TableClosed {
        /// Table object or provider-specific key.
        name: String,
    },

    /// A requested table is not present.
    #[error("table {name} does not exist")]
    MissingTable {
        /// Missing table name.
        name: String,
    },
}
