//! Community report generation workflow.

use async_trait::async_trait;
use graphloom_llm::{PromptLoader, TiktokenTokenizer};

use super::common::{resolve_completion_encoding_model, resolve_completion_model};
use crate::{
    GraphRagConfig, PipelineRunContext, Result, Workflow, WorkflowFunctionOutput,
    operations::community_reports::{
        CommunityReportCallbacks, CommunityReportExtractionConfig, CommunityReportOperationInput,
        community_report_value, community_reports_dataframe, create_community_reports,
        read_claim_context_rows, read_community_input_rows, read_entity_context_rows,
        read_relationship_context_rows,
    },
};

/// Workflow name.
pub const CREATE_COMMUNITY_REPORTS_WORKFLOW: &str = "create_community_reports";

/// Create graph-context community reports.
#[derive(Debug, Clone, Copy, Default)]
pub struct CreateCommunityReportsWorkflow;

#[async_trait]
impl Workflow for CreateCommunityReportsWorkflow {
    fn name(&self) -> &'static str {
        CREATE_COMMUNITY_REPORTS_WORKFLOW
    }

    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut PipelineRunContext,
    ) -> Result<WorkflowFunctionOutput> {
        let entities = read_entity_context_rows(
            &context
                .output_table_provider
                .read_dataframe("entities")
                .await?,
        )?;
        let relationships = read_relationship_context_rows(
            &context
                .output_table_provider
                .read_dataframe("relationships")
                .await?,
        )?;
        let communities = read_community_input_rows(
            &context
                .output_table_provider
                .read_dataframe("communities")
                .await?,
        )?;
        let claims = if config.extract_claims.enabled
            && context.output_table_provider.has("covariates").await?
        {
            read_claim_context_rows(
                &context
                    .output_table_provider
                    .read_dataframe("covariates")
                    .await?,
            )?
        } else {
            Vec::new()
        };

        let model = resolve_completion_model(
            config,
            context,
            &config.community_reports.completion_model_id,
            &config.community_reports.model_instance_name,
            CREATE_COMMUNITY_REPORTS_WORKFLOW,
        )?;
        let encoding_model = resolve_completion_encoding_model(
            config,
            &config.community_reports.completion_model_id,
        );
        let tokenizer = TiktokenTokenizer::new(encoding_model)?;
        let prompt_loader = PromptLoader::new(".");
        let rows = create_community_reports(
            model.as_ref(),
            &prompt_loader,
            &tokenizer,
            CommunityReportOperationInput {
                entities: &entities,
                relationships: &relationships,
                communities: &communities,
                claims: &claims,
            },
            CommunityReportExtractionConfig {
                prompt_path: config.community_reports.graph_prompt.as_deref(),
                max_report_length: config.community_reports.max_length,
                max_input_length: config.community_reports.max_input_length,
                concurrency: config.concurrent_requests.max(1),
            },
            CommunityReportCallbacks {
                progress: &|completed, total| {
                    context.callbacks.progress(
                        CREATE_COMMUNITY_REPORTS_WORKFLOW,
                        completed,
                        Some(total),
                    );
                },
                warning: &|message| {
                    context
                        .callbacks
                        .warning(CREATE_COMMUNITY_REPORTS_WORKFLOW, message);
                },
            },
        )
        .await?;

        context
            .output_table_provider
            .write_dataframe("community_reports", community_reports_dataframe(&rows)?)
            .await?;
        context.stats.report_count = rows.len();

        Ok(WorkflowFunctionOutput {
            result: rows.iter().take(5).map(community_report_value).collect(),
            stop: false,
            input_rows: communities.len(),
            output_rows: rows.len(),
        })
    }
}
