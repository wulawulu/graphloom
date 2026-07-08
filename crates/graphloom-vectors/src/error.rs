//! Error types for vector stores.

use thiserror::Error;

/// Result type used by vector store implementations.
pub type Result<T> = std::result::Result<T, VectorError>;

/// Errors raised by vector store configuration and providers.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum VectorError {
    /// Vector store configuration is invalid.
    #[error("invalid vector store configuration: {message}")]
    InvalidConfig {
        /// Failure details.
        message: String,
    },

    /// A vector document is invalid.
    #[error("invalid vector document in index {index_name}: {message}")]
    InvalidDocument {
        /// Index/table name.
        index_name: String,
        /// Failure details.
        message: String,
    },

    /// `LanceDB` operation failed.
    #[error("lancedb operation failed: {source}")]
    LanceDb {
        /// `LanceDB` source error.
        #[from]
        source: lancedb::Error,
    },

    /// Arrow operation failed.
    #[error("arrow operation failed: {source}")]
    Arrow {
        /// Arrow source error.
        #[from]
        source: arrow_schema::ArrowError,
    },
}
