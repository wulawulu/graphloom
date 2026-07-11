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
        let chunks = split_text_on_tokens_with_spans(
            text,
            self.config.size.get(),
            self.config.overlap,
            self.encode.as_ref(),
            self.decode.as_ref(),
        )?;
        create_chunk_results(&chunks, transform, self.encode.as_ref())
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
    split_text_on_tokens_with_spans(text, chunk_size, chunk_overlap, encode, decode)
        .map(|chunks| chunks.into_iter().map(|chunk| chunk.text).collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TokenChunk {
    text: String,
    start_token: usize,
    end_token: usize,
}

fn split_text_on_tokens_with_spans(
    text: &str,
    chunk_size: usize,
    chunk_overlap: usize,
    encode: &TokenEncode,
    decode: &TokenDecode,
) -> Result<Vec<TokenChunk>> {
    if chunk_overlap >= chunk_size {
        return Err(ChunkingError::InvalidConfig(format!(
            "overlap {chunk_overlap} must be smaller than size {chunk_size}",
        )));
    }

    let input_tokens = encode(text)?;
    let mut result = Vec::new();
    let mut start_idx = 0usize;
    let mut cur_idx = start_idx.saturating_add(chunk_size).min(input_tokens.len());

    while start_idx < input_tokens.len() {
        let chunk_tokens = &input_tokens[start_idx..cur_idx];
        result.push(TokenChunk {
            text: decode(chunk_tokens)?,
            start_token: start_idx,
            end_token: cur_idx.saturating_sub(1),
        });
        if cur_idx == input_tokens.len() {
            break;
        }
        start_idx = start_idx.saturating_add(chunk_size.saturating_sub(chunk_overlap));
        cur_idx = start_idx.saturating_add(chunk_size).min(input_tokens.len());
    }

    Ok(result)
}

fn create_chunk_results(
    chunks: &[TokenChunk],
    transform: Option<&TextTransform>,
    encode: &TokenEncode,
) -> Result<Vec<TextChunk>> {
    let mut results = Vec::with_capacity(chunks.len());

    for (index, chunk) in chunks.iter().enumerate() {
        let text = transform.map_or_else(|| chunk.text.clone(), |transform| transform(&chunk.text));
        let token_count = encode(&text)?.len();

        results.push(TextChunk {
            original: chunk.text.clone(),
            text,
            index,
            start_char: None,
            end_char: None,
            start_token: Some(chunk.start_token),
            end_token: Some(chunk.end_token),
            token_count: Some(token_count),
        });
    }

    Ok(results)
}
