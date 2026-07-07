//! Built-in Step-5 workflows.

mod base_text_units;
mod final_documents;
mod graph;
mod input_documents;

pub use base_text_units::{CREATE_BASE_TEXT_UNITS_WORKFLOW, CreateBaseTextUnitsWorkflow};
pub use final_documents::{CREATE_FINAL_DOCUMENTS_WORKFLOW, CreateFinalDocumentsWorkflow};
pub use graph::{
    EXTRACT_GRAPH_WORKFLOW, ExtractGraphWorkflow, FINALIZE_GRAPH_WORKFLOW, FinalizeGraphWorkflow,
};
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
