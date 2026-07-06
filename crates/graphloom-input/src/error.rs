//! Error types for input readers.

use graphloom_storage::StorageError;
use regex::Error as RegexError;
use thiserror::Error;

/// Result type used by input readers.
pub type Result<T> = std::result::Result<T, InputError>;

/// Errors raised while discovering or reading input documents.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum InputError {
    /// A file pattern cannot be compiled.
    #[error("invalid input file pattern {pattern:?}: {source}")]
    InvalidPattern {
        /// Rejected regular expression pattern.
        pattern: String,
        /// Original regex compilation error.
        #[source]
        source: RegexError,
    },

    /// A storage operation failed.
    #[error("storage operation failed: {0}")]
    Storage(#[from] StorageError),

    /// Input bytes were not valid UTF-8.
    #[error("input file {path} is not valid UTF-8: {source}")]
    Utf8 {
        /// Input object path.
        path: String,
        /// UTF-8 conversion error.
        #[source]
        source: std::string::FromUtf8Error,
    },
}
