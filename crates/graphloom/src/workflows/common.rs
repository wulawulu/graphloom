//! Shared workflow helpers.

use std::sync::Arc;

use graphloom_llm::{CompletionModel, EmbeddingModel, OpenAiCompletionModel, OpenAiEmbeddingModel};

use crate::{GraphLoomError, GraphRagConfig, PipelineRunContext, Result};

pub(crate) fn resolve_completion_model(
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

pub(crate) fn resolve_completion_encoding_model<'a>(
    config: &'a GraphRagConfig,
    model_id: &str,
) -> &'a str {
    config
        .completion_models
        .get(model_id)
        .and_then(|model| model.encoding_model.as_deref())
        .unwrap_or(config.chunking.encoding_model.as_str())
}

pub(crate) fn resolve_embedding_model(
    config: &GraphRagConfig,
    context: &PipelineRunContext,
    model_id: &str,
    model_instance_name: &str,
    workflow: &'static str,
) -> Result<Arc<dyn EmbeddingModel>> {
    if let Some(model) = context.embedding_models.get(model_id) {
        return Ok(Arc::clone(model));
    }
    let model_config =
        config
            .embedding_models
            .get(model_id)
            .ok_or_else(|| GraphLoomError::InvalidData {
                workflow,
                message: format!("embedding model {model_id} is not configured"),
            })?;
    Ok(Arc::new(OpenAiEmbeddingModel::new(
        model_instance_name,
        model_config.clone(),
        config.concurrent_requests,
    )?))
}

pub(crate) fn resolve_embedding_encoding_model<'a>(
    config: &'a GraphRagConfig,
    model_id: &str,
) -> &'a str {
    config
        .embedding_models
        .get(model_id)
        .and_then(|model| model.encoding_model.as_deref())
        .unwrap_or(config.chunking.encoding_model.as_str())
}
