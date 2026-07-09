//! Public `GraphLoom` API.

pub mod index;

pub use index::{BuildIndexOptions, CacheMode, IndexRunResult, IndexingMethod, build_index};
