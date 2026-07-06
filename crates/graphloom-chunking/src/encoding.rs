//! Token encode/decode function types.

use crate::{ChunkingError, Result};

/// Function type used to encode text into token ids.
pub type TokenEncode = dyn Fn(&str) -> Result<Vec<u32>> + Send + Sync;

/// Function type used to decode token ids into text.
pub type TokenDecode = dyn Fn(&[u32]) -> Result<String> + Send + Sync;

/// Deterministic Unicode-scalar encoder used by tests and simple local runs.
///
/// This is a convenience function, not the chunking crate's tokenizer contract.
/// The production tokenizer choice should be injected by the caller through
/// encode/decode functions, matching `GraphRAG`'s `TokenChunker`.
///
/// # Errors
///
/// This implementation is infallible, but returns [`Result`] to match the
/// encode function contract accepted by [`crate::TokenOverlapChunker`].
pub fn unicode_scalar_encode(text: &str) -> Result<Vec<u32>> {
    Ok(text.chars().map(u32::from).collect())
}

/// Decode tokens produced by [`unicode_scalar_encode`].
///
/// # Errors
///
/// Returns an error when a token id is not a valid Unicode scalar value.
pub fn unicode_scalar_decode(tokens: &[u32]) -> Result<String> {
    tokens
        .iter()
        .map(|token| {
            char::from_u32(*token).ok_or(ChunkingError::InvalidToken {
                codec: "unicode-scalar",
                token: *token,
            })
        })
        .collect()
}
