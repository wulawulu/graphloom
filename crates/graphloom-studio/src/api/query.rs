//! Local Query admission and detached lifecycle orchestration.

use std::{fmt, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use graphloom::{
    GraphRagConfig,
    explainability::{
        ExplainabilityContentMode, ExplainabilityQueryMethod, ExplainabilityRun,
        ExplainabilityRunId, ExplainabilityRunKind, ExplainabilityRunStatus, RunCompletion,
        StoreExplainabilityOptions, StoreExplainabilityRecorder,
    },
    query::{QueryExplainabilityOptions, QueryOptions, SearchMethod},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, oneshot};

use super::{
    StudioApiState,
    query_result::{QueryExecutionResult, QueryResultConversionError, QueryResultRegistry},
};

const MAX_QUERY_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_TYPE_BYTES: usize = 256;
const DEFAULT_RESPONSE_TYPE: &str = "Multiple Paragraphs";

const INVALID_QUERY_BODY: &str = "invalid Studio query request";
const UNSUPPORTED_METHOD_BODY: &str = "query method is not yet supported by Studio explainability";
const TOO_MANY_QUERIES_BODY: &str = "too many active Studio queries";
const QUERY_UNAVAILABLE_BODY: &str = "Studio query service unavailable";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StartQueryRequest {
    query: String,
    #[serde(default = "default_query_method")]
    method: ExplainabilityQueryMethod,
    #[serde(default)]
    content_mode: ExplainabilityContentMode,
    #[serde(default = "default_response_type")]
    response_type: String,
}

impl fmt::Debug for StartQueryRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StartQueryRequest { .. }")
    }
}

#[derive(Debug, Serialize)]
struct StartQueryResponse {
    run_id: ExplainabilityRunId,
    run_url: String,
    events_url: String,
    result_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum QueryRunnerError {
    #[error("Studio Query execution failed")]
    Failed,
    #[error("Studio Query result materialization failed")]
    ResultMaterialization,
}

#[async_trait]
pub(super) trait QueryRunner: Send + Sync + fmt::Debug {
    async fn run(&self, options: QueryOptions) -> Result<QueryExecutionResult, QueryRunnerError>;
}

pub(super) struct GraphLoomQueryRunner {
    config: GraphRagConfig,
}

impl GraphLoomQueryRunner {
    pub(super) fn new(config: GraphRagConfig) -> Self {
        Self { config }
    }
}

impl fmt::Debug for GraphLoomQueryRunner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GraphLoomQueryRunner { .. }")
    }
}

#[async_trait]
impl QueryRunner for GraphLoomQueryRunner {
    async fn run(&self, options: QueryOptions) -> Result<QueryExecutionResult, QueryRunnerError> {
        let config = self.config.clone();
        let result = Box::pin(graphloom::api::query(config, options))
            .await
            .map_err(|_| QueryRunnerError::Failed)?;
        QueryExecutionResult::try_from(result)
            .map_err(|QueryResultConversionError::Failed| QueryRunnerError::ResultMaterialization)
    }
}

pub(super) async fn start_query(
    State(state): State<Arc<StudioApiState>>,
    payload: Result<Json<StartQueryRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = payload else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_QUERY_BODY);
    };
    if !valid_request(&request) {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_QUERY_BODY);
    }
    if !matches!(request.method, ExplainabilityQueryMethod::Local) {
        return fixed_error(StatusCode::UNPROCESSABLE_ENTITY, UNSUPPORTED_METHOD_BODY);
    }
    let Ok(permit) = Arc::clone(&state.query_permits).try_acquire_owned() else {
        return fixed_error(StatusCode::TOO_MANY_REQUESTS, TOO_MANY_QUERIES_BODY);
    };

    let run_id = ExplainabilityRunId::generate();
    let run = make_run(&run_id, &request);
    let (ready_sender, ready_receiver) = oneshot::channel();
    let execution = QueryExecution {
        project_root: state.project_root.clone(),
        store: Arc::clone(&state.store),
        live_hub: Arc::clone(&state.live_hub),
        query_runner: Arc::clone(&state.query_runner),
        query_results: Arc::clone(&state.query_results),
        request,
        run_id: run_id.clone(),
        run,
        permit,
        ready_sender,
    };
    drop(tokio::spawn(execute_query(execution)));

    match ready_receiver.await {
        Ok(Ok(())) => accepted_response(run_id),
        Ok(Err(QueryStartupError::Unavailable)) | Err(_) => {
            fixed_error(StatusCode::INTERNAL_SERVER_ERROR, QUERY_UNAVAILABLE_BODY)
        }
    }
}

fn valid_request(request: &StartQueryRequest) -> bool {
    !request.query.is_empty()
        && request.query.len() <= MAX_QUERY_BYTES
        && !request.response_type.is_empty()
        && request.response_type.len() <= MAX_RESPONSE_TYPE_BYTES
}

fn make_run(run_id: &ExplainabilityRunId, request: &StartQueryRequest) -> ExplainabilityRun {
    let mut run = ExplainabilityRun::new(run_id.clone(), ExplainabilityRunKind::Query, Utc::now());
    run.status = ExplainabilityRunStatus::Running;
    run.query_method = Some(ExplainabilityQueryMethod::Local);
    run.query = request
        .content_mode
        .includes_content()
        .then(|| request.query.clone());
    run
}

fn accepted_response(run_id: ExplainabilityRunId) -> Response {
    let run_url = format!("/api/explainability/runs/{run_id}");
    let events_url = format!("{run_url}/events");
    let result_url = format!("/api/query/{run_id}/result");
    let Ok(location) = HeaderValue::from_str(&run_url) else {
        return fixed_error(StatusCode::INTERNAL_SERVER_ERROR, QUERY_UNAVAILABLE_BODY);
    };
    let mut response = (
        StatusCode::ACCEPTED,
        Json(StartQueryResponse {
            run_id,
            run_url,
            events_url,
            result_url,
        }),
    )
        .into_response();
    response.headers_mut().insert(header::LOCATION, location);
    response
}

fn fixed_error(status: StatusCode, body: &'static str) -> Response {
    (status, body).into_response()
}

const fn default_query_method() -> ExplainabilityQueryMethod {
    ExplainabilityQueryMethod::Local
}

fn default_response_type() -> String {
    DEFAULT_RESPONSE_TYPE.to_owned()
}

struct QueryExecution {
    project_root: PathBuf,
    store: Arc<dyn graphloom::explainability::ExplainabilityStore>,
    live_hub: Arc<graphloom::explainability::ExplainabilityLiveHub>,
    query_runner: Arc<dyn QueryRunner>,
    query_results: Arc<QueryResultRegistry>,
    request: StartQueryRequest,
    run_id: ExplainabilityRunId,
    run: ExplainabilityRun,
    permit: OwnedSemaphorePermit,
    ready_sender: oneshot::Sender<Result<(), QueryStartupError>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryStartupError {
    Unavailable,
}

async fn execute_query(execution: QueryExecution) {
    let QueryExecution {
        project_root,
        store,
        live_hub,
        query_runner,
        query_results,
        request,
        run_id,
        run,
        permit: _permit,
        ready_sender,
    } = execution;
    let Ok(recorder) = StoreExplainabilityRecorder::new_with_live_hub(
        store,
        live_hub,
        StoreExplainabilityOptions::new(),
    ) else {
        let _send_result = ready_sender.send(Err(QueryStartupError::Unavailable));
        return;
    };
    if recorder.create_run(run).await.is_err() {
        let _send_result = ready_sender.send(Err(QueryStartupError::Unavailable));
        let _shutdown_result = recorder.shutdown().await;
        return;
    }

    let _send_result = ready_sender.send(Ok(()));
    let mut options = QueryOptions::new(project_root, request.query, SearchMethod::Local);
    options.response_type = request.response_type;
    options = options.with_explainability(QueryExplainabilityOptions::new(
        run_id.clone(),
        request.content_mode,
        recorder.sink(),
    ));
    match query_runner.run(options).await {
        Ok(result) => {
            let studio_result = result.with_run_id(run_id.clone());
            query_results.insert(studio_result).await;
            let completion_succeeded = match RunCompletion::new(
                run_id.clone(),
                ExplainabilityRunStatus::Completed,
                Utc::now(),
            ) {
                Ok(completion) => recorder.complete_run(completion).await.is_ok(),
                Err(_) => false,
            };
            if !completion_succeeded {
                query_results.remove(&run_id).await;
            }
        }
        Err(QueryRunnerError::Failed) => {
            if let Ok(completion) =
                RunCompletion::new(run_id, ExplainabilityRunStatus::Failed, Utc::now())
            {
                let _completion_result = recorder.complete_run(completion).await;
            }
        }
        Err(QueryRunnerError::ResultMaterialization) => {}
    }
    let _shutdown_result = recorder.shutdown().await;
}
