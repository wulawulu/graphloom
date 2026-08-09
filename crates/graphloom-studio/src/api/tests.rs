use std::{fmt, num::NonZeroUsize, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use chrono::{TimeZone, Utc};
use graphloom::{
    GraphRagConfig,
    explainability::{
        EventQuery, ExplainabilityContentMode, ExplainabilityEnvelope, ExplainabilityEvent,
        ExplainabilityLiveHub, ExplainabilityLiveHubOptions, ExplainabilityLiveRecvError,
        ExplainabilityQueryMethod, ExplainabilityRecord, ExplainabilityRun, ExplainabilityRunId,
        ExplainabilityRunKind, ExplainabilityRunStatus, ExplainabilitySink, ExplainabilitySpanId,
        ExplainabilityStore, ExplainabilityStoreError, InMemoryExplainabilityStore, QueryStarted,
        RunCompleted, RunCompletion, RunFailed, RunQuery, RunStarted,
    },
    query::{QueryOptions, SearchMethod},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Semaphore, mpsc};
use tower::ServiceExt;

use super::{
    StudioApiOptions, StudioApiService,
    query::{QueryRunner, QueryRunnerError},
};

#[derive(Debug, Clone, Copy)]
enum RunnerOutcome {
    Success,
    Failure,
    MissingFinish,
}

#[derive(Debug)]
struct ObservedQuery {
    query: String,
    method: SearchMethod,
    content_mode: ExplainabilityContentMode,
    response_type: String,
}

struct ControlledRunner {
    entered: mpsc::Sender<ObservedQuery>,
    release: Arc<Semaphore>,
    outcome: RunnerOutcome,
}

#[derive(Debug, Clone, Copy)]
enum StoreFailure {
    Create,
    Get,
    List,
}

#[derive(Debug)]
struct FailingStore {
    inner: InMemoryExplainabilityStore,
    failure: StoreFailure,
}

#[derive(Debug)]
struct CompletionProbeStore {
    inner: InMemoryExplainabilityStore,
    completed: Semaphore,
}

impl Default for CompletionProbeStore {
    fn default() -> Self {
        Self {
            inner: InMemoryExplainabilityStore::new(),
            completed: Semaphore::new(0),
        }
    }
}

impl CompletionProbeStore {
    async fn wait_for_completion(&self) {
        let permit = self
            .completed
            .acquire()
            .await
            .expect("completion probe open");
        permit.forget();
    }
}

#[async_trait]
impl ExplainabilityStore for CompletionProbeStore {
    async fn create_run(&self, run: ExplainabilityRun) -> Result<(), ExplainabilityStoreError> {
        self.inner.create_run(run).await
    }

    async fn append_events(
        &self,
        events: &[ExplainabilityEnvelope],
    ) -> Result<(), ExplainabilityStoreError> {
        self.inner.append_events(events).await
    }

    async fn complete_run(
        &self,
        completion: RunCompletion,
    ) -> Result<(), ExplainabilityStoreError> {
        self.inner.complete_run(completion).await?;
        self.completed.add_permits(1);
        Ok(())
    }

    async fn get_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<Option<ExplainabilityRun>, ExplainabilityStoreError> {
        self.inner.get_run(run_id).await
    }

    async fn list_runs(
        &self,
        query: &RunQuery,
    ) -> Result<Vec<ExplainabilityRun>, ExplainabilityStoreError> {
        self.inner.list_runs(query).await
    }

    async fn load_events(
        &self,
        run_id: &ExplainabilityRunId,
        query: &EventQuery,
    ) -> Result<Vec<ExplainabilityEnvelope>, ExplainabilityStoreError> {
        self.inner.load_events(run_id, query).await
    }

    async fn delete_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<(), ExplainabilityStoreError> {
        self.inner.delete_run(run_id).await
    }
}

#[async_trait]
impl ExplainabilityStore for FailingStore {
    async fn create_run(&self, run: ExplainabilityRun) -> Result<(), ExplainabilityStoreError> {
        if matches!(self.failure, StoreFailure::Create) {
            return Err(store_failure("create Studio test run"));
        }
        self.inner.create_run(run).await
    }

    async fn append_events(
        &self,
        events: &[ExplainabilityEnvelope],
    ) -> Result<(), ExplainabilityStoreError> {
        self.inner.append_events(events).await
    }

    async fn complete_run(
        &self,
        completion: RunCompletion,
    ) -> Result<(), ExplainabilityStoreError> {
        self.inner.complete_run(completion).await
    }

    async fn get_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<Option<ExplainabilityRun>, ExplainabilityStoreError> {
        if matches!(self.failure, StoreFailure::Get) {
            return Err(store_failure("get Studio test run"));
        }
        self.inner.get_run(run_id).await
    }

    async fn list_runs(
        &self,
        query: &RunQuery,
    ) -> Result<Vec<ExplainabilityRun>, ExplainabilityStoreError> {
        if matches!(self.failure, StoreFailure::List) {
            return Err(store_failure("list Studio test runs"));
        }
        self.inner.list_runs(query).await
    }

    async fn load_events(
        &self,
        run_id: &ExplainabilityRunId,
        query: &EventQuery,
    ) -> Result<Vec<ExplainabilityEnvelope>, ExplainabilityStoreError> {
        self.inner.load_events(run_id, query).await
    }

    async fn delete_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<(), ExplainabilityStoreError> {
        self.inner.delete_run(run_id).await
    }
}

fn store_failure(operation: &'static str) -> ExplainabilityStoreError {
    ExplainabilityStoreError::Internal {
        operation,
        source: Box::new(std::io::Error::other("test failure")),
    }
}

impl fmt::Debug for ControlledRunner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ControlledRunner { .. }")
    }
}

#[async_trait]
impl QueryRunner for ControlledRunner {
    async fn run(&self, options: QueryOptions) -> Result<(), QueryRunnerError> {
        let explainability = options
            .explainability
            .as_ref()
            .ok_or(QueryRunnerError::Failed)?;
        self.entered
            .send(ObservedQuery {
                query: options.query.clone(),
                method: options.method,
                content_mode: explainability.content_mode(),
                response_type: options.response_type.clone(),
            })
            .await
            .map_err(|_| QueryRunnerError::Failed)?;
        let permit = self
            .release
            .acquire()
            .await
            .map_err(|_| QueryRunnerError::Failed)?;
        permit.forget();

        if matches!(self.outcome, RunnerOutcome::MissingFinish) {
            return Ok(());
        }
        let sink = explainability.sink();
        let run_id = explainability.run_id().clone();
        emit(
            sink,
            &run_id,
            ExplainabilityEvent::RunStarted(RunStarted::new(
                ExplainabilityRunKind::Query,
                explainability.content_mode(),
            )),
        )
        .await?;
        let mut started = QueryStarted::new(ExplainabilityQueryMethod::Local);
        started.query = explainability
            .content_mode()
            .includes_content()
            .then(|| options.query.clone());
        emit(sink, &run_id, ExplainabilityEvent::QueryStarted(started)).await?;
        match self.outcome {
            RunnerOutcome::Success => {
                emit(
                    sink,
                    &run_id,
                    ExplainabilityEvent::RunCompleted(RunCompleted::new(1)),
                )
                .await?;
                sink.finish_run(&run_id)
                    .await
                    .map_err(|_| QueryRunnerError::Failed)?;
                Ok(())
            }
            RunnerOutcome::Failure => {
                emit(
                    sink,
                    &run_id,
                    ExplainabilityEvent::RunFailed(RunFailed::new(
                        "query".to_owned(),
                        "Local query execution failed.".to_owned(),
                    )),
                )
                .await?;
                sink.finish_run(&run_id)
                    .await
                    .map_err(|_| QueryRunnerError::Failed)?;
                Err(QueryRunnerError::Failed)
            }
            RunnerOutcome::MissingFinish => Ok(()),
        }
    }
}

async fn emit(
    sink: &Arc<dyn ExplainabilitySink>,
    run_id: &ExplainabilityRunId,
    event: ExplainabilityEvent,
) -> Result<(), QueryRunnerError> {
    sink.emit(Arc::new(ExplainabilityRecord::new(
        run_id.clone(),
        Utc::now(),
        ExplainabilitySpanId::generate(),
        None,
        event,
    )))
    .await
    .map_err(|_| QueryRunnerError::Failed)
}

struct Harness {
    router: Router,
    store: Arc<CompletionProbeStore>,
    hub: Arc<ExplainabilityLiveHub>,
    observations: mpsc::Receiver<ObservedQuery>,
    release: Arc<Semaphore>,
    query_permits: Arc<Semaphore>,
}

fn harness(outcome: RunnerOutcome, maximum: usize) -> Harness {
    let store = Arc::new(CompletionProbeStore::default());
    let hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new(),
    ));
    let (entered, observations) = mpsc::channel(8);
    let release = Arc::new(Semaphore::new(0));
    let runner = Arc::new(ControlledRunner {
        entered,
        release: Arc::clone(&release),
        outcome,
    });
    let store_dependency: Arc<dyn ExplainabilityStore> = store.clone();
    let service = StudioApiService::with_runner(
        PathBuf::from("."),
        store_dependency,
        Arc::clone(&hub),
        StudioApiOptions::new().with_max_concurrent_queries(
            NonZeroUsize::new(maximum).expect("non-zero test concurrency"),
        ),
        runner,
    );
    let query_permits = Arc::clone(&service.state.query_permits);
    Harness {
        router: service.router(),
        store,
        hub,
        observations,
        release,
        query_permits,
    }
}

fn failing_router(failure: StoreFailure) -> Router {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(FailingStore {
        inner: InMemoryExplainabilityStore::new(),
        failure,
    });
    let hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new(),
    ));
    let (entered, _observations) = mpsc::channel(1);
    let runner = Arc::new(ControlledRunner {
        entered,
        release: Arc::new(Semaphore::new(0)),
        outcome: RunnerOutcome::Success,
    });
    StudioApiService::with_runner(
        PathBuf::from("."),
        store,
        hub,
        StudioApiOptions::new(),
        runner,
    )
    .router()
}

#[derive(Deserialize)]
struct Accepted {
    run_id: ExplainabilityRunId,
    run_url: String,
    events_url: String,
}

async fn post(router: &Router, body: Value) -> axum::response::Response {
    router
        .clone()
        .oneshot(
            Request::post("/api/query")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("test request"),
        )
        .await
        .expect("router response")
}

async fn accepted(response: axum::response::Response) -> Accepted {
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body"),
    )
    .expect("accepted response")
}

async fn drain_closed(
    subscription: &mut graphloom::explainability::ExplainabilityLiveSubscription,
) {
    loop {
        match subscription.recv().await {
            Ok(_) | Err(ExplainabilityLiveRecvError::Lagged { .. }) => {}
            Err(error) => {
                assert_eq!(error, ExplainabilityLiveRecvError::Closed);
                break;
            }
        }
    }
}

#[tokio::test]
async fn test_should_accept_local_only_after_run_is_live_and_not_wait_for_query() {
    let mut harness = harness(RunnerOutcome::Success, 4);
    let response = post(&harness.router, json!({"query":"Who is Alice?"})).await;
    let location = response.headers().get(header::LOCATION).cloned();
    let accepted = accepted(response).await;
    assert_eq!(
        location.and_then(|value| value.to_str().ok().map(str::to_owned)),
        Some(accepted.run_url.clone())
    );
    assert_eq!(accepted.events_url, format!("{}/events", accepted.run_url));
    let mut live = harness
        .hub
        .subscribe(&accepted.run_id)
        .expect("run live before 202");
    let observed = harness.observations.recv().await.expect("runner entered");
    assert_eq!(observed.query, "Who is Alice?");
    assert_eq!(observed.method, SearchMethod::Local);
    assert_eq!(observed.content_mode, ExplainabilityContentMode::Metadata);
    assert_eq!(observed.response_type, "Multiple Paragraphs");
    let running = harness
        .store
        .get_run(&accepted.run_id)
        .await
        .expect("store read")
        .expect("run");
    assert_eq!(running.status, ExplainabilityRunStatus::Running);
    assert!(running.query.is_none());
    harness.release.add_permits(1);
    drain_closed(&mut live).await;
    harness.store.wait_for_completion().await;
    let completed = harness
        .store
        .get_run(&accepted.run_id)
        .await
        .expect("store read")
        .expect("run");
    assert_eq!(completed.status, ExplainabilityRunStatus::Completed);
    assert!(completed.completed_at.is_some());
    let events = harness
        .store
        .load_events(&accepted.run_id, &EventQuery::new())
        .await
        .expect("events");
    assert!(events.iter().any(|envelope| {
        matches!(
            &envelope.record.event,
            ExplainabilityEvent::QueryStarted(event) if event.query.is_none()
        )
    }));
}

#[tokio::test]
async fn test_should_complete_failed_query_as_failed_and_keep_post_accepted() {
    let mut harness = harness(RunnerOutcome::Failure, 1);
    let accepted = accepted(post(&harness.router, json!({"query":"failure"})).await).await;
    let mut live = harness.hub.subscribe(&accepted.run_id).expect("live run");
    let _observed = harness.observations.recv().await.expect("runner entered");
    harness.release.add_permits(1);
    drain_closed(&mut live).await;
    harness.store.wait_for_completion().await;
    let run = harness
        .store
        .get_run(&accepted.run_id)
        .await
        .expect("store read")
        .expect("run");
    assert_eq!(run.status, ExplainabilityRunStatus::Failed);
}

#[tokio::test]
async fn test_should_leave_run_running_when_executor_does_not_finish() {
    let mut harness = harness(RunnerOutcome::MissingFinish, 1);
    let accepted = accepted(post(&harness.router, json!({"query":"unfinished"})).await).await;
    let mut live = harness.hub.subscribe(&accepted.run_id).expect("live run");
    let _observed = harness.observations.recv().await.expect("runner entered");
    harness.release.add_permits(1);
    drain_closed(&mut live).await;
    let run = harness
        .store
        .get_run(&accepted.run_id)
        .await
        .expect("store read")
        .expect("run");
    assert_eq!(run.status, ExplainabilityRunStatus::Running);
    assert!(run.completed_at.is_none());
}

#[tokio::test]
async fn test_should_reject_invalid_unsupported_and_excess_queries_without_ghost_runs() {
    let mut harness = harness(RunnerOutcome::Success, 1);
    for body in [
        json!({"query":""}),
        json!({"query":"q","unknown":true}),
        json!({"query":"q","response_type":""}),
        json!({"query":"x".repeat(1024 * 1024 + 1)}),
        json!({"query":"q","response_type":"x".repeat(257)}),
    ] {
        assert_eq!(
            post(&harness.router, body).await.status(),
            StatusCode::BAD_REQUEST
        );
    }
    let invalid_json = harness
        .router
        .clone()
        .oneshot(
            Request::post("/api/query")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{"))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(invalid_json.status(), StatusCode::BAD_REQUEST);
    for method in ["basic", "global", "drift"] {
        assert_eq!(
            post(&harness.router, json!({"query":"q","method":method}))
                .await
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let first = accepted(post(&harness.router, json!({"query":"first"})).await).await;
    let _observed = harness.observations.recv().await.expect("runner entered");
    assert_eq!(
        post(&harness.router, json!({"query":"second"}))
            .await
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        harness
            .store
            .list_runs(&RunQuery::new())
            .await
            .expect("list")
            .len(),
        1
    );
    let mut live = harness.hub.subscribe(&first.run_id).expect("live");
    harness.release.add_permits(1);
    drain_closed(&mut live).await;
    let released = Arc::clone(&harness.query_permits)
        .acquire_owned()
        .await
        .expect("query admission remains open");
    drop(released);
    let third = accepted(post(&harness.router, json!({"query":"third"})).await).await;
    let mut third_live = harness.hub.subscribe(&third.run_id).expect("live");
    let _observed = harness.observations.recv().await.expect("runner entered");
    harness.release.add_permits(1);
    drain_closed(&mut third_live).await;
}

#[tokio::test]
async fn test_should_return_safe_500_when_run_creation_fails() {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(FailingStore {
        inner: InMemoryExplainabilityStore::new(),
        failure: StoreFailure::Create,
    });
    let hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new(),
    ));
    let (entered, _observations) = mpsc::channel(1);
    let runner = Arc::new(ControlledRunner {
        entered,
        release: Arc::new(Semaphore::new(0)),
        outcome: RunnerOutcome::Success,
    });
    let service = StudioApiService::with_runner(
        PathBuf::from("."),
        store,
        hub,
        StudioApiOptions::new(),
        runner,
    );
    let response = post(&service.router(), json!({"query":"secret query"})).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(body.as_ref(), b"Studio query service unavailable");
}

#[tokio::test]
async fn test_should_preserve_content_mode_in_run_and_events() {
    let mut harness = harness(RunnerOutcome::Success, 1);
    let accepted = accepted(
        post(
            &harness.router,
            json!({"query":"visible query","content_mode":"content"}),
        )
        .await,
    )
    .await;
    let mut live = harness.hub.subscribe(&accepted.run_id).expect("live");
    let _observed = harness.observations.recv().await.expect("entered");
    let running = harness
        .store
        .get_run(&accepted.run_id)
        .await
        .expect("read")
        .expect("run");
    assert_eq!(running.query.as_deref(), Some("visible query"));
    harness.release.add_permits(1);
    drain_closed(&mut live).await;
    let events = harness
        .store
        .load_events(&accepted.run_id, &EventQuery::new())
        .await
        .expect("events");
    assert!(events.iter().any(|envelope| matches!(&envelope.record.event, ExplainabilityEvent::QueryStarted(event) if event.query.as_deref() == Some("visible query"))));
}

#[tokio::test]
async fn test_should_complete_post_to_sse_to_run_lifecycle() {
    let mut harness = harness(RunnerOutcome::Success, 1);
    let accepted = accepted(post(&harness.router, json!({"query":"e2e"})).await).await;
    let _observed = harness.observations.recv().await.expect("entered");
    let sse = harness
        .router
        .clone()
        .oneshot(
            Request::get(&accepted.events_url)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("SSE response");
    assert_eq!(sse.status(), StatusCode::OK);
    harness.release.add_permits(1);
    let body = to_bytes(sse.into_body(), usize::MAX)
        .await
        .expect("SSE body");
    harness.store.wait_for_completion().await;
    let text = std::str::from_utf8(&body).expect("UTF-8 SSE");
    let live_events = text
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(serde_json::from_str::<ExplainabilityEnvelope>)
        .collect::<Result<Vec<_>, _>>()
        .expect("SSE envelopes");
    let stored_events = harness
        .store
        .load_events(&accepted.run_id, &EventQuery::new())
        .await
        .expect("stored events");
    assert_eq!(live_events, stored_events);
    assert_eq!(
        live_events
            .iter()
            .map(ExplainabilityEnvelope::sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert!(
        live_events
            .iter()
            .all(|envelope| envelope.record.run_id == accepted.run_id)
    );
    let run = harness
        .store
        .get_run(&accepted.run_id)
        .await
        .expect("read")
        .expect("run");
    assert_eq!(run.status, ExplainabilityRunStatus::Completed);
    assert_eq!(run.event_count, 3);
}

#[tokio::test]
async fn test_should_continue_query_after_sse_client_disconnects() {
    let mut harness = harness(RunnerOutcome::Success, 1);
    let accepted = accepted(post(&harness.router, json!({"query":"disconnect"})).await).await;
    let _observed = harness.observations.recv().await.expect("entered");
    let sse = harness
        .router
        .clone()
        .oneshot(
            Request::get(&accepted.events_url)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("SSE response");
    drop(sse);
    harness.release.add_permits(1);
    harness.store.wait_for_completion().await;
    let run = harness
        .store
        .get_run(&accepted.run_id)
        .await
        .expect("read")
        .expect("run");
    assert_eq!(run.status, ExplainabilityRunStatus::Completed);
}

#[tokio::test]
async fn test_should_get_runs_and_paginate_history_with_canonical_cursor() {
    let harness = harness(RunnerOutcome::Success, 1);
    let started = Utc
        .with_ymd_and_hms(2026, 8, 9, 1, 2, 3)
        .single()
        .expect("timestamp")
        + chrono::Duration::nanoseconds(123_456_789);
    for id in ["run-e", "run-d", "run-c", "run-b", "run-a"] {
        let run_id: ExplainabilityRunId = id.parse().expect("run id");
        let mut run = ExplainabilityRun::new(run_id.clone(), ExplainabilityRunKind::Query, started);
        run.status = ExplainabilityRunStatus::Running;
        run.query_method = Some(ExplainabilityQueryMethod::Local);
        harness.store.create_run(run).await.expect("create");
        let completion = RunCompletion::new(run_id, ExplainabilityRunStatus::Completed, started)
            .expect("completion");
        harness
            .store
            .complete_run(completion)
            .await
            .expect("complete");
    }
    let other_kind_id: ExplainabilityRunId = "other-kind".parse().expect("run id");
    harness
        .store
        .create_run(ExplainabilityRun::new(
            other_kind_id,
            ExplainabilityRunKind::Index,
            started,
        ))
        .await
        .expect("create other kind");
    let running_id: ExplainabilityRunId = "wrong-status".parse().expect("run id");
    let mut running = ExplainabilityRun::new(running_id, ExplainabilityRunKind::Query, started);
    running.status = ExplainabilityRunStatus::Running;
    running.query_method = Some(ExplainabilityQueryMethod::Local);
    harness
        .store
        .create_run(running)
        .await
        .expect("create running");
    let wrong_method_id: ExplainabilityRunId = "wrong-method".parse().expect("run id");
    let mut wrong_method = ExplainabilityRun::new(
        wrong_method_id.clone(),
        ExplainabilityRunKind::Query,
        started,
    );
    wrong_method.status = ExplainabilityRunStatus::Running;
    wrong_method.query_method = Some(ExplainabilityQueryMethod::Basic);
    harness
        .store
        .create_run(wrong_method)
        .await
        .expect("create wrong method");
    harness
        .store
        .complete_run(
            RunCompletion::new(wrong_method_id, ExplainabilityRunStatus::Completed, started)
                .expect("completion"),
        )
        .await
        .expect("complete wrong method");
    let page = harness
        .router
        .clone()
        .oneshot(
            Request::get(
                "/api/explainability/runs?kind=query&status=completed&query_method=local&limit=2",
            )
            .body(Body::empty())
            .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(page.status(), StatusCode::OK);
    let value: Value =
        serde_json::from_slice(&to_bytes(page.into_body(), usize::MAX).await.expect("body"))
            .expect("json");
    assert_eq!(value["runs"].as_array().expect("runs").len(), 2);
    assert_eq!(value["runs"][0]["run_id"], "run-e");
    assert_eq!(value["runs"][1]["run_id"], "run-d");
    let timestamp = value["next_cursor"]["started_at"]
        .as_str()
        .expect("timestamp");
    assert_eq!(timestamp, "2026-08-09T01:02:03.123456789Z");
    let cursor_id = value["next_cursor"]["run_id"].as_str().expect("cursor id");
    let uri = format!(
        "/api/explainability/runs?kind=query&status=completed&query_method=local&limit=2&\
         before_started_at={timestamp}&before_run_id={cursor_id}"
    );
    let next = harness
        .router
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let next_value: Value =
        serde_json::from_slice(&to_bytes(next.into_body(), usize::MAX).await.expect("body"))
            .expect("json");
    assert_eq!(next_value["runs"][0]["run_id"], "run-c");
    assert_eq!(next_value["runs"][1]["run_id"], "run-b");
    let next_timestamp = next_value["next_cursor"]["started_at"]
        .as_str()
        .expect("timestamp");
    let next_id = next_value["next_cursor"]["run_id"]
        .as_str()
        .expect("cursor id");
    let final_uri = format!(
        "/api/explainability/runs?kind=query&status=completed&query_method=local&limit=2&\
         before_started_at={next_timestamp}&before_run_id={next_id}"
    );
    let final_page = harness
        .router
        .clone()
        .oneshot(
            Request::get(final_uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let final_value: Value = serde_json::from_slice(
        &to_bytes(final_page.into_body(), usize::MAX)
            .await
            .expect("body"),
    )
    .expect("json");
    assert_eq!(final_value["runs"].as_array().expect("runs").len(), 1);
    assert_eq!(final_value["runs"][0]["run_id"], "run-a");
    assert!(final_value["next_cursor"].is_null());
    for invalid in [
        "/api/explainability/runs?limit=0",
        "/api/explainability/runs?limit=201",
        "/api/explainability/runs?kind=unknown",
        "/api/explainability/runs?status=unknown",
        "/api/explainability/runs?query_method=unknown",
        "/api/explainability/runs?before_run_id=run-a",
        "/api/explainability/runs?before_started_at=2026-08-09T01:02:03.123456789Z",
        "/api/explainability/runs?before_started_at=2026-08-09T01:02:03.123456789Z&\
         before_run_id=bad%20id",
        "/api/explainability/runs?before_started_at=2026-08-09T01:02:03Z&before_run_id=run-a",
    ] {
        assert_eq!(
            harness
                .router
                .clone()
                .oneshot(Request::get(invalid).body(Body::empty()).expect("request"))
                .await
                .expect("response")
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        harness
            .router
            .clone()
            .oneshot(
                Request::get("/api/explainability/runs/run-a")
                    .body(Body::empty())
                    .expect("request")
            )
            .await
            .expect("response")
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        harness
            .router
            .clone()
            .oneshot(
                Request::get("/api/explainability/runs/missing")
                    .body(Body::empty())
                    .expect("request")
            )
            .await
            .expect("response")
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        harness
            .router
            .clone()
            .oneshot(
                Request::get("/api/explainability/runs/%20")
                    .body(Body::empty())
                    .expect("request")
            )
            .await
            .expect("response")
            .status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn test_should_list_empty_and_single_run_pages_without_next_cursor() {
    let harness = harness(RunnerOutcome::Success, 1);
    let empty = harness
        .router
        .clone()
        .oneshot(
            Request::get("/api/explainability/runs?limit=2")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let empty_value: Value =
        serde_json::from_slice(&to_bytes(empty.into_body(), usize::MAX).await.expect("body"))
            .expect("json");
    assert!(empty_value["runs"].as_array().expect("runs").is_empty());
    assert!(empty_value["next_cursor"].is_null());

    let run_id: ExplainabilityRunId = "single-run".parse().expect("run id");
    let mut run = ExplainabilityRun::new(run_id, ExplainabilityRunKind::Query, Utc::now());
    run.status = ExplainabilityRunStatus::Running;
    run.query_method = Some(ExplainabilityQueryMethod::Local);
    harness.store.create_run(run).await.expect("create run");
    let single = harness
        .router
        .clone()
        .oneshot(
            Request::get("/api/explainability/runs?limit=2")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let single_value: Value = serde_json::from_slice(
        &to_bytes(single.into_body(), usize::MAX)
            .await
            .expect("body"),
    )
    .expect("json");
    assert_eq!(single_value["runs"].as_array().expect("runs").len(), 1);
    assert!(single_value["next_cursor"].is_null());
}

#[tokio::test]
async fn test_should_return_safe_500_for_run_read_failures() {
    for (failure, uri) in [
        (StoreFailure::Get, "/api/explainability/runs/run-a"),
        (StoreFailure::List, "/api/explainability/runs"),
    ] {
        let response = failing_router(failure)
            .oneshot(Request::get(uri).body(Body::empty()).expect("request"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(body.as_ref(), b"explainability service unavailable");
    }
}

#[test]
fn test_should_compile_direct_public_query_tokio_spawn() {
    fn spawn(
        config: GraphRagConfig,
        options: QueryOptions,
    ) -> tokio::task::JoinHandle<graphloom::Result<graphloom::query::QueryResult>> {
        tokio::spawn(async move { graphloom::api::query(config, options).await })
    }
    let _spawn = spawn;
}

#[test]
fn test_should_keep_service_debug_opaque_and_options_bounded() {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new(),
    ));
    let service = StudioApiService::new(
        GraphRagConfig::default(),
        PathBuf::from("STUDIO_PATH_SECRET_SENTINEL"),
        store,
        hub,
        StudioApiOptions::new(),
    );
    assert_eq!(format!("{service:?}"), "StudioApiService { .. }");
    assert_eq!(StudioApiOptions::new().max_concurrent_queries().get(), 4);
}
