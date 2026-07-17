//! Basic Search context construction and orchestration.

mod context;
mod search;

pub(crate) use context::BasicContextBuilder;
pub(crate) use search::{basic_search, basic_search_streaming};
