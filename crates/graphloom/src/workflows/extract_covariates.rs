//! Claim covariate extraction workflow.

use async_trait::async_trait;

use super::common::resolve_completion_model;
use crate::{
    GraphRagConfig, IndexPipelineContext, IndexWorkflow, IndexWorkflowOutput,
    IndexWorkflowRequirements, Result,
    operations::covariates::{
        ClaimExtractionConfig, covariate_value, covariates_dataframe, default_claim_entity_types,
        extract_covariates, read_text_unit_inputs,
    },
    prompts::PromptRepository,
};

/// IndexWorkflow name.
pub const EXTRACT_COVARIATES_WORKFLOW: &str = "extract_covariates";

/// Extract claim covariates from text units.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExtractCovariatesWorkflow;

#[async_trait]
impl IndexWorkflow for ExtractCovariatesWorkflow {
    fn name(&self) -> &'static str {
        EXTRACT_COVARIATES_WORKFLOW
    }

    fn requirements(&self, config: &GraphRagConfig) -> Result<IndexWorkflowRequirements> {
        let mut requirements = IndexWorkflowRequirements::default();
        if config.extract_claims.enabled {
            requirements.require_completion_model(&config.extract_claims.completion_model_id);
        }
        Ok(requirements)
    }

    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut IndexPipelineContext,
    ) -> Result<IndexWorkflowOutput> {
        if !config.extract_claims.enabled {
            return Ok(IndexWorkflowOutput {
                result: Vec::new(),
                stop: false,
                input_rows: 0,
                output_rows: 0,
            });
        }

        let text_units = read_text_unit_inputs(
            &context
                .output_table_provider()
                .read_dataframe("text_units")
                .await?,
        )?;
        let model = resolve_completion_model(
            context,
            &config.extract_claims.completion_model_id,
            EXTRACT_COVARIATES_WORKFLOW,
        )?;
        let prompt_repository = PromptRepository::new(context.prompt_root());
        let entity_types = default_claim_entity_types();
        let rows = extract_covariates(
            model.as_ref(),
            &prompt_repository,
            &text_units,
            ClaimExtractionConfig {
                prompt_path: config.extract_claims.prompt.as_deref(),
                claim_description: &config.extract_claims.description,
                entity_types: &entity_types,
                max_gleanings: config.extract_claims.max_gleanings,
                concurrency: config.concurrent_requests.max(1),
            },
            &|completed, total| {
                context
                    .callbacks
                    .progress(EXTRACT_COVARIATES_WORKFLOW, completed, Some(total));
            },
        )
        .await?;

        context
            .output_table_provider()
            .write_dataframe("covariates", covariates_dataframe(&rows)?)
            .await?;

        Ok(IndexWorkflowOutput {
            result: rows.iter().take(5).map(covariate_value).collect(),
            stop: false,
            input_rows: text_units.len(),
            output_rows: rows.len(),
        })
    }
}
