//! Local Search completion and streaming orchestration.

use std::{collections::BTreeMap, sync::Arc, time::Instant};

use futures_util::StreamExt;
use graphloom_llm::{ChatMessage, CompletionRequest, CompletionStream};
use serde::Serialize;
use tracing::Instrument;

use super::super::{
    ConversationHistory, LocalQueryRuntime, QueryContext, QueryContextText, QueryError, QueryEvent,
    QueryEventStream, QueryResult, Result, SearchMethod,
    explainability::LocalQueryExplainability as QueryExplainabilitySession,
    observability::{
        LlmSpanLatch, QueryInstrumentation, QueryTraceSession, query_error_kind,
        record_stage_error, record_u64, usize_to_u64,
    },
    result::count_completion_input,
    streaming::{CompletionStreamState, completion_event_stream},
};
use crate::{
    explainability::{ExplainabilityEvent, LlmRequestCompleted, LlmRequestStarted},
    observability::{field_name, operation, span_name, status},
};

#[derive(Debug, Serialize)]
struct LocalPromptContext<'a> {
    context_data: &'a str,
    response_type: &'a str,
}

pub(crate) async fn local_search(
    runtime: LocalQueryRuntime,
    query: &str,
    response_type: &str,
    conversation_history: Option<&ConversationHistory>,
    instrumentation: Option<QueryInstrumentation>,
) -> Result<QueryResult> {
    let mut events = local_search_streaming(
        runtime,
        query,
        response_type,
        conversation_history,
        instrumentation,
    )
    .await?;
    while let Some(event) = events.next().await {
        if let QueryEvent::Completed(result) = event? {
            return Ok(result);
        }
    }
    Err(QueryError::QueryCompletion {
        method: SearchMethod::Local,
        operation: "aggregate Local Search stream",
        model: "unknown".to_owned(),
        source: Box::new(graphloom_llm::LlmError::InvalidResponse {
            model_instance: "unknown".to_owned(),
            operation: "query stream",
            message: "stream ended without a completed event".to_owned(),
        }),
    })
}

pub(crate) async fn local_search_streaming(
    runtime: LocalQueryRuntime,
    query: &str,
    response_type: &str,
    conversation_history: Option<&ConversationHistory>,
    instrumentation: Option<QueryInstrumentation>,
) -> Result<QueryEventStream> {
    match prepare_local_stream(
        runtime,
        query,
        response_type,
        conversation_history,
        instrumentation.clone(),
    )
    .await
    {
        Ok(events) => Ok(events),
        Err(error) => {
            if let Some(instrumentation) = instrumentation {
                instrumentation.finish_query_error(&error).await;
            }
            Err(error)
        }
    }
}

async fn prepare_local_stream(
    runtime: LocalQueryRuntime,
    query: &str,
    response_type: &str,
    conversation_history: Option<&ConversationHistory>,
    instrumentation: Option<QueryInstrumentation>,
) -> Result<QueryEventStream> {
    let started = Instant::now();
    let explainability = instrumentation
        .as_ref()
        .and_then(|item| item.local_explainability());
    let trace = instrumentation.as_ref().and_then(|item| item.trace());
    let built = runtime
        .local_context
        .build_explainable(query, conversation_history, explainability, trace)
        .await?;
    let context_text = local_context_text(&built.context)?;
    let context_tokens = usize_to_u64(built.context_tokens);
    let (request, prompt_tokens) = prepare_prompt_stage(
        &runtime,
        query,
        response_type,
        context_text,
        context_tokens,
        trace,
    )
    .await?;
    runtime.callbacks.on_context(&built.context);
    let (provider, llm_latch, llm_started, completion_model_id) =
        prepare_llm_stage(&runtime, request, prompt_tokens, explainability, trace).await?;
    let state = CompletionStreamState {
        provider,
        context: built.context,
        started,
        categories: BTreeMap::from([("build_context".to_owned(), built.usage)]),
        completion_category: "response",
        prompt_tokens,
        tokenizer: Arc::clone(&runtime.local_context.tokenizer),
        callbacks: runtime.callbacks,
        completion_model_id: runtime.completion_model_id,
        method: SearchMethod::Local,
        consume_operation: "consume Local Search completion stream",
        output_count_operation: "count Local Search output tokens",
        output_count_is_context_error: false,
        notify_reduce_end: false,
    };
    let events = completion_event_stream(state);
    Ok(instrument_local_completion_stream(
        events,
        instrumentation,
        llm_latch,
        llm_started,
        completion_model_id,
        built.context_tokens,
    ))
}

async fn prepare_prompt_stage(
    runtime: &LocalQueryRuntime,
    query: &str,
    response_type: &str,
    context_text: &str,
    context_tokens: Option<u64>,
    trace: Option<&QueryTraceSession>,
) -> Result<(CompletionRequest, usize)> {
    let prompt_span = trace.and_then(|trace| {
        trace.clone_root_span().map(|root_span| {
            tracing::info_span!(
                parent: &root_span,
                span_name::QUERY_PROMPT,
                "graphloom.operation" = operation::PROMPT_RENDER,
                "graphloom.context.tokens" = tracing::field::Empty,
                "graphloom.input.tokens" = tracing::field::Empty,
                "graphloom.status" = tracing::field::Empty,
                "graphloom.error.kind" = tracing::field::Empty,
            )
        })
    });
    let record_prompt_span = prompt_span.clone();
    let prompt_future = async {
        let outcome: Result<(CompletionRequest, usize)> = async {
            let rendered = runtime
                .prompt
                .bind(&LocalPromptContext {
                    context_data: context_text,
                    response_type,
                })
                .and_then(|prompt| prompt.render())
                .map_err(|source| QueryError::QueryPrompt {
                    method: SearchMethod::Local,
                    operation: "render Local Search prompt",
                    prompt: "local_search_system_prompt.txt",
                    source: Box::new(source),
                })?;
            let mut request = CompletionRequest::new(vec![
                ChatMessage::system(rendered),
                ChatMessage::user(query),
            ]);
            let prompt_tokens = count_completion_input(
                runtime.local_context.tokenizer.as_ref(),
                &request.messages,
                SearchMethod::Local,
                "count Local Search completion input tokens",
            )?;
            request
                .apply_call_args(&runtime.completion_config.call_args)
                .and_then(|()| {
                    request.stream = Some(true);
                    request.validate()
                })
                .map_err(|source| QueryError::InvalidQueryConfig {
                    method: SearchMethod::Local,
                    operation: "build Local Search completion request",
                    message: source.to_string(),
                })?;
            Ok((request, prompt_tokens))
        }
        .await;
        if let Some(span) = &record_prompt_span {
            match &outcome {
                Ok((_, prompt_tokens)) => {
                    record_u64(span, field_name::CONTEXT_TOKENS, context_tokens);
                    record_u64(span, field_name::INPUT_TOKENS, usize_to_u64(*prompt_tokens));
                    span.record(field_name::STATUS, status::OK);
                }
                Err(error) => record_stage_error(span, query_error_kind(error)),
            }
        }
        outcome
    };
    let (request, prompt_tokens) = match prompt_span {
        Some(span) => prompt_future.instrument(span).await?,
        None => prompt_future.await?,
    };
    Ok((request, prompt_tokens))
}

async fn prepare_llm_stage(
    runtime: &LocalQueryRuntime,
    request: CompletionRequest,
    prompt_tokens: usize,
    explainability: Option<&QueryExplainabilitySession>,
    trace: Option<&QueryTraceSession>,
) -> Result<(CompletionStream, LlmSpanLatch, Instant, String)> {
    let explainability_prompt = request.messages.first().and_then(|message| {
        explainability.and_then(|session| session.content(message.content.as_str()))
    });
    if let Some((session, prompt_tokens)) = explainability.and_then(|session| {
        session
            .usize_to_u64(prompt_tokens)
            .map(|tokens| (session, tokens))
    }) {
        let mut event = LlmRequestStarted::new(runtime.completion_model_id.clone(), prompt_tokens);
        event.prompt = explainability_prompt;
        session
            .emit(
                session.spans().llm(),
                Some(session.spans().root()),
                ExplainabilityEvent::LlmRequestStarted(event),
            )
            .await;
    }
    let llm_started = Instant::now();
    let llm_span = trace.and_then(|trace| {
        trace.clone_root_span().map(|root_span| {
            tracing::info_span!(
                parent: &root_span,
                span_name::LLM_REQUEST,
                "graphloom.operation" = operation::COMPLETION,
                "graphloom.model.instance" = &runtime.completion_model_id,
                "graphloom.model.provider" = runtime.completion_config.provider_type(),
                "graphloom.query.streaming" = true,
                "graphloom.input.tokens" = tracing::field::Empty,
                "graphloom.output.tokens" = tracing::field::Empty,
                "graphloom.status" = tracing::field::Empty,
                "graphloom.error.kind" = tracing::field::Empty,
                "graphloom.elapsed_ms" = tracing::field::Empty,
            )
        })
    });
    let mut llm_latch = LlmSpanLatch::new(llm_span.clone(), llm_started);
    if let Some(span) = &llm_span {
        record_u64(span, field_name::INPUT_TOKENS, usize_to_u64(prompt_tokens));
    }
    let provider_result = async { runtime.completion_model.stream(request).await }
        .instrument(llm_span.clone().unwrap_or_else(tracing::Span::none))
        .await;
    let provider = match provider_result {
        Ok(provider) => provider,
        Err(source) => {
            let error = QueryError::QueryCompletion {
                method: SearchMethod::Local,
                operation: "start Local Search completion stream",
                model: runtime.completion_model_id.clone(),
                source: Box::new(source),
            };
            llm_latch.finish_completion_error();
            return Err(error);
        }
    };
    Ok((
        provider,
        llm_latch,
        llm_started,
        runtime.completion_model_id.clone(),
    ))
}

fn local_context_text(context: &QueryContext) -> Result<&str> {
    match &context.text {
        QueryContextText::Text(value) => Ok(value),
        _ => Err(QueryError::QueryContext {
            method: SearchMethod::Local,
            operation: "read Local Search context text",
            message: "Local Search requires one context string".to_owned(),
        }),
    }
}

struct LocalCompletionState {
    events: QueryEventStream,
    llm: LlmSpanLatch,
    instrumentation: Option<QueryInstrumentation>,
    llm_started: Instant,
    completion_model_id: String,
    context_tokens: Option<u64>,
}

fn instrument_local_completion_stream(
    events: QueryEventStream,
    instrumentation: Option<QueryInstrumentation>,
    llm: LlmSpanLatch,
    llm_started: Instant,
    completion_model_id: String,
    context_tokens: usize,
) -> QueryEventStream {
    let state = LocalCompletionState {
        events,
        llm,
        instrumentation,
        llm_started,
        completion_model_id,
        context_tokens: usize_to_u64(context_tokens),
    };
    Box::pin(futures_util::stream::unfold(
        Some(state),
        next_local_completion_event,
    ))
}

async fn next_local_completion_event(
    state: Option<LocalCompletionState>,
) -> Option<(Result<QueryEvent>, Option<LocalCompletionState>)> {
    let mut state = state?;
    let llm_span = state.llm.span();
    let next_event = match llm_span {
        Some(span) => state.events.next().instrument(span).await,
        None => state.events.next().await,
    };
    match next_event {
        Some(Ok(QueryEvent::Completed(result))) => {
            state.llm.finish_ok(&result);
            if let Some(instrumentation) = &state.instrumentation {
                instrumentation.finish_trace_success(&result, state.context_tokens);
                if let Some(session) = instrumentation.local_explainability() {
                    emit_llm_completed(
                        session,
                        &state.completion_model_id,
                        state.llm_started,
                        &result,
                    )
                    .await;
                }
                instrumentation.finish_explainability_success().await;
            }
            Some((Ok(QueryEvent::Completed(result)), None))
        }
        Some(Ok(event)) => Some((Ok(event), Some(state))),
        Some(Err(error)) => {
            state.llm.finish_completion_error();
            if let Some(instrumentation) = &state.instrumentation {
                instrumentation.finish_query_error(&error).await;
            }
            Some((Err(error), None))
        }
        None => {
            state.llm.finish_completion_error();
            if let Some(instrumentation) = &state.instrumentation {
                instrumentation.finish_stream_ended().await;
            }
            None
        }
    }
}

async fn emit_llm_completed(
    session: &super::super::explainability::LocalQueryExplainability,
    completion_model_id: &str,
    llm_started: Instant,
    result: &QueryResult,
) {
    let Some(usage) = result.usage.categories.get("response") else {
        session.mark_sidecar_failure("missing_llm_usage");
        return;
    };
    let Some(input_tokens) = session.usize_to_u64(usage.prompt_tokens) else {
        return;
    };
    let Some(output_tokens) = session.usize_to_u64(usage.output_tokens) else {
        return;
    };
    let Some(elapsed_ms) = session.duration_millis(llm_started.elapsed()) else {
        return;
    };
    let mut event = LlmRequestCompleted::new(
        completion_model_id.to_owned(),
        input_tokens,
        output_tokens,
        elapsed_ms,
    );
    event.response = session.content(&result.response);
    session
        .emit(
            session.spans().llm(),
            Some(session.spans().root()),
            ExplainabilityEvent::LlmRequestCompleted(event),
        )
        .await;
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Instant,
    };

    use futures_util::stream;

    use super::{LlmSpanLatch, LocalCompletionState, next_local_completion_event};
    use crate::{
        observability::span_name,
        query::{QueryError, QueryEvent, QueryEventStream},
        test_support::tracing_capture,
    };

    #[tokio::test(flavor = "current_thread")]
    async fn test_should_poll_provider_stream_inside_llm_span() {
        let state = Arc::new(Mutex::new(tracing_capture::CaptureState::default()));
        let subscriber = tracing_capture::capture_subscriber(state.clone());
        let _guard = tracing::subscriber::set_default(subscriber);

        let llm_span = tracing::info_span!(
            span_name::LLM_REQUEST,
            "graphloom.operation" = "completion",
            "graphloom.model.instance" = "test-model",
            "graphloom.model.provider" = "openai",
            "graphloom.query.streaming" = true,
            "graphloom.input.tokens" = tracing::field::Empty,
            "graphloom.output.tokens" = tracing::field::Empty,
            "graphloom.status" = tracing::field::Empty,
            "graphloom.error.kind" = tracing::field::Empty,
            "graphloom.elapsed_ms" = tracing::field::Empty,
        );
        let llm_latch = LlmSpanLatch::new(Some(llm_span.clone()), Instant::now());

        let inner: QueryEventStream = Box::pin(stream::unfold(0_u8, |step| async move {
            if step == 0 {
                tracing::info!(
                    name: "graphloom.test.provider.stream.polled",
                    "provider stream polled"
                );
                return Some((
                    Ok::<QueryEvent, QueryError>(QueryEvent::Token("t".to_owned())),
                    1_u8,
                ));
            }
            None
        }));
        let state_holder = LocalCompletionState {
            events: inner,
            llm: llm_latch,
            instrumentation: None,
            llm_started: Instant::now(),
            completion_model_id: "test-model".to_owned(),
            context_tokens: None,
        };

        let (event, _) = next_local_completion_event(Some(state_holder))
            .await
            .expect("event");
        assert!(matches!(event, Ok(QueryEvent::Token(_))));

        let state = state.lock().expect("capture state");
        let llm_captured = state
            .spans
            .iter()
            .find(|span| span.name == span_name::LLM_REQUEST)
            .expect("LLM span");
        let polled = state
            .events
            .iter()
            .find(|event| event.name == "graphloom.test.provider.stream.polled")
            .expect("polled event");
        assert_eq!(
            polled.parent.as_ref(),
            Some(&llm_captured.id),
            "provider poll must run inside the LLM span"
        );
    }
}
