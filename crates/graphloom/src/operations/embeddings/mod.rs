//! Text embedding operations.

mod execute;
mod types;

pub(crate) use execute::embed_text_rows;
pub(crate) use types::{EmbeddingOperationConfig, EmbeddingSourceRow};
