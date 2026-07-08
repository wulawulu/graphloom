//! Built-in Step-5 workflows.

mod base_text_units;
pub(crate) mod common;
mod communities;
mod extract_covariates;
mod extract_graph;
mod final_documents;
mod final_text_units;
mod finalize_graph;
pub(crate) mod input_documents;

pub use base_text_units::{CREATE_BASE_TEXT_UNITS_WORKFLOW, CreateBaseTextUnitsWorkflow};
pub use communities::{CREATE_COMMUNITIES_WORKFLOW, CreateCommunitiesWorkflow};
pub use extract_covariates::{EXTRACT_COVARIATES_WORKFLOW, ExtractCovariatesWorkflow};
pub use extract_graph::{EXTRACT_GRAPH_WORKFLOW, ExtractGraphWorkflow};
pub use final_documents::{CREATE_FINAL_DOCUMENTS_WORKFLOW, CreateFinalDocumentsWorkflow};
pub use final_text_units::{CREATE_FINAL_TEXT_UNITS_WORKFLOW, CreateFinalTextUnitsWorkflow};
pub use finalize_graph::{FINALIZE_GRAPH_WORKFLOW, FinalizeGraphWorkflow};
pub use input_documents::{LOAD_INPUT_DOCUMENTS_WORKFLOW, LoadInputDocumentsWorkflow};

use crate::WorkflowRegistry;

/// Workflow name for `load_input_documents`.
pub const STEP5_WORKFLOWS: &[&str] = &[
    LOAD_INPUT_DOCUMENTS_WORKFLOW,
    CREATE_BASE_TEXT_UNITS_WORKFLOW,
    CREATE_FINAL_DOCUMENTS_WORKFLOW,
];

/// Workflow names for the implemented Step-6 standard prefix.
pub const STEP6_WORKFLOWS: &[&str] = &[
    LOAD_INPUT_DOCUMENTS_WORKFLOW,
    CREATE_BASE_TEXT_UNITS_WORKFLOW,
    CREATE_FINAL_DOCUMENTS_WORKFLOW,
    EXTRACT_GRAPH_WORKFLOW,
    FINALIZE_GRAPH_WORKFLOW,
];

/// Workflow names for the implemented Step-7 standard prefix.
pub const STEP7_WORKFLOWS: &[&str] = &[
    LOAD_INPUT_DOCUMENTS_WORKFLOW,
    CREATE_BASE_TEXT_UNITS_WORKFLOW,
    CREATE_FINAL_DOCUMENTS_WORKFLOW,
    EXTRACT_GRAPH_WORKFLOW,
    FINALIZE_GRAPH_WORKFLOW,
    EXTRACT_COVARIATES_WORKFLOW,
    CREATE_COMMUNITIES_WORKFLOW,
    CREATE_FINAL_TEXT_UNITS_WORKFLOW,
];

/// Register built-in Step-5 workflows.
pub fn register_step5_workflows(registry: &mut WorkflowRegistry) {
    registry.register(LoadInputDocumentsWorkflow);
    registry.register(CreateBaseTextUnitsWorkflow);
    registry.register(CreateFinalDocumentsWorkflow);
}

/// Register built-in Step-6 workflows.
pub fn register_step6_workflows(registry: &mut WorkflowRegistry) {
    register_step5_workflows(registry);
    registry.register(ExtractGraphWorkflow);
    registry.register(FinalizeGraphWorkflow);
}

/// Register built-in Step-7 workflows.
pub fn register_step7_workflows(registry: &mut WorkflowRegistry) {
    register_step6_workflows(registry);
    registry.register(ExtractCovariatesWorkflow);
    registry.register(CreateCommunitiesWorkflow);
    registry.register(CreateFinalTextUnitsWorkflow);
}
