//! `GraphRAG` indexing pipeline contracts.
//!
//! Query execution uses a separate runtime and does not share these pipeline,
//! workflow, context, callback, or statistics abstractions.

pub use crate::{
    callbacks::{IndexWorkflowCallbackChain, IndexWorkflowCallbacks, NoopIndexWorkflowCallbacks},
    stats::IndexRunStats,
    workflow::IndexWorkflowOutput,
};
