//! `GraphRAG` prompt catalog, loading, syntax selection, and rendering.

mod catalog;
mod prompt;
mod repository;

pub(crate) use catalog::PromptKind;
pub(crate) use prompt::{PromptSource, PromptTemplate};
pub(crate) use repository::PromptRepository;
