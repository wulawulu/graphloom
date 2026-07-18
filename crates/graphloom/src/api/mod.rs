//! Public `GraphLoom` API.

pub mod index;
pub mod query;

pub(crate) use index::build_validated_index;
pub use index::{BuildIndexOptions, CacheMode, IndexRunResult, IndexingMethod, build_index};
pub use query::{
    basic_search, basic_search_streaming, drift_search, drift_search_streaming, query, query_stream,
};
