//! Public `GraphLoom` API.

pub mod index;

pub(crate) use index::build_validated_index;
pub use index::{BuildIndexOptions, CacheMode, IndexRunResult, build_index};
