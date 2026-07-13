//! Project-scoped Tera prompt catalog, loading, context binding, and rendering.

mod catalog;
pub(crate) mod extract_claims;
pub(crate) mod extract_graph;
mod prompt;
mod repository;

pub(crate) use catalog::PromptKind;
pub(crate) use prompt::{Prompt, PromptSource, PromptTemplate};
pub(crate) use repository::PromptRepository;
