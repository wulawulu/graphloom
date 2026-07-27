//! Public `GraphLoom` API.

pub mod index;
pub mod query;

pub use index::{
    BuildIndexOptions, CacheMode, IndexRunResult, IndexingMethod, UpdateIndexOptions, build_index,
    update_index,
};
pub(crate) use index::{build_validated_index, update_validated_index};
pub use query::{
    basic_search, basic_search_streaming, drift_search, drift_search_streaming, global_search,
    global_search_streaming, local_search, local_search_streaming, query, query_stream,
};

pub use crate::prompt_tune::{
    DocSelectionType, GenerateIndexingPromptsOptions, GeneratedIndexingPrompts,
    generate_indexing_prompts,
};
