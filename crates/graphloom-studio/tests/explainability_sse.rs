//! Public HTTP contract tests for the Explainability SSE service.

use std::{error::Error, num::NonZeroUsize, sync::Arc};

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use chrono::{TimeZone, Utc};
#[cfg(feature = "sqlite-store")]
use graphloom::explainability::SqliteExplainabilityStore;
use graphloom::explainability::{
    EventQuery, ExplainabilityContentMode, ExplainabilityEnvelope, ExplainabilityEvent,
    ExplainabilityLiveHub, ExplainabilityLiveHubOptions, ExplainabilityQueryMethod,
    ExplainabilityRecord, ExplainabilityRun, ExplainabilityRunId, ExplainabilityRunKind,
    ExplainabilityRunStatus, ExplainabilityStore, ExplainabilityStoreError,
    InMemoryExplainabilityStore, RunCompletion, RunQuery, RunStarted, StoreExplainabilityOptions,
    StoreExplainabilityRecorder,
};
use graphloom_studio::explainability::ExplainabilitySseService;
#[cfg(feature = "sqlite-store")]
use tempfile::TempDir;
use tower::ServiceExt;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const BODY_LIMIT: usize = 16 * 1024 * 1024;
const SSE_QUERY_SECRET_SENTINEL: &str = "SSE_QUERY_SECRET_SENTINEL";
const SSE_EVENT_SECRET_SENTINEL: &str = "SSE_EVENT_SECRET_SENTINEL";
const SSE_COMPAT_SECRET_SENTINEL: &str = "SSE_COMPAT_SECRET_SENTINEL";

fn run_id(value: &str) -> ExplainabilityRunId {
    value.parse().expect("run id")
}

fn run(id: &ExplainabilityRunId) -> ExplainabilityRun {
    let mut run = ExplainabilityRun::new(
        id.clone(),
        ExplainabilityRunKind::Query,
        Utc.with_ymd_and_hms(2026, 8, 8, 8, 0, 0)
            .single()
            .expect("timestamp"),
    );
    run.status = ExplainabilityRunStatus::Running;
    run.query_method = Some(ExplainabilityQueryMethod::Local);
    run
}

fn envelope(id: &ExplainabilityRunId, sequence: u64) -> ExplainabilityEnvelope {
    ExplainabilityEnvelope::new(
        sequence,
        ExplainabilityRecord::new(
            id.clone(),
            Utc.with_ymd_and_hms(2026, 8, 8, 9, 0, 0)
                .single()
                .expect("timestamp"),
            "http-span".parse().expect("span id"),
            None,
            ExplainabilityEvent::RunStarted(RunStarted::new(
                ExplainabilityRunKind::Query,
                ExplainabilityContentMode::Metadata,
            )),
        ),
    )
    .expect("envelope")
}

async fn historical_service(
    id: &ExplainabilityRunId,
    event_count: u64,
) -> TestResult<(
    Arc<InMemoryExplainabilityStore>,
    ExplainabilitySseService,
    Vec<ExplainabilityEnvelope>,
)> {
    let store = Arc::new(InMemoryExplainabilityStore::new());
    store.create_run(run(id)).await?;
    let events = (1..=event_count)
        .map(|sequence| envelope(id, sequence))
        .collect::<Vec<_>>();
    store.append_events(&events).await?;
    let hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new(),
    ));
    let service =
        ExplainabilitySseService::new(Arc::clone(&store) as Arc<dyn ExplainabilityStore>, hub);
    Ok((store, service, events))
}

fn request(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

async fn response_text(response: axum::response::Response) -> TestResult<String> {
    let bytes = to_bytes(response.into_body(), BODY_LIMIT).await?;
    Ok(String::from_utf8(bytes.to_vec())?)
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedSseEvent {
    id: String,
    event: String,
    data: String,
}

fn parse_sse(body: &str) -> Vec<ParsedSseEvent> {
    body.split("\n\n")
        .filter_map(|block| {
            let mut id = None;
            let mut event = None;
            let mut data = Vec::new();
            for raw_line in block.lines() {
                let line = raw_line.trim_end_matches('\r');
                if let Some(value) = line.strip_prefix("id:") {
                    id = Some(value.trim_start().to_owned());
                } else if let Some(value) = line.strip_prefix("event:") {
                    event = Some(value.trim_start().to_owned());
                } else if let Some(value) = line.strip_prefix("data:") {
                    data.push(value.trim_start().to_owned());
                }
            }
            match (id, event) {
                (Some(id), Some(event)) => Some(ParsedSseEvent {
                    id,
                    event,
                    data: data.join("\n"),
                }),
                _ => None,
            }
        })
        .collect()
}

#[tokio::test]
async fn test_should_return_safe_http_preflight_errors() -> TestResult {
    let unknown_store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new(),
    ));
    let unknown_service = ExplainabilitySseService::new(unknown_store, Arc::clone(&hub));
    let response = unknown_service
        .router()
        .oneshot(request("/api/explainability/runs/unknown/events"))
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response_text(response).await?,
        "explainability run not found"
    );

    let response = unknown_service
        .router()
        .oneshot(request("/api/explainability/runs/bad%2Frun/events"))
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_text(response).await?,
        "invalid explainability event request"
    );

    let failing: Arc<dyn ExplainabilityStore> = Arc::new(FailingGetStore {
        query_secret: SSE_QUERY_SECRET_SENTINEL,
        compat_secret: SSE_COMPAT_SECRET_SENTINEL,
    });
    let failing_service = ExplainabilitySseService::new(failing, hub);
    let response = failing_service
        .router()
        .oneshot(request("/api/explainability/runs/safe/events"))
        .await?;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = response_text(response).await?;
    assert_eq!(body, "explainability service unavailable");
    assert!(!body.contains(SSE_QUERY_SECRET_SENTINEL));
    assert!(!body.contains(SSE_EVENT_SECRET_SENTINEL));
    assert!(!body.contains(SSE_COMPAT_SECRET_SENTINEL));
    assert_eq!(
        format!("{failing_service:?}"),
        "ExplainabilitySseService { .. }"
    );
    Ok(())
}

#[tokio::test]
async fn test_should_reject_invalid_last_event_ids_and_cursor_ahead() -> TestResult {
    let id = run_id("cursor-validation");
    let (_, service, _) = historical_service(&id, 3).await?;
    for value in ["", "abc", "-1", "+1", "1.5", "18446744073709551616"] {
        let request = Request::builder()
            .uri("/api/explainability/runs/cursor-validation/events")
            .header("last-event-id", value)
            .body(Body::empty())?;
        let response = service.router().oneshot(request).await?;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "accepted {value:?}"
        );
        assert_eq!(
            response_text(response).await?,
            "invalid explainability event request"
        );
    }

    let header_request = Request::builder()
        .uri("/api/explainability/runs/cursor-validation/events")
        .header("last-event-id", "4")
        .body(Body::empty())?;
    let response = service.router().oneshot(header_request).await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_text(response).await?,
        "explainability event cursor is ahead of persisted history"
    );

    let response = service
        .router()
        .oneshot(request(
            "/api/explainability/runs/cursor-validation/events?after_sequence=abc",
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_text(response).await?,
        "invalid explainability event request"
    );
    Ok(())
}

#[tokio::test]
async fn test_should_apply_header_precedence_and_query_cursor() -> TestResult {
    let id = run_id("cursor-precedence");
    let (_, service, _) = historical_service(&id, 5).await?;
    let header_request = Request::builder()
        .uri("/api/explainability/runs/cursor-precedence/events?after_sequence=1")
        .header("last-event-id", "3")
        .body(Body::empty())?;
    let response = service.router().oneshot(header_request).await?;
    let events = parse_sse(&response_text(response).await?);
    assert_eq!(
        events
            .iter()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>(),
        vec!["4", "5"]
    );

    let header_request = Request::builder()
        .uri("/api/explainability/runs/cursor-precedence/events?after_sequence=invalid")
        .header("last-event-id", "3")
        .body(Body::empty())?;
    let response = service.router().oneshot(header_request).await?;
    let events = parse_sse(&response_text(response).await?);
    assert_eq!(
        events
            .iter()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>(),
        vec!["4", "5"]
    );

    let response = service
        .router()
        .oneshot(request(
            "/api/explainability/runs/cursor-precedence/events?after_sequence=2",
        ))
        .await?;
    let events = parse_sse(&response_text(response).await?);
    assert_eq!(
        events
            .iter()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>(),
        vec!["3", "4", "5"]
    );
    Ok(())
}

#[tokio::test]
async fn test_should_emit_sse_headers_and_exact_envelope_wire_contract() -> TestResult {
    let id = run_id("wire-contract");
    let (store, service, expected) = historical_service(&id, 2).await?;
    let response = service
        .router()
        .oneshot(request("/api/explainability/runs/wire-contract/events"))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&header::HeaderValue::from_static("text/event-stream"))
    );
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&header::HeaderValue::from_static("no-cache"))
    );
    let parsed = parse_sse(&response_text(response).await?);
    assert_eq!(parsed.len(), 2);
    for (index, event) in parsed.iter().enumerate() {
        assert_eq!(event.event, "explainability");
        assert_eq!(event.id, (index + 1).to_string());
        let envelope: ExplainabilityEnvelope = serde_json::from_str(&event.data)?;
        assert_eq!(envelope, expected[index]);
    }
    assert_eq!(store.load_events(&id, &EventQuery::new()).await?, expected);
    Ok(())
}

#[tokio::test]
async fn test_should_recover_slow_client_without_backpressuring_writer() -> TestResult {
    let id = run_id("slow-client");
    let store = Arc::new(InMemoryExplainabilityStore::new());
    let hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new()
            .with_channel_capacity(NonZeroUsize::new(1).ok_or("capacity")?),
    ));
    let recorder = StoreExplainabilityRecorder::new_with_live_hub(
        Arc::clone(&store) as Arc<dyn ExplainabilityStore>,
        Arc::clone(&hub),
        StoreExplainabilityOptions::new(),
    )?;
    recorder.create_run(run(&id)).await?;
    let service =
        ExplainabilitySseService::new(Arc::clone(&store) as Arc<dyn ExplainabilityStore>, hub);
    let response = service
        .router()
        .oneshot(request("/api/explainability/runs/slow-client/events"))
        .await?;

    let sink = recorder.sink();
    for _ in 0..5 {
        sink.emit(Arc::new(envelope(&id, 1).record)).await?;
    }
    sink.finish_run(&id).await?;
    assert_eq!(store.get_run(&id).await?.ok_or("run")?.event_count, 5);

    let parsed = parse_sse(&response_text(response).await?);
    assert_eq!(
        parsed
            .iter()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>(),
        vec!["1", "2", "3", "4", "5"]
    );
    recorder.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_should_drop_client_without_changing_run_lifecycle() -> TestResult {
    let id = run_id("disconnect");
    let store = Arc::new(InMemoryExplainabilityStore::new());
    let hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new(),
    ));
    let recorder = StoreExplainabilityRecorder::new_with_live_hub(
        Arc::clone(&store) as Arc<dyn ExplainabilityStore>,
        Arc::clone(&hub),
        StoreExplainabilityOptions::new(),
    )?;
    recorder.create_run(run(&id)).await?;
    let service =
        ExplainabilitySseService::new(Arc::clone(&store) as Arc<dyn ExplainabilityStore>, hub);
    let response = service
        .router()
        .oneshot(request("/api/explainability/runs/disconnect/events"))
        .await?;
    drop(response);

    recorder
        .sink()
        .emit(Arc::new(envelope(&id, 1).record))
        .await?;
    recorder.sink().finish_run(&id).await?;
    recorder
        .complete_run(RunCompletion::new(
            id.clone(),
            ExplainabilityRunStatus::Completed,
            Utc.with_ymd_and_hms(2026, 8, 8, 10, 0, 0)
                .single()
                .ok_or("timestamp")?,
        )?)
        .await?;
    recorder.shutdown().await?;
    let stored = store.get_run(&id).await?.ok_or("run")?;
    assert_eq!(stored.status, ExplainabilityRunStatus::Completed);
    assert_eq!(stored.event_count, 1);
    Ok(())
}

#[cfg(feature = "sqlite-store")]
#[tokio::test]
async fn test_should_replay_from_backend_independent_sqlite_store() -> TestResult {
    let directory = TempDir::new()?;
    let store: Arc<dyn ExplainabilityStore> = Arc::new(
        SqliteExplainabilityStore::open(directory.path().join("studio-sse.sqlite")).await?,
    );
    let id = run_id("sqlite-sse");
    store.create_run(run(&id)).await?;
    let expected = vec![envelope(&id, 1), envelope(&id, 2), envelope(&id, 3)];
    store.append_events(&expected).await?;
    let service = ExplainabilitySseService::new(
        Arc::clone(&store),
        Arc::new(ExplainabilityLiveHub::new(
            ExplainabilityLiveHubOptions::new(),
        )),
    );
    let response = service
        .router()
        .oneshot(request("/api/explainability/runs/sqlite-sse/events"))
        .await?;
    let parsed = parse_sse(&response_text(response).await?);
    let actual = parsed
        .iter()
        .map(|event| serde_json::from_str::<ExplainabilityEnvelope>(&event.data))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(actual, expected);
    Ok(())
}

#[derive(Debug)]
struct FailingGetStore {
    query_secret: &'static str,
    compat_secret: &'static str,
}

#[async_trait]
impl ExplainabilityStore for FailingGetStore {
    async fn create_run(&self, _run: ExplainabilityRun) -> Result<(), ExplainabilityStoreError> {
        Ok(())
    }

    async fn append_events(
        &self,
        _events: &[ExplainabilityEnvelope],
    ) -> Result<(), ExplainabilityStoreError> {
        Ok(())
    }

    async fn complete_run(
        &self,
        _completion: RunCompletion,
    ) -> Result<(), ExplainabilityStoreError> {
        Ok(())
    }

    async fn get_run(
        &self,
        _run_id: &ExplainabilityRunId,
    ) -> Result<Option<ExplainabilityRun>, ExplainabilityStoreError> {
        let _query_secret = self.query_secret;
        Err(ExplainabilityStoreError::InvalidLimit {
            kind: self.compat_secret,
            limit: 0,
            min: 1,
            max: 1,
        })
    }

    async fn list_runs(
        &self,
        _query: &RunQuery,
    ) -> Result<Vec<ExplainabilityRun>, ExplainabilityStoreError> {
        Ok(Vec::new())
    }

    async fn load_events(
        &self,
        _run_id: &ExplainabilityRunId,
        _query: &EventQuery,
    ) -> Result<Vec<ExplainabilityEnvelope>, ExplainabilityStoreError> {
        Ok(Vec::new())
    }

    async fn delete_run(
        &self,
        _run_id: &ExplainabilityRunId,
    ) -> Result<(), ExplainabilityStoreError> {
        Ok(())
    }
}
