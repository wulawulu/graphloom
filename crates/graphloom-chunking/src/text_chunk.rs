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
    /// Character index where the raw chunk text begins in the source document.
    pub start_char: usize,
    /// Character index where the raw chunk text ends in the source document.
    pub end_char: usize,
    /// Number of tokens in the final chunk text, if computed.
    pub token_count: Option<usize>,
}
