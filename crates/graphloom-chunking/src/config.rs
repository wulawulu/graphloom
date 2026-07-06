//! Chunking configuration.

use std::num::NonZeroUsize;

use serde::{Deserialize, Serialize};

use crate::{ChunkingError, Result};

/// Chunking configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ChunkingConfig {
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
