//! Chunker trait.

use crate::{Result, TextChunk};

/// Text transform applied to each raw chunk.
pub type TextTransform = dyn Fn(&str) -> String + Send + Sync;

/// Chunker abstraction.
pub trait Chunker: Send + Sync + std::fmt::Debug {
    /// Split text into chunks.
    ///
    /// # Errors
    ///
    /// Returns an error when encoding, decoding, or configuration validation
    /// fails.
    fn chunk(&self, text: &str, transform: Option<&TextTransform>) -> Result<Vec<TextChunk>>;
}
