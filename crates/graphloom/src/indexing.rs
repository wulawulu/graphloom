//! GraphRAG indexing pipeline contracts.
//!
//! Query execution uses a separate runtime and does not share these pipeline,
//! workflow, context, callback, or statistics abstractions.

pub use crate::{
    callbacks::{IndexWorkflowCallbackChain, IndexWorkflowCallbacks, NoopIndexWorkflowCallbacks},
    context::IndexPipelineContext,
    pipeline::{IndexPipeline, IndexPipelineFactory, IndexPipelineStep},
    runtime::ModelRegistry,
    stats::IndexRunStats,
    workflow::{
        IndexWorkflow, IndexWorkflowOutput, IndexWorkflowRegistry, IndexWorkflowRequirements,
    },
};
