//! Chunk result data model.

use serde::{Deserialize, Serialize};

/// Result of chunking a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TextChunk {
    /// Raw original text chunk before any transformation.
    pub original: String,
    /// Final text content of this chunk.
    pub text: String,
    /// Zero-based index of this chunk within the source document.
    pub index: usize,
    /// Inclusive character index where the raw chunk begins, when known.
    #[serde(default)]
    pub start_char: Option<usize>,
    /// Inclusive character index where the raw chunk ends, when known.
    #[serde(default)]
    pub end_char: Option<usize>,
    /// Inclusive token index where the raw chunk begins, when known.
    #[serde(default)]
    pub start_token: Option<usize>,
    /// Inclusive token index where the raw chunk ends, when known.
    #[serde(default)]
    pub end_token: Option<usize>,
    /// Number of tokens in the final chunk text, if computed.
    pub token_count: Option<usize>,
}
