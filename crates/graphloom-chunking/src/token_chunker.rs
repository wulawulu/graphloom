//! Token-overlap chunker.

use std::{fmt, sync::Arc};

use crate::{
    Chunker, ChunkingConfig, ChunkingError, Result, TextChunk, TextTransform, TokenDecode,
    TokenEncode,
};

/// Token overlap chunker.
#[derive(Clone)]
pub struct TokenOverlapChunker {
    config: ChunkingConfig,
    encode: Arc<TokenEncode>,
    decode: Arc<TokenDecode>,
}

impl TokenOverlapChunker {
    /// Create a token overlap chunker from caller-provided encode/decode functions.
    ///
    /// # Errors
    ///
    /// Returns an error when the supplied config is invalid.
    pub fn new(
        config: ChunkingConfig,
        encode: Arc<TokenEncode>,
        decode: Arc<TokenDecode>,
    ) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            encode,
            decode,
        })
    }
}

impl fmt::Debug for TokenOverlapChunker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TokenOverlapChunker")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl Chunker for TokenOverlapChunker {
    fn chunk(&self, text: &str, transform: Option<&TextTransform>) -> Result<Vec<TextChunk>> {
        let chunks = split_text_on_tokens(
            text,
            self.config.size.get(),
            self.config.overlap,
            self.encode.as_ref(),
            self.decode.as_ref(),
        )?;
        create_chunk_results(&chunks, transform, Some(self.encode.as_ref()))
    }
}

/// Split a single text and return chunks using caller-provided token functions.
///
/// # Errors
///
/// Returns an error if token encoding or decoding fails, or when overlap is
/// greater than or equal to chunk size.
pub fn split_text_on_tokens(
    text: &str,
    chunk_size: usize,
    chunk_overlap: usize,
    encode: &TokenEncode,
    decode: &TokenDecode,
) -> Result<Vec<String>> {
    if chunk_overlap >= chunk_size {
        return Err(ChunkingError::InvalidConfig(format!(
            "overlap {chunk_overlap} must be smaller than size {chunk_size}",
        )));
    }

    let input_tokens = encode(text)?;
    let mut result = Vec::new();
    let mut start_idx = 0usize;
    let mut cur_idx = start_idx.saturating_add(chunk_size).min(input_tokens.len());
    let mut chunk_tokens = &input_tokens[start_idx..cur_idx];

    while start_idx < input_tokens.len() {
        result.push(decode(chunk_tokens)?);
        if cur_idx == input_tokens.len() {
            break;
        }
        start_idx = start_idx.saturating_add(chunk_size.saturating_sub(chunk_overlap));
        cur_idx = start_idx.saturating_add(chunk_size).min(input_tokens.len());
        chunk_tokens = &input_tokens[start_idx..cur_idx];
    }

    Ok(result)
}

fn create_chunk_results(
    chunks: &[String],
    transform: Option<&TextTransform>,
    encode: Option<&TokenEncode>,
) -> Result<Vec<TextChunk>> {
    let mut results = Vec::with_capacity(chunks.len());
    let mut start_char = 0usize;

    for (index, chunk) in chunks.iter().enumerate() {
        let char_len = chunk.chars().count();
        let end_char = start_char.saturating_add(char_len).saturating_sub(1);
        let text = transform.map_or_else(|| chunk.clone(), |transform| transform(chunk));
        let token_count = encode
            .map(|encode| encode(&text).map(|tokens| tokens.len()))
            .transpose()?;

        results.push(TextChunk {
            original: chunk.clone(),
            text,
            index,
            start_char,
            end_char,
            token_count,
        });
        start_char = end_char.saturating_add(1);
    }

    Ok(results)
}
