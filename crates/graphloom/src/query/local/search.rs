//! Local Search completion and streaming orchestration.

use std::{collections::BTreeMap, sync::Arc, time::Instant};

use futures_util::StreamExt;
use graphloom_llm::{ChatMessage, CompletionRequest};
use serde::Serialize;

use super::super::{
    ConversationHistory, LocalQueryRuntime, QueryContext, QueryContextText, QueryError, QueryEvent,
    QueryEventStream, QueryExplainabilityOptions, QueryResult, Result, SearchMethod,
    explainability::QueryExplainabilitySession,
    result::count_completion_input,
    streaming::{CompletionStreamState, completion_event_stream},
};
use crate::explainability::{ExplainabilityEvent, LlmRequestCompleted, LlmRequestStarted};

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
    explainability: Option<&QueryExplainabilityOptions>,
) -> Result<QueryResult> {
    let mut events = local_search_streaming(
        runtime,
        query,
        response_type,
        conversation_history,
        explainability,
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
    explainability: Option<&QueryExplainabilityOptions>,
) -> Result<QueryEventStream> {
    let session = explainability.map(|options| Arc::new(QueryExplainabilitySession::new(options)));
    if let Some(session) = &session {
        session.start(query).await;
    }
    match prepare_local_stream(
        runtime,
        query,
        response_type,
        conversation_history,
        session.clone(),
    )
    .await
    {
        Ok(events) => Ok(events),
        Err(error) => {
            if let Some(session) = &session {
                session.finish_query_error(&error).await;
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
    session: Option<Arc<QueryExplainabilitySession>>,
) -> Result<QueryEventStream> {
    let started = Instant::now();
    let built = runtime
        .local_context
        .build_explainable(query, conversation_history, session.as_deref())
        .await?;
    let context_text = local_context_text(&built.context)?;
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
    let explainability_prompt = session
        .as_ref()
        .and_then(|session| session.content(&rendered));
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
    runtime.callbacks.on_context(&built.context);
    if let Some((session, prompt_tokens)) = session.as_ref().and_then(|session| {
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
    let provider = runtime
        .completion_model
        .stream(request)
        .await
        .map_err(|source| QueryError::QueryCompletion {
            method: SearchMethod::Local,
            operation: "start Local Search completion stream",
            model: runtime.completion_model_id.clone(),
            source: Box::new(source),
        })?;
    let completion_model_id = runtime.completion_model_id.clone();
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
    Ok(match session {
        Some(session) => {
            explainable_completion_stream(events, session, llm_started, completion_model_id)
        }
        None => events,
    })
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

struct ExplainableCompletionState {
    events: QueryEventStream,
    session: Arc<QueryExplainabilitySession>,
    llm_started: Instant,
    completion_model_id: String,
}

fn explainable_completion_stream(
    events: QueryEventStream,
    session: Arc<QueryExplainabilitySession>,
    llm_started: Instant,
    completion_model_id: String,
) -> QueryEventStream {
    let state = ExplainableCompletionState {
        events,
        session,
        llm_started,
        completion_model_id,
    };
    Box::pin(futures_util::stream::unfold(
        Some(state),
        next_explainable_event,
    ))
}

async fn next_explainable_event(
    state: Option<ExplainableCompletionState>,
) -> Option<(Result<QueryEvent>, Option<ExplainableCompletionState>)> {
    let mut state = state?;
    match state.events.next().await {
        Some(Ok(QueryEvent::Completed(result))) => {
            emit_llm_completed(
                &state.session,
                &state.completion_model_id,
                state.llm_started,
                &result,
            )
            .await;
            state.session.finish_success().await;
            Some((Ok(QueryEvent::Completed(result)), None))
        }
        Some(Ok(event)) => Some((Ok(event), Some(state))),
        Some(Err(error)) => {
            state.session.finish_query_error(&error).await;
            Some((Err(error), None))
        }
        None => {
            state.session.finish_stream_ended().await;
            None
        }
    }
}

async fn emit_llm_completed(
    session: &QueryExplainabilitySession,
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
