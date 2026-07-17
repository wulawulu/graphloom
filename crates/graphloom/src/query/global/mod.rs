//! GraphRAG-compatible Global Search context, map, and reduce orchestration.

mod context;
mod parse;
mod random;
mod search;

pub(crate) use context::GlobalContextBuilder;
pub use parse::{MapPoint, MapSearchResult};
pub(crate) use search::{global_search, global_search_streaming};
