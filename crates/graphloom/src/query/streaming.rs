//! Shared provider-stream consumption for Query completions.

use std::{collections::BTreeMap, sync::Arc, time::Instant};

use futures_util::{StreamExt, stream};
use graphloom_llm::{CompletionStream, Tokenizer};

use super::{
    QueryCallbacks, QueryContext, QueryError, QueryEvent, QueryEventStream, QueryResult,
    QueryUsage, QueryUsageCategory, Result, SearchMethod,
};

pub(crate) struct CompletionStreamState {
    pub(crate) provider: CompletionStream,
    pub(crate) context: QueryContext,
    pub(crate) started: Instant,
    pub(crate) categories: BTreeMap<String, QueryUsageCategory>,
    pub(crate) completion_category: &'static str,
    pub(crate) prompt_tokens: usize,
    pub(crate) tokenizer: Arc<dyn Tokenizer>,
    pub(crate) callbacks: Arc<dyn QueryCallbacks>,
    pub(crate) completion_model_id: String,
    pub(crate) method: SearchMethod,
    pub(crate) consume_operation: &'static str,
    pub(crate) output_count_operation: &'static str,
    pub(crate) output_count_is_context_error: bool,
    pub(crate) notify_reduce_end: bool,
}

struct ActiveCompletionStream {
    configured: CompletionStreamState,
    response: String,
    context_pending: bool,
}

pub(crate) fn completion_event_stream(state: CompletionStreamState) -> QueryEventStream {
    let active = ActiveCompletionStream {
        configured: state,
        response: String::new(),
        context_pending: true,
    };
    Box::pin(stream::unfold(Some(active), next_event))
}

async fn next_event(
    state: Option<ActiveCompletionStream>,
) -> Option<(Result<QueryEvent>, Option<ActiveCompletionStream>)> {
    let mut state = state?;
    if state.context_pending {
        state.context_pending = false;
        return Some((
            Ok(QueryEvent::Context(state.configured.context.clone())),
            Some(state),
        ));
    }
    loop {
        match state.configured.provider.next().await {
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
                state.configured.callbacks.on_llm_new_token(content);
                return Some((Ok(QueryEvent::Token(content.to_owned())), Some(state)));
            }
            Some(Err(source)) => {
                let error = QueryError::QueryCompletion {
                    method: state.configured.method,
                    operation: state.configured.consume_operation,
                    model: state.configured.completion_model_id,
                    source: Box::new(source),
                };
                return Some((Err(error), None));
            }
            None => return Some(completed_event(state)),
        }
    }
}

fn completed_event(
    mut state: ActiveCompletionStream,
) -> (Result<QueryEvent>, Option<ActiveCompletionStream>) {
    let output_tokens = match state.configured.tokenizer.count(&state.response) {
        Ok(value) => value,
        Err(source) => {
            let error = if state.configured.output_count_is_context_error {
                QueryError::QueryContext {
                    method: state.configured.method,
                    operation: state.configured.output_count_operation,
                    message: source.to_string(),
                }
            } else {
                QueryError::QueryCompletion {
                    method: state.configured.method,
                    operation: state.configured.output_count_operation,
                    model: state.configured.completion_model_id,
                    source: Box::new(source),
                }
            };
            return (Err(error), None);
        }
    };
    if state.configured.notify_reduce_end {
        state
            .configured
            .callbacks
            .on_reduce_response_end(&state.response);
    }
    state.configured.categories.insert(
        state.configured.completion_category.to_owned(),
        QueryUsageCategory {
            llm_calls: 1,
            prompt_tokens: state.configured.prompt_tokens,
            output_tokens,
        },
    );
    let result = QueryResult {
        response: state.response,
        context: state.configured.context,
        elapsed: state.configured.started.elapsed(),
        usage: QueryUsage::from_categories(state.configured.categories),
    };
    (Ok(QueryEvent::Completed(result)), None)
}
