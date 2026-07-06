//! Error types for tokenization and chunking.

use thiserror::Error;

/// Result type used by tokenizers and chunkers.
pub type Result<T> = std::result::Result<T, ChunkingError>;

/// Errors raised by tokenization or chunking.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ChunkingError {
    /// Token IDs cannot be decoded by the selected tokenizer.
    #[error("token {token} cannot be decoded by {codec}")]
    InvalidToken {
        /// Codec name.
        codec: &'static str,
        /// Invalid token id.
        token: u32,
    },

    /// Chunking configuration is invalid.
    #[error("invalid chunking config: {0}")]
    InvalidConfig(String),
}
