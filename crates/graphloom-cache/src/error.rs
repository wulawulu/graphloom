use std::path::PathBuf;

use thiserror::Error;

/// Result type used by cache operations.
pub type Result<T> = std::result::Result<T, CacheError>;

/// Errors raised by cache providers.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CacheError {
    /// The requested cache key is invalid.
    #[error("invalid cache key {key:?}: {reason}")]
    InvalidKey {
        /// Rejected cache key.
        key: String,
        /// Human-readable rejection reason.
        reason: &'static str,
    },

    /// A storage operation failed.
    #[error("cache storage operation failed: {0}")]
    Storage(#[from] graphloom_storage::StorageError),

    /// JSON serialization or deserialization failed.
    #[error("cache JSON operation failed for {key}: {source}")]
    Json {
        /// Cache key being decoded or encoded.
        key: String,
        /// JSON error.
        #[source]
        source: serde_json::Error,
    },

    /// Cached bytes were not valid UTF-8.
    #[error("cache entry {key} is not valid UTF-8: {source}")]
    Utf8 {
        /// Cache key being decoded.
        key: String,
        /// UTF-8 error.
        #[source]
        source: std::string::FromUtf8Error,
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
}
