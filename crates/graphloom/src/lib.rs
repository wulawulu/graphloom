//! Public `GraphLoom` crate.
//!
//! The top-level crate owns configuration, provider assembly, and workflow
//! orchestration for the indexing pipeline.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

mod callbacks;
mod config;
mod context;
mod dataframe;
mod error;
mod operations;
mod pipeline;
mod stats;
mod workflow;
pub mod workflows;

pub use callbacks::{NoopWorkflowCallbacks, WorkflowCallbacks};
pub use config::{
    ClusterGraphConfig, ExtractClaimsConfig, ExtractGraphConfig, GraphRagConfig, InputConfig,
    SnapshotsConfig, SummarizeDescriptionsConfig,
};
pub use context::PipelineRunContext;
pub use error::{GraphLoomError, Result};
pub use graphloom_common as common;
pub use graphloom_storage as storage;
pub use pipeline::{Pipeline, PipelineFactory};
pub use stats::PipelineRunStats;
pub use workflow::{Workflow, WorkflowFunctionOutput, WorkflowRegistry};
pub use workflows::{
    CREATE_BASE_TEXT_UNITS_WORKFLOW, CREATE_COMMUNITIES_WORKFLOW, CREATE_FINAL_DOCUMENTS_WORKFLOW,
    CREATE_FINAL_TEXT_UNITS_WORKFLOW, EXTRACT_COVARIATES_WORKFLOW, EXTRACT_GRAPH_WORKFLOW,
    FINALIZE_GRAPH_WORKFLOW, LOAD_INPUT_DOCUMENTS_WORKFLOW, register_step5_workflows,
    register_step6_workflows, register_step7_workflows,
};

#[cfg(test)]
mod tests;
