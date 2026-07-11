//! Shared workflow helpers.

use std::sync::Arc;

use graphloom_llm::{CompletionModel, EmbeddingModel};

use crate::{GraphRagConfig, IndexPipelineContext, Result};

pub(crate) fn resolve_completion_model(
    context: &IndexPipelineContext,
    model_id: &str,
    workflow: &'static str,
) -> Result<Arc<dyn CompletionModel>> {
    context.models().completion_for_workflow(model_id, workflow)
}

pub(crate) fn resolve_completion_encoding_model<'a>(
    config: &'a GraphRagConfig,
    model_id: &str,
) -> &'a str {
    crate::config::effective_completion_encoding(config, model_id)
}

pub(crate) fn resolve_embedding_model(
    context: &IndexPipelineContext,
    model_id: &str,
    workflow: &'static str,
) -> Result<Arc<dyn EmbeddingModel>> {
    context.models().embedding_for_workflow(model_id, workflow)
}

pub(crate) fn resolve_embedding_encoding_model<'a>(
    config: &'a GraphRagConfig,
    model_id: &str,
) -> &'a str {
    crate::config::effective_embedding_encoding(config, model_id)
}
