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

use crate::{IndexWorkflowRegistry, Result};

/// IndexWorkflow prefix used by focused pipeline tests.
#[cfg(test)]
pub(crate) const STEP5_WORKFLOWS: &[&str] = &[
    LOAD_INPUT_DOCUMENTS_WORKFLOW,
    CREATE_BASE_TEXT_UNITS_WORKFLOW,
    CREATE_FINAL_DOCUMENTS_WORKFLOW,
];

/// IndexWorkflow prefix used by focused validation tests.
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

/// IndexWorkflow names for the standard indexing pipeline.
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
pub(crate) fn register_step5_workflows(registry: &mut IndexWorkflowRegistry) -> Result<()> {
    registry.register(LoadInputDocumentsWorkflow)?;
    registry.register(CreateBaseTextUnitsWorkflow)?;
    registry.register(CreateFinalDocumentsWorkflow)
}

#[cfg(test)]
pub(crate) fn register_step6_workflows(registry: &mut IndexWorkflowRegistry) -> Result<()> {
    register_step5_workflows(registry)?;
    registry.register(ExtractGraphWorkflow)?;
    registry.register(FinalizeGraphWorkflow)
}

#[cfg(test)]
pub(crate) fn register_step7_workflows(registry: &mut IndexWorkflowRegistry) -> Result<()> {
    register_step6_workflows(registry)?;
    registry.register(ExtractCovariatesWorkflow)?;
    registry.register(CreateCommunitiesWorkflow)?;
    registry.register(CreateFinalTextUnitsWorkflow)
}

#[cfg(test)]
pub(crate) fn register_step8_workflows(registry: &mut IndexWorkflowRegistry) -> Result<()> {
    register_step7_workflows(registry)?;
    registry.register(CreateCommunityReportsWorkflow)
}

/// Register every workflow in the standard indexing pipeline.
pub fn register_standard_index_workflows(registry: &mut IndexWorkflowRegistry) -> Result<()> {
    registry.register(LoadInputDocumentsWorkflow)?;
    registry.register(CreateBaseTextUnitsWorkflow)?;
    registry.register(CreateFinalDocumentsWorkflow)?;
    registry.register(ExtractGraphWorkflow)?;
    registry.register(FinalizeGraphWorkflow)?;
    registry.register(ExtractCovariatesWorkflow)?;
    registry.register(CreateCommunitiesWorkflow)?;
    registry.register(CreateFinalTextUnitsWorkflow)?;
    registry.register(CreateCommunityReportsWorkflow)?;
    registry.register(GenerateTextEmbeddingsWorkflow)
}

#[cfg(test)]
mod tests {
    use super::{
        CreateBaseTextUnitsWorkflow, CreateCommunityReportsWorkflow, ExtractCovariatesWorkflow,
        ExtractGraphWorkflow, GenerateTextEmbeddingsWorkflow,
    };
    use crate::{GraphRagConfig, IndexWorkflow, prompts::PromptKind};

    #[test]
    fn test_should_declare_model_requirements_from_active_workflows() {
        let mut config = GraphRagConfig::default();
        config.extract_graph.completion_model_id = "extract".to_owned();
        config.summarize_descriptions.completion_model_id = "summarize".to_owned();
        config.extract_claims.completion_model_id = "claims".to_owned();
        config.community_reports.completion_model_id = "reports".to_owned();
        config.embed_text.embedding_model_id = "embeddings".to_owned();

        let graph = ExtractGraphWorkflow
            .requirements(&config)
            .expect("graph requirements");
        assert_eq!(
            graph.completion_models().collect::<Vec<_>>(),
            vec!["extract", "summarize"]
        );
        assert_eq!(
            graph
                .prompt_requirements()
                .map(|requirement| requirement.kind)
                .collect::<Vec<_>>(),
            vec![PromptKind::ExtractGraph, PromptKind::SummarizeDescriptions]
        );
        assert!(!graph.requires_chunking_config());
        assert_eq!(
            graph
                .tokenizer_requirements()
                .map(|requirement| requirement.source.as_str())
                .collect::<Vec<_>>(),
            vec!["chunking.encoding_model"]
        );
        let base = CreateBaseTextUnitsWorkflow
            .requirements(&config)
            .expect("base text unit requirements");
        assert!(base.requires_chunking_config());
        assert_eq!(base.tokenizer_requirements().count(), 1);
        let report_requirements = CreateCommunityReportsWorkflow
            .requirements(&config)
            .expect("report requirements");
        assert_eq!(
            report_requirements.completion_models().collect::<Vec<_>>(),
            vec!["reports"]
        );
        assert_eq!(
            report_requirements
                .prompt_requirements()
                .map(|requirement| requirement.kind)
                .collect::<Vec<_>>(),
            vec![PromptKind::CommunityReportGraph]
        );
        assert_eq!(
            report_requirements
                .tokenizer_requirements()
                .map(|requirement| requirement.source.as_str())
                .collect::<Vec<_>>(),
            vec!["completion_models.reports.encoding_model"]
        );
        let embedding_requirements = GenerateTextEmbeddingsWorkflow
            .requirements(&config)
            .expect("embedding requirements");
        assert_eq!(
            embedding_requirements
                .embedding_models()
                .collect::<Vec<_>>(),
            vec!["embeddings"]
        );
        assert!(embedding_requirements.requires_vector_store());
        assert!(!embedding_requirements.requires_chunking_config());
        assert_eq!(
            embedding_requirements
                .tokenizer_requirements()
                .map(|requirement| requirement.source.as_str())
                .collect::<Vec<_>>(),
            vec!["embedding_models.embeddings.encoding_model"]
        );

        assert!(
            ExtractCovariatesWorkflow
                .requirements(&config)
                .expect("disabled claims requirements")
                .completion_models()
                .next()
                .is_none()
        );
        config.extract_claims.enabled = true;
        assert_eq!(
            ExtractCovariatesWorkflow
                .requirements(&config)
                .expect("enabled claims requirements")
                .completion_models()
                .collect::<Vec<_>>(),
            vec!["claims"]
        );
    }
}
