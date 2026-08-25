//! Server-Sent Events transport for transient live Query answers.

use std::{fmt, sync::Arc, time::Duration};

use axum::{
    extract::{Path, State, rejection::PathRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::{Stream, StreamExt, stream};
use graphloom::explainability::ExplainabilityRunId;
use thiserror::Error;

use super::{
    StudioApiState,
    answer_live::{
        LiveAnswerStatus, QueryAnswerEvent, QueryAnswerRecvError, QueryAnswerSubscription,
    },
};

const SSE_EVENT_NAME: &str = "answer";
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);
const LAST_EVENT_ID: &str = "last-event-id";
const INVALID_REQUEST_BODY: &str = "invalid query answer event request";
const ANSWER_NOT_RETAINED_BODY: &str = "query answer stream is no longer available";

pub(super) async fn get_query_answer_events(
    State(state): State<Arc<StudioApiState>>,
    path: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
) -> Response {
    let Ok(Path(raw_run_id)) = path else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_REQUEST_BODY);
    };
    let Ok(run_id) = raw_run_id.parse::<ExplainabilityRunId>() else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_REQUEST_BODY);
    };
    if !valid_last_event_id(&headers) {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_REQUEST_BODY);
    }
    let Some(subscription) = state.query_answer_live.subscribe(&run_id) else {
        return fixed_error(StatusCode::GONE, ANSWER_NOT_RETAINED_BODY);
    };
    let event_stream = answer_stream(subscription).map(|event| event.and_then(answer_event_to_sse));
    let mut response = Sse::new(event_stream)
        .keep_alive(KeepAlive::new().interval(KEEP_ALIVE_INTERVAL))
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

fn valid_last_event_id(headers: &HeaderMap) -> bool {
    let Some(value) = headers.get(LAST_EVENT_ID) else {
        return true;
    };
    let Ok(value) = value.to_str() else {
        return false;
    };
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn fixed_error(status: StatusCode, body: &'static str) -> Response {
    (status, body).into_response()
}

fn answer_stream(
    subscription: QueryAnswerSubscription,
) -> impl Stream<Item = Result<QueryAnswerEvent, QueryAnswerSseError>> + Send {
    stream::unfold(
        AnswerStreamState {
            subscription,
            initial: true,
            done: false,
        },
        |mut state| async move {
            if state.done {
                return None;
            }
            let event = if state.initial {
                state.initial = false;
                state.subscription.initial_snapshot()
            } else {
                match state.subscription.recv().await {
                    Ok(event) => event,
                    Err(QueryAnswerRecvError::Closed) => return None,
                }
            };
            state.done = event_is_terminal(&event);
            Some((Ok(event), state))
        },
    )
}

fn event_is_terminal(event: &QueryAnswerEvent) -> bool {
    matches!(
        event,
        QueryAnswerEvent::Completed { .. }
            | QueryAnswerEvent::Failed { .. }
            | QueryAnswerEvent::Snapshot {
                status: LiveAnswerStatus::Completed | LiveAnswerStatus::Failed,
                ..
            }
    )
}

fn answer_event_to_sse(event: QueryAnswerEvent) -> Result<Event, QueryAnswerSseError> {
    Event::default()
        .event(SSE_EVENT_NAME)
        .id(event.sequence().to_string())
        .json_data(event)
        .map_err(|_| QueryAnswerSseError::Serialization)
}

struct AnswerStreamState {
    subscription: QueryAnswerSubscription,
    initial: bool,
    done: bool,
}

impl fmt::Debug for AnswerStreamState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AnswerStreamState { .. }")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
enum QueryAnswerSseError {
    #[error("query answer SSE serialization failed")]
    Serialization,
}
