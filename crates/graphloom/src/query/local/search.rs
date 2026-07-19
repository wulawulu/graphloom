//! Local Search completion and streaming orchestration.

use std::{collections::BTreeMap, sync::Arc, time::Instant};

use futures_util::StreamExt;
use graphloom_llm::{ChatMessage, CompletionRequest};
use serde::Serialize;

use super::super::{
    ConversationHistory, LocalQueryRuntime, QueryError, QueryEvent, QueryEventStream, QueryResult,
    Result, SearchMethod,
    result::count_completion_input,
    streaming::{CompletionStreamState, completion_event_stream},
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
    Ok(completion_event_stream(state))
}
