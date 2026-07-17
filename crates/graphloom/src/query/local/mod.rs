//! GraphRAG-compatible Local Search.

mod context;
mod search;

pub(crate) use context::LocalContextBuilder;
pub(crate) use search::{local_search, local_search_streaming};
