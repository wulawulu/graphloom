//! Public `GraphLoom` crate.
//!
//! The top-level crate owns configuration, provider assembly, and workflow
//! orchestration for the indexing pipeline.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::format_push_string,
    clippy::map_unwrap_or,
    clippy::needless_pass_by_value,
    clippy::redundant_closure,
    clippy::redundant_closure_for_method_calls,
    clippy::too_many_lines,
    clippy::unnecessary_wraps,
    reason = "GraphLoom keeps Microsoft GraphRAG-compatible numeric ids, Polars helper Result \
              signatures, and integration-style tests; these pedantic lints are low signal for \
              this indexing crate"
)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

pub mod api;
mod callbacks;
pub mod cli;
mod config;
mod context;
mod dataframe;
mod error;
mod operations;
mod pipeline;
mod project;
mod runtime;
mod stats;
mod workflow;
pub mod workflows;

pub use callbacks::{CallbackChain, NoopWorkflowCallbacks, WorkflowCallbacks};
pub use config::{
    ALL_EMBEDDINGS, COMMUNITY_FULL_CONTENT_EMBEDDING, CacheConfig, CacheStorageConfig,
    ClusterGraphConfig, CommunityReportsConfig, DEFAULT_EMBEDDINGS, ENTITY_DESCRIPTION_EMBEDDING,
    EmbedTextConfig, ExtractClaimsConfig, ExtractGraphConfig, GraphRagConfig, InputConfig,
    ReportingConfig, SnapshotsConfig, StorageConfig, SummarizeDescriptionsConfig,
    TEXT_UNIT_TEXT_EMBEDDING,
};
pub use context::PipelineRunContext;
pub use error::{GraphLoomError, Result};
pub use graphloom_common as common;
pub use graphloom_storage as storage;
pub use pipeline::{Pipeline, PipelineFactory};
pub use stats::PipelineRunStats;
pub use workflow::{Workflow, WorkflowFunctionOutput, WorkflowRegistry};
pub use workflows::{
    CREATE_BASE_TEXT_UNITS_WORKFLOW, CREATE_COMMUNITIES_WORKFLOW,
    CREATE_COMMUNITY_REPORTS_WORKFLOW, CREATE_FINAL_DOCUMENTS_WORKFLOW,
    CREATE_FINAL_TEXT_UNITS_WORKFLOW, EXTRACT_COVARIATES_WORKFLOW, EXTRACT_GRAPH_WORKFLOW,
    FINALIZE_GRAPH_WORKFLOW, GENERATE_TEXT_EMBEDDINGS_WORKFLOW, LOAD_INPUT_DOCUMENTS_WORKFLOW,
    register_step5_workflows, register_step6_workflows, register_step7_workflows,
    register_step8_workflows, register_step9_workflows,
};

#[cfg(test)]
mod tests;
