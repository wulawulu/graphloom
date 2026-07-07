//! Text tokenization and chunking primitives for `GraphLoom`.
//!
//! The crate follows Microsoft `GraphRAG`'s `graphrag-chunking` package shape:
//! chunk configuration, chunk result creation, tokenizer abstraction, token
//! overlap chunking, and metadata transformers live in separate modules.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::sync::Arc;

mod chunker;
mod config;
mod encoding;
mod error;
mod semantic_text_chunker;
mod text_chunk;
mod tiktoken;
mod token_chunker;
mod transformers;

pub use chunker::{Chunker, TextTransform};
pub use config::{ChunkerType, ChunkingConfig};
pub use encoding::{TokenDecode, TokenEncode, unicode_scalar_decode, unicode_scalar_encode};
pub use error::{ChunkingError, Result};
pub use semantic_text_chunker::SemanticTextChunker;
pub use text_chunk::TextChunk;
pub use token_chunker::{TokenOverlapChunker, split_text_on_tokens};
pub use transformers::{MetadataTransform, add_metadata, prepend_metadata};

/// Create a chunker selected by [`ChunkingConfig::chunker_type`].
///
/// The default [`ChunkerType::TokenOverlap`] creates the Microsoft `GraphRAG`
/// compatible strict token window chunker, while [`ChunkerType::SemanticText`]
/// creates the `text-splitter` based semantic text chunker.
///
/// # Errors
///
/// Returns an error when the config is invalid or the requested tokenizer
/// cannot be initialized.
pub fn create_chunker(config: &ChunkingConfig) -> Result<Box<dyn Chunker>> {
    config.validate()?;
    match config.chunker_type {
        ChunkerType::TokenOverlap => token_overlap_chunker(config),
        ChunkerType::SemanticText => Ok(Box::new(SemanticTextChunker::new(config.clone())?)),
    }
}

fn token_overlap_chunker(config: &ChunkingConfig) -> Result<Box<dyn Chunker>> {
    let tokenizer = tiktoken::TiktokenCodec::new(config.encoding_model.clone())?;
    let encode_tokenizer = tokenizer.clone();
    let decode_tokenizer = tokenizer;
    let encode: Arc<TokenEncode> = Arc::new(move |text| Ok(encode_tokenizer.encode(text)));
    let decode: Arc<TokenDecode> = Arc::new(move |tokens| decode_tokenizer.decode(tokens));

    TokenOverlapChunker::new(config.clone(), encode, decode)
        .map(|chunker| Box::new(chunker) as Box<dyn Chunker>)
}

#[cfg(test)]
mod tests;
