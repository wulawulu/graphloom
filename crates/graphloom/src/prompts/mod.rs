//! `GraphRAG` prompt catalog, loading, syntax selection, and rendering.

mod catalog;
mod loader;
mod renderer;

pub(crate) use catalog::PromptKind;
pub(crate) use loader::PromptLoader;
