//! Public `GraphLoom` crate.
//!
//! The top-level crate owns configuration, provider assembly, and workflow
//! orchestration for the indexing pipeline.

#![deny(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

pub mod api;
mod callbacks;
pub mod cli;
mod config;
mod context;
mod dataframe;
mod error;
pub mod indexing;
mod operations;
#[allow(
    unsafe_code,
    reason = "the Windows ordinal comparison API requires a narrowly scoped FFI call"
)]
mod path_safety;
mod pipeline;
mod project;
pub(crate) mod prompts;
mod runtime;
mod stats;
mod workflow;
pub mod workflows;

pub use config::{
    ALL_EMBEDDINGS, COMMUNITY_FULL_CONTENT_EMBEDDING, CacheConfig, CacheStorageConfig,
    ClusterGraphConfig, CommunityReportsConfig, DEFAULT_EMBEDDINGS, ENTITY_DESCRIPTION_EMBEDDING,
    EmbedTextConfig, ExtractClaimsConfig, ExtractGraphConfig, GraphRagConfig, InputConfig,
    ReportingConfig, SnapshotsConfig, StorageConfig, SummarizeDescriptionsConfig,
    TEXT_UNIT_TEXT_EMBEDDING,
};
pub use error::{GraphLoomError, Result};
pub use graphloom_storage as storage;
pub use indexing::{
    IndexPipeline, IndexPipelineContext, IndexPipelineFactory, IndexPipelineStep, IndexRunStats,
    IndexWorkflow, IndexWorkflowCallbackChain, IndexWorkflowCallbacks, IndexWorkflowOutput,
    IndexWorkflowRegistry, IndexWorkflowRequirements, ModelRegistry, NoopIndexWorkflowCallbacks,
};
pub(crate) use runtime::IndexRuntimeServices;
pub use workflows::{
    CREATE_BASE_TEXT_UNITS_WORKFLOW, CREATE_COMMUNITIES_WORKFLOW,
    CREATE_COMMUNITY_REPORTS_WORKFLOW, CREATE_FINAL_DOCUMENTS_WORKFLOW,
    CREATE_FINAL_TEXT_UNITS_WORKFLOW, EXTRACT_COVARIATES_WORKFLOW, EXTRACT_GRAPH_WORKFLOW,
    FINALIZE_GRAPH_WORKFLOW, GENERATE_TEXT_EMBEDDINGS_WORKFLOW, LOAD_INPUT_DOCUMENTS_WORKFLOW,
    register_standard_index_workflows,
};
#[cfg(test)]
pub(crate) use workflows::{
    register_step5_workflows, register_step6_workflows, register_step7_workflows,
    register_step8_workflows,
};

#[cfg(test)]
mod tests;
