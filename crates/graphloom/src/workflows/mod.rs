//! Built-in indexing workflows.

mod base_text_units;
pub(crate) mod common;
mod communities;
mod community_reports;
mod extract_covariates;
mod extract_graph;
mod final_documents;
mod final_text_units;
mod finalize_graph;
mod generate_text_embeddings;
pub(crate) mod input_documents;

pub use base_text_units::{CREATE_BASE_TEXT_UNITS_WORKFLOW, CreateBaseTextUnitsWorkflow};
pub use communities::{CREATE_COMMUNITIES_WORKFLOW, CreateCommunitiesWorkflow};
pub use community_reports::{CREATE_COMMUNITY_REPORTS_WORKFLOW, CreateCommunityReportsWorkflow};
pub use extract_covariates::{EXTRACT_COVARIATES_WORKFLOW, ExtractCovariatesWorkflow};
pub use extract_graph::{EXTRACT_GRAPH_WORKFLOW, ExtractGraphWorkflow};
pub use final_documents::{CREATE_FINAL_DOCUMENTS_WORKFLOW, CreateFinalDocumentsWorkflow};
pub use final_text_units::{CREATE_FINAL_TEXT_UNITS_WORKFLOW, CreateFinalTextUnitsWorkflow};
pub use finalize_graph::{FINALIZE_GRAPH_WORKFLOW, FinalizeGraphWorkflow};
pub use generate_text_embeddings::{
    GENERATE_TEXT_EMBEDDINGS_WORKFLOW, GenerateTextEmbeddingsWorkflow,
};
pub use input_documents::{LOAD_INPUT_DOCUMENTS_WORKFLOW, LoadInputDocumentsWorkflow};

use crate::WorkflowRegistry;

/// Workflow prefix used by focused pipeline tests.
#[cfg(test)]
pub(crate) const STEP5_WORKFLOWS: &[&str] = &[
    LOAD_INPUT_DOCUMENTS_WORKFLOW,
    CREATE_BASE_TEXT_UNITS_WORKFLOW,
    CREATE_FINAL_DOCUMENTS_WORKFLOW,
];

/// Workflow prefix used by focused validation tests.
#[cfg(test)]
pub(crate) const STEP8_WORKFLOWS: &[&str] = &[
    LOAD_INPUT_DOCUMENTS_WORKFLOW,
    CREATE_BASE_TEXT_UNITS_WORKFLOW,
    CREATE_FINAL_DOCUMENTS_WORKFLOW,
    EXTRACT_GRAPH_WORKFLOW,
    FINALIZE_GRAPH_WORKFLOW,
    EXTRACT_COVARIATES_WORKFLOW,
    CREATE_COMMUNITIES_WORKFLOW,
    CREATE_FINAL_TEXT_UNITS_WORKFLOW,
    CREATE_COMMUNITY_REPORTS_WORKFLOW,
];

/// Workflow names for the standard indexing pipeline.
pub const STANDARD_WORKFLOWS: &[&str] = &[
    LOAD_INPUT_DOCUMENTS_WORKFLOW,
    CREATE_BASE_TEXT_UNITS_WORKFLOW,
    CREATE_FINAL_DOCUMENTS_WORKFLOW,
    EXTRACT_GRAPH_WORKFLOW,
    FINALIZE_GRAPH_WORKFLOW,
    EXTRACT_COVARIATES_WORKFLOW,
    CREATE_COMMUNITIES_WORKFLOW,
    CREATE_FINAL_TEXT_UNITS_WORKFLOW,
    CREATE_COMMUNITY_REPORTS_WORKFLOW,
    GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
];

/// Register the smallest workflow prefix used by focused tests.
#[cfg(test)]
pub(crate) fn register_step5_workflows(registry: &mut WorkflowRegistry) {
    registry.register(LoadInputDocumentsWorkflow);
    registry.register(CreateBaseTextUnitsWorkflow);
    registry.register(CreateFinalDocumentsWorkflow);
}

#[cfg(test)]
pub(crate) fn register_step6_workflows(registry: &mut WorkflowRegistry) {
    register_step5_workflows(registry);
    registry.register(ExtractGraphWorkflow);
    registry.register(FinalizeGraphWorkflow);
}

#[cfg(test)]
pub(crate) fn register_step7_workflows(registry: &mut WorkflowRegistry) {
    register_step6_workflows(registry);
    registry.register(ExtractCovariatesWorkflow);
    registry.register(CreateCommunitiesWorkflow);
    registry.register(CreateFinalTextUnitsWorkflow);
}

#[cfg(test)]
pub(crate) fn register_step8_workflows(registry: &mut WorkflowRegistry) {
    register_step7_workflows(registry);
    registry.register(CreateCommunityReportsWorkflow);
}

/// Register every workflow in the standard indexing pipeline.
pub fn register_standard_workflows(registry: &mut WorkflowRegistry) {
    registry.register(LoadInputDocumentsWorkflow);
    registry.register(CreateBaseTextUnitsWorkflow);
    registry.register(CreateFinalDocumentsWorkflow);
    registry.register(ExtractGraphWorkflow);
    registry.register(FinalizeGraphWorkflow);
    registry.register(ExtractCovariatesWorkflow);
    registry.register(CreateCommunitiesWorkflow);
    registry.register(CreateFinalTextUnitsWorkflow);
    registry.register(CreateCommunityReportsWorkflow);
    registry.register(GenerateTextEmbeddingsWorkflow);
}
