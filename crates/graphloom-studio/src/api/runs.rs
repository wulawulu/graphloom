//! Read-only Explainability Run metadata and history endpoints.

use std::sync::Arc;

use axum::{
    Json,
    extract::{
        Path, Query, State,
        rejection::{PathRejection, QueryRejection},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, SecondsFormat, Utc};
use graphloom::explainability::{
    ExplainabilityQueryMethod, ExplainabilityRun, ExplainabilityRunId, ExplainabilityRunKind,
    ExplainabilityRunStatus, RunListCursor, RunQuery,
};
use serde::{Deserialize, Serialize};

use super::StudioApiState;

const INVALID_RUN_BODY: &str = "invalid explainability run request";
const RUN_NOT_FOUND_BODY: &str = "explainability run not found";
const SERVICE_UNAVAILABLE_BODY: &str = "explainability service unavailable";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RunHistoryQuery {
    kind: Option<ExplainabilityRunKind>,
    status: Option<ExplainabilityRunStatus>,
    query_method: Option<ExplainabilityQueryMethod>,
    limit: Option<u32>,
    before_started_at: Option<String>,
    before_run_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct RunHistoryCursor {
    started_at: String,
    run_id: ExplainabilityRunId,
}

#[derive(Debug, Serialize)]
struct RunHistoryResponse {
    runs: Vec<ExplainabilityRun>,
    next_cursor: Option<RunHistoryCursor>,
}

pub(super) async fn get_run(
    State(state): State<Arc<StudioApiState>>,
    path: Result<Path<String>, PathRejection>,
) -> Response {
    let Ok(Path(raw_run_id)) = path else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_RUN_BODY);
    };
    let Ok(run_id) = raw_run_id.parse::<ExplainabilityRunId>() else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_RUN_BODY);
    };
    match state.store.get_run(&run_id).await {
        Ok(Some(run)) => Json(run).into_response(),
        Ok(None) => fixed_error(StatusCode::NOT_FOUND, RUN_NOT_FOUND_BODY),
        Err(_) => fixed_error(StatusCode::INTERNAL_SERVER_ERROR, SERVICE_UNAVAILABLE_BODY),
    }
}

pub(super) async fn list_runs(
    State(state): State<Arc<StudioApiState>>,
    query: Result<Query<RunHistoryQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(parameters)) = query else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_RUN_BODY);
    };
    let Ok(query) = build_store_query(&parameters) else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_RUN_BODY);
    };
    let requested_limit = query.limit();
    match state.store.list_runs(&query).await {
        Ok(runs) => {
            let next_cursor = if u32::try_from(runs.len()).ok() == Some(requested_limit) {
                runs.last().map(|run| RunHistoryCursor {
                    started_at: format_cursor_timestamp(run.started_at),
                    run_id: run.run_id.clone(),
                })
            } else {
                None
            };
            Json(RunHistoryResponse { runs, next_cursor }).into_response()
        }
        Err(_) => fixed_error(StatusCode::INTERNAL_SERVER_ERROR, SERVICE_UNAVAILABLE_BODY),
    }
}

fn build_store_query(parameters: &RunHistoryQuery) -> Result<RunQuery, RunRequestError> {
    let mut query = RunQuery::new();
    if let Some(kind) = parameters.kind {
        query = query.kind(kind);
    }
    if let Some(status) = parameters.status {
        query = query.status(status);
    }
    if let Some(method) = parameters.query_method {
        query = query.query_method(method);
    }
    match (&parameters.before_started_at, &parameters.before_run_id) {
        (Some(timestamp), Some(run_id)) => {
            query = query.before(RunListCursor::new(
                parse_cursor_timestamp(timestamp)?,
                run_id.parse().map_err(|_| RunRequestError::Invalid)?,
            ));
        }
        (None, None) => {}
        _ => return Err(RunRequestError::Invalid),
    }
    if let Some(limit) = parameters.limit {
        query = query
            .with_limit(limit)
            .map_err(|_| RunRequestError::Invalid)?;
    }
    Ok(query)
}

fn format_cursor_timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn parse_cursor_timestamp(value: &str) -> Result<DateTime<Utc>, RunRequestError> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| RunRequestError::Invalid)?
        .with_timezone(&Utc);
    if format_cursor_timestamp(parsed) != value {
        return Err(RunRequestError::Invalid);
    }
    Ok(parsed)
}

fn fixed_error(status: StatusCode, body: &'static str) -> Response {
    (status, body).into_response()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunRequestError {
    Invalid,
}
