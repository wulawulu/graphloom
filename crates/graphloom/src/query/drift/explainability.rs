//! Shared DRIFT explainability emission helpers.

use std::time::Instant;

use crate::{
    explainability::{
        ExplainabilityEvent, ExplainabilitySpanId, LlmRequestCompleted, LlmRequestStarted,
    },
    query::explainability::DriftQueryExplainability,
};

pub(super) async fn emit_llm_started(
    session: Option<&DriftQueryExplainability>,
    span: &ExplainabilitySpanId,
    parent: &ExplainabilitySpanId,
    model_id: &str,
    prompt_tokens: usize,
    prompt: &str,
) {
    let Some(session) = session else {
        return;
    };
    let Some(prompt_tokens) = session.usize_to_u64(prompt_tokens) else {
        return;
    };
    let mut event = LlmRequestStarted::new(model_id.to_owned(), prompt_tokens);
    event.prompt = session.content(prompt);
    session
        .emit(
            span,
            Some(parent),
            ExplainabilityEvent::LlmRequestStarted(event),
        )
        .await;
}

#[allow(
    clippy::too_many_arguments,
    reason = "the helper mirrors the existing generic LLM event fields plus its trusted span \
              topology"
)]
pub(super) async fn emit_llm_completed(
    session: Option<&DriftQueryExplainability>,
    span: &ExplainabilitySpanId,
    parent: &ExplainabilitySpanId,
    model_id: &str,
    input_tokens: usize,
    output_tokens: usize,
    started: Instant,
    response: &str,
) {
    let Some(session) = session else {
        return;
    };
    let values = (
        session.usize_to_u64(input_tokens),
        session.usize_to_u64(output_tokens),
        session.duration_millis(started.elapsed()),
    );
    let (Some(input_tokens), Some(output_tokens), Some(elapsed_ms)) = values else {
        return;
    };
    let mut event =
        LlmRequestCompleted::new(model_id.to_owned(), input_tokens, output_tokens, elapsed_ms);
    event.response = session.content(response);
    session
        .emit(
            span,
            Some(parent),
            ExplainabilityEvent::LlmRequestCompleted(event),
        )
        .await;
}
