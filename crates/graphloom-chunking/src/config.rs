//! Chunking configuration.

use std::num::NonZeroUsize;

use serde::{Deserialize, Serialize};

use crate::{ChunkingError, Result};

/// Default tiktoken encoding model used by GraphRAG.
pub const DEFAULT_ENCODING_MODEL: &str = "o200k_base";

/// Chunking configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ChunkingConfig {
    /// Tokenizer encoding model.
    #[serde(default = "default_encoding_model")]
    pub encoding_model: String,
    /// Maximum tokens per chunk.
    pub size: NonZeroUsize,
    /// Overlap tokens between adjacent chunks.
    pub overlap: usize,
    /// Metadata fields from the source document to prepend on each chunk.
    pub prepend_metadata: Vec<String>,
}

impl ChunkingConfig {
    /// Create a validated chunking config.
    ///
    /// # Errors
    ///
    /// Returns an error when `overlap >= size`.
    pub fn new(size: NonZeroUsize, overlap: usize, prepend_metadata: Vec<String>) -> Result<Self> {
        let config = Self {
            encoding_model: default_encoding_model(),
            size,
            overlap,
            prepend_metadata,
        };
        config.validate()?;
        Ok(config)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.overlap >= self.size.get() {
            return Err(ChunkingError::InvalidConfig(format!(
                "overlap {} must be smaller than size {}",
                self.overlap, self.size
            )));
        }
        Ok(())
    }
}

impl Default for ChunkingConfig {
    fn default() -> Self {
        Self {
            encoding_model: default_encoding_model(),
            size: NonZeroUsize::new(1200).unwrap_or(NonZeroUsize::MIN),
            overlap: 100,
            prepend_metadata: Vec::new(),
        }
    }
}

fn default_encoding_model() -> String {
    DEFAULT_ENCODING_MODEL.to_owned()
}
