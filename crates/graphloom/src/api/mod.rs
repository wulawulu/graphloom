//! Public `GraphLoom` API.

pub mod index;

pub(crate) use index::build_validated_index;
pub use index::{BuildIndexOptions, CacheMode, IndexRunResult, IndexingMethod, build_index};
