//! Claim covariate extraction workflow.

use std::{path::Path, sync::Arc};

use async_trait::async_trait;
use graphloom_llm::{
    ChatMessage, CompletionModel, CompletionRequest, DefaultPrompt, OpenAiCompletionModel,
    PromptLoader, parse_claim_tuples,
};
use polars_core::prelude::*;
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{common::string_value, input_documents::usize_to_i64};
use crate::{
    GraphLoomError, GraphRagConfig, PipelineRunContext, Result, Workflow, WorkflowFunctionOutput,
};

/// Workflow name.
pub const EXTRACT_COVARIATES_WORKFLOW: &str = "extract_covariates";

const DEFAULT_CLAIM_ENTITY_TYPES: &[&str] = &["organization", "person", "geo", "event"];

/// Extract claim covariates from text units.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExtractCovariatesWorkflow;

#[async_trait]
impl Workflow for ExtractCovariatesWorkflow {
    fn name(&self) -> &'static str {
        EXTRACT_COVARIATES_WORKFLOW
    }

    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut PipelineRunContext,
    ) -> Result<WorkflowFunctionOutput> {
        if !config.extract_claims.enabled {
            return Ok(WorkflowFunctionOutput {
                result: Vec::new(),
                stop: false,
                input_rows: 0,
                output_rows: 0,
            });
        }

        let text_units = read_text_unit_inputs(
            &context
                .output_table_provider
                .read_dataframe("text_units")
                .await?,
        )?;
        let model = resolve_completion_model(
            config,
            context,
            &config.extract_claims.completion_model_id,
            &config.extract_claims.model_instance_name,
            EXTRACT_COVARIATES_WORKFLOW,
        )?;
        let prompt_loader = PromptLoader::new(".");
        let entity_types = default_claim_entity_types();
        let mut rows = Vec::new();

        for (index, text_unit) in text_units.iter().enumerate() {
            let claims = extract_claims_for_text_unit(
                model.as_ref(),
                &prompt_loader,
                config.extract_claims.prompt.as_deref(),
                &config.extract_claims.description,
                &entity_types,
                text_unit,
                config.extract_claims.max_gleanings,
            )
            .await?;
            for claim in claims {
                let row = CovariateRow {
                    id: Uuid::new_v4().to_string(),
                    human_readable_id: usize_to_i64(rows.len(), EXTRACT_COVARIATES_WORKFLOW)?,
                    covariate_type: "claim".to_owned(),
                    claim_type: claim.claim_type,
                    description: claim.description,
                    subject_id: claim.subject_id,
                    object_id: claim.object_id,
                    status: claim.status,
                    start_date: claim.start_date,
                    end_date: claim.end_date,
                    source_text: claim.source_text,
                    text_unit_id: text_unit.id.clone(),
                };
                rows.push(row);
            }
            context.callbacks.progress(
                EXTRACT_COVARIATES_WORKFLOW,
                index.saturating_add(1),
                Some(text_units.len()),
            );
        }

        context
            .output_table_provider
            .write_dataframe("covariates", covariates_dataframe(&rows)?)
            .await?;

        Ok(WorkflowFunctionOutput {
            result: rows.iter().take(5).map(covariate_value).collect(),
            stop: false,
            input_rows: text_units.len(),
            output_rows: rows.len(),
        })
    }
}

#[derive(Debug, Clone)]
struct TextUnitInput {
    id: String,
    text: String,
}

#[derive(Debug, Clone)]
struct CovariateRow {
    id: String,
    human_readable_id: i64,
    covariate_type: String,
    claim_type: Option<String>,
    description: Option<String>,
    subject_id: Option<String>,
    object_id: Option<String>,
    status: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
    source_text: Option<String>,
    text_unit_id: String,
}

#[derive(Debug, Serialize)]
struct ClaimPromptValues<'a> {
    input_text: &'a str,
    entity_specs: &'a [String],
    claim_description: &'a str,
}

fn resolve_completion_model(
    config: &GraphRagConfig,
    context: &PipelineRunContext,
    model_id: &str,
    model_instance_name: &str,
    workflow: &'static str,
) -> Result<Arc<dyn CompletionModel>> {
    if let Some(model) = context.completion_models.get(model_id) {
        return Ok(Arc::clone(model));
    }
    let model_config =
        config
            .completion_models
            .get(model_id)
            .ok_or_else(|| GraphLoomError::InvalidData {
                workflow,
                message: format!("completion model {model_id} is not configured"),
            })?;
    Ok(Arc::new(OpenAiCompletionModel::new(
        model_instance_name,
        model_config.clone(),
        config.concurrent_requests,
    )?))
}

async fn extract_claims_for_text_unit(
    model: &dyn CompletionModel,
    prompt_loader: &PromptLoader,
    prompt_path: Option<&str>,
    claim_description: &str,
    entity_types: &[String],
    text_unit: &TextUnitInput,
    max_gleanings: usize,
) -> Result<Vec<graphloom_llm::ClaimRecord>> {
    let initial_prompt = render_claim_prompt(
        prompt_loader,
        prompt_path,
        &text_unit.text,
        entity_types,
        claim_description,
    )
    .await?;
    let mut messages = vec![ChatMessage::user(initial_prompt)];
    let initial = model
        .complete(CompletionRequest {
            messages: messages.clone(),
            temperature: None,
            top_p: None,
            max_tokens: None,
            response_format: None,
            cache_namespace: None,
        })
        .await?
        .content;
    messages.push(ChatMessage::assistant(initial.clone()));

    for glean_index in 0..max_gleanings {
        messages.push(ChatMessage::user(
            render_builtin_prompt(prompt_loader, DefaultPrompt::ExtractClaimsContinue).await?,
        ));
        let extension = model
            .complete(CompletionRequest {
                messages: messages.clone(),
                temperature: None,
                top_p: None,
                max_tokens: None,
                response_format: None,
                cache_namespace: None,
            })
            .await?
            .content;
        messages.push(ChatMessage::assistant(extension));

        if glean_index >= max_gleanings.saturating_sub(1) {
            break;
        }

        messages.push(ChatMessage::user(
            render_builtin_prompt(prompt_loader, DefaultPrompt::ExtractClaimsLoop).await?,
        ));
        let response = model
            .complete(CompletionRequest {
                messages: messages.clone(),
                temperature: None,
                top_p: None,
                max_tokens: None,
                response_format: None,
                cache_namespace: None,
            })
            .await?
            .content;
        if response != "Y" {
            break;
        }
    }

    Ok(parse_claim_tuples(&initial))
}

async fn render_claim_prompt(
    prompt_loader: &PromptLoader,
    prompt_path: Option<&str>,
    input_text: &str,
    entity_specs: &[String],
    claim_description: &str,
) -> Result<String> {
    prompt_loader
        .render(
            DefaultPrompt::ExtractClaims,
            prompt_path.map(Path::new),
            &ClaimPromptValues {
                input_text,
                entity_specs,
                claim_description,
            },
        )
        .await
        .map_err(GraphLoomError::from)
}

async fn render_builtin_prompt(
    prompt_loader: &PromptLoader,
    prompt: DefaultPrompt,
) -> Result<String> {
    prompt_loader
        .render(prompt, None, &json!({}))
        .await
        .map_err(GraphLoomError::from)
}

fn default_claim_entity_types() -> Vec<String> {
    DEFAULT_CLAIM_ENTITY_TYPES
        .iter()
        .map(|entity_type| (*entity_type).to_owned())
        .collect()
}

fn read_text_unit_inputs(dataframe: &DataFrame) -> Result<Vec<TextUnitInput>> {
    let ids = dataframe.column("id")?.str()?;
    let texts = dataframe.column("text")?.str()?;
    let mut rows = Vec::with_capacity(dataframe.height());
    for index in 0..dataframe.height() {
        rows.push(TextUnitInput {
            id: string_value(ids.get(index), "id", EXTRACT_COVARIATES_WORKFLOW)?,
            text: string_value(texts.get(index), "text", EXTRACT_COVARIATES_WORKFLOW)?,
        });
    }
    Ok(rows)
}

fn covariates_dataframe(rows: &[CovariateRow]) -> Result<DataFrame> {
    Ok(df!(
        "id" => rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        "human_readable_id" => rows.iter().map(|row| row.human_readable_id).collect::<Vec<_>>(),
        "covariate_type" => rows.iter().map(|row| row.covariate_type.as_str()).collect::<Vec<_>>(),
        "type" => rows.iter().map(|row| row.claim_type.as_deref()).collect::<Vec<_>>(),
        "description" => rows.iter().map(|row| row.description.as_deref()).collect::<Vec<_>>(),
        "subject_id" => rows.iter().map(|row| row.subject_id.as_deref()).collect::<Vec<_>>(),
        "object_id" => rows.iter().map(|row| row.object_id.as_deref()).collect::<Vec<_>>(),
        "status" => rows.iter().map(|row| row.status.as_deref()).collect::<Vec<_>>(),
        "start_date" => rows.iter().map(|row| row.start_date.as_deref()).collect::<Vec<_>>(),
        "end_date" => rows.iter().map(|row| row.end_date.as_deref()).collect::<Vec<_>>(),
        "source_text" => rows.iter().map(|row| row.source_text.as_deref()).collect::<Vec<_>>(),
        "text_unit_id" => rows.iter().map(|row| row.text_unit_id.as_str()).collect::<Vec<_>>(),
    )?)
}

fn covariate_value(row: &CovariateRow) -> Value {
    json!({
        "id": row.id,
        "human_readable_id": row.human_readable_id,
        "covariate_type": row.covariate_type,
        "type": row.claim_type,
        "description": row.description,
        "subject_id": row.subject_id,
        "object_id": row.object_id,
        "status": row.status,
        "start_date": row.start_date,
        "end_date": row.end_date,
        "source_text": row.source_text,
        "text_unit_id": row.text_unit_id,
    })
}
