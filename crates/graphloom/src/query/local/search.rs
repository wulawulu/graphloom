//! Local Search completion and streaming orchestration.

use std::{collections::BTreeMap, sync::Arc, time::Instant};

use futures_util::{StreamExt, stream};
use graphloom_llm::{ChatMessage, CompletionRequest, CompletionStream, Tokenizer};
use serde::Serialize;

use super::super::{
    ConversationHistory, LocalQueryRuntime, QueryError, QueryEvent, QueryEventStream, QueryResult,
    QueryUsage, QueryUsageCategory, Result, SearchMethod,
};

#[derive(Debug, Serialize)]
struct LocalPromptContext<'a> {
    context_data: &'a str,
    response_type: &'a str,
}

struct LocalStreamState {
    provider: CompletionStream,
    context: super::super::QueryContext,
    response: String,
    started: Instant,
    prompt_tokens: usize,
    build_context_usage: QueryUsageCategory,
    tokenizer: Arc<dyn Tokenizer>,
    callbacks: Arc<dyn super::super::QueryCallbacks>,
    completion_model_id: String,
    phase: LocalStreamPhase,
}

#[derive(Debug, Clone, Copy)]
enum LocalStreamPhase {
    Context,
    Tokens,
    Completed,
}

pub(crate) async fn local_search(
    runtime: LocalQueryRuntime,
    query: &str,
    response_type: &str,
    conversation_history: Option<&ConversationHistory>,
) -> Result<QueryResult> {
    let mut events =
        local_search_streaming(runtime, query, response_type, conversation_history).await?;
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
) -> Result<QueryEventStream> {
    let started = Instant::now();
    let built = runtime
        .local_context
        .build(query, conversation_history)
        .await?;
    let context_text = match &built.context.text {
        super::super::QueryContextText::Text(value) => value.as_str(),
        _ => {
            return Err(QueryError::QueryContext {
                method: SearchMethod::Local,
                operation: "read Local Search context text",
                message: "Local Search requires one context string".to_owned(),
            });
        }
    };
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
    let prompt_tokens = runtime
        .local_context
        .tokenizer
        .count(&rendered)
        .map_err(|source| QueryError::QueryContext {
            method: SearchMethod::Local,
            operation: "count Local Search prompt tokens",
            message: source.to_string(),
        })?;
    let mut request = CompletionRequest::new(vec![
        ChatMessage::system(rendered),
        ChatMessage::user(query),
    ]);
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
    let state = LocalStreamState {
        provider,
        context: built.context,
        response: String::new(),
        started,
        prompt_tokens,
        build_context_usage: built.usage,
        tokenizer: Arc::clone(&runtime.local_context.tokenizer),
        callbacks: runtime.callbacks,
        completion_model_id: runtime.completion_model_id,
        phase: LocalStreamPhase::Context,
    };
    Ok(Box::pin(stream::unfold(Some(state), |state| async move {
        next_event(state).await
    })))
}

async fn next_event(
    state: Option<LocalStreamState>,
) -> Option<(Result<QueryEvent>, Option<LocalStreamState>)> {
    let mut state = state?;
    match state.phase {
        LocalStreamPhase::Context => {
            state.phase = LocalStreamPhase::Tokens;
            Some((Ok(QueryEvent::Context(state.context.clone())), Some(state)))
        }
        LocalStreamPhase::Tokens => loop {
            match state.provider.next().await {
                Some(Ok(chunk)) => {
                    let content = chunk
                        .choices
                        .first()
                        .and_then(|choice| choice.delta.content.as_deref())
                        .unwrap_or_default();
                    if content.is_empty() {
                        continue;
                    }
                    state.response.push_str(content);
                    state.callbacks.on_llm_new_token(content);
                    return Some((Ok(QueryEvent::Token(content.to_owned())), Some(state)));
                }
                Some(Err(source)) => {
                    return Some((
                        Err(QueryError::QueryCompletion {
                            method: SearchMethod::Local,
                            operation: "consume Local Search completion stream",
                            model: state.completion_model_id.clone(),
                            source: Box::new(source),
                        }),
                        None,
                    ));
                }
                None => {
                    state.phase = LocalStreamPhase::Completed;
                    return Some(completed_event(state));
                }
            }
        },
        LocalStreamPhase::Completed => Some(completed_event(state)),
    }
}

fn completed_event(state: LocalStreamState) -> (Result<QueryEvent>, Option<LocalStreamState>) {
    let output_tokens = match state.tokenizer.count(&state.response) {
        Ok(value) => value,
        Err(source) => {
            return (
                Err(QueryError::QueryCompletion {
                    method: SearchMethod::Local,
                    operation: "count Local Search output tokens",
                    model: state.completion_model_id,
                    source: Box::new(source),
                }),
                None,
            );
        }
    };
    let usage = QueryUsage::from_categories(BTreeMap::from([
        ("build_context".to_owned(), state.build_context_usage),
        (
            "response".to_owned(),
            QueryUsageCategory {
                llm_calls: 1,
                prompt_tokens: state.prompt_tokens,
                output_tokens,
            },
        ),
    ]));
    let result = QueryResult {
        response: state.response,
        context: state.context,
        elapsed: state.started.elapsed(),
        usage,
    };
    (Ok(QueryEvent::Completed(result)), None)
}
