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

    /// Arrow schema or array conversion failed.
    #[error("arrow conversion failed: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    /// Parquet read or write failed.
    #[error("parquet operation failed: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),

    /// A blocking table task failed before returning its domain result.
    #[error("blocking task failed while {operation}: {source}")]
    BlockingTask {
        /// Operation being performed.
        operation: &'static str,
        /// Join failure from Tokio.
        #[source]
        source: tokio::task::JoinError,
    },

    /// A table value does not match the declared Arrow schema.
    #[error("schema mismatch for column {column}: {reason}")]
    SchemaMismatch {
        /// Column name.
        column: String,
        /// Human-readable mismatch reason.
        reason: String,
    },

    /// A requested table is not present.
    #[error("table {name} does not exist")]
    MissingTable {
        /// Missing table name.
        name: String,
    },
}
