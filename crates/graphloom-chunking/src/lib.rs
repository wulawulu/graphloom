//! Text tokenization and chunking primitives for `GraphLoom`.
//!
//! The crate follows Microsoft `GraphRAG`'s `graphrag-chunking` package shape:
//! chunk configuration, chunk result creation, tokenizer abstraction, token
//! overlap chunking, and metadata transformers live in separate modules.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

mod chunker;
mod config;
mod encoding;
mod error;
mod text_chunk;
mod token_chunker;
mod transformers;

pub use chunker::{Chunker, TextTransform};
pub use config::ChunkingConfig;
pub use encoding::{TokenDecode, TokenEncode, unicode_scalar_decode, unicode_scalar_encode};
pub use error::{ChunkingError, Result};
pub use text_chunk::TextChunk;
pub use token_chunker::{TokenOverlapChunker, split_text_on_tokens};
pub use transformers::{MetadataTransform, add_metadata, prepend_metadata};

#[cfg(test)]
mod tests;
