//! Stateless SSE replay and realtime recovery for explainability runs.

use std::{collections::VecDeque, fmt, mem, sync::Arc, time::Duration};

use axum::{
    Router,
    extract::{
        Path, Query, State,
        rejection::{PathRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use futures_util::{Stream, StreamExt, stream};
use graphloom::explainability::{
    EventQuery, ExplainabilityEnvelope, ExplainabilityLiveHub, ExplainabilityLiveRecvError,
    ExplainabilityLiveSubscription, ExplainabilityRunId, ExplainabilityStore,
};
use serde::Deserialize;
use thiserror::Error;

const SSE_ROUTE: &str = "/api/explainability/runs/{run_id}/events";
const SSE_EVENT_NAME: &str = "explainability";
const SSE_REPLAY_PAGE_SIZE: u32 = 64;
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);
const LAST_EVENT_ID: &str = "last-event-id";

const INVALID_REQUEST_BODY: &str = "invalid explainability event request";
const RUN_NOT_FOUND_BODY: &str = "explainability run not found";
const CURSOR_AHEAD_BODY: &str = "explainability event cursor is ahead of persisted history";
const SERVICE_UNAVAILABLE_BODY: &str = "explainability service unavailable";

/// Axum service exposing persisted explainability envelopes over Server-Sent Events.
///
/// The supplied Store and Live Hub must represent the same logical Run-ID namespace. The service
/// is read-only: it owns neither persistence nor live-run lifecycle and stores no client cursor.
#[derive(Clone)]
#[non_exhaustive]
pub struct ExplainabilitySseService {
    state: Arc<ServiceState>,
}

impl fmt::Debug for ExplainabilitySseService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExplainabilitySseService { .. }")
    }
}

impl ExplainabilitySseService {
    /// Create a read-only SSE service over one Store and matching Live Hub namespace.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use graphloom::explainability::{
    ///     ExplainabilityLiveHub, ExplainabilityLiveHubOptions, ExplainabilityStore,
    ///     InMemoryExplainabilityStore,
    /// };
    /// use graphloom_studio::explainability::ExplainabilitySseService;
    ///
    /// let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    /// let hub = Arc::new(ExplainabilityLiveHub::new(
    ///     ExplainabilityLiveHubOptions::new(),
    /// ));
    /// let service = ExplainabilitySseService::new(store, hub);
    /// let _router = service.router();
    /// ```
    #[must_use]
    pub fn new(store: Arc<dyn ExplainabilityStore>, live_hub: Arc<ExplainabilityLiveHub>) -> Self {
        Self {
            state: Arc::new(ServiceState { store, live_hub }),
        }
    }

    /// Build a composable Router containing only the explainability event endpoint.
    pub fn router(&self) -> Router {
        Router::new()
            .route(SSE_ROUTE, get(events_handler))
            .with_state(Arc::clone(&self.state))
    }
}

struct ServiceState {
    store: Arc<dyn ExplainabilityStore>,
    live_hub: Arc<ExplainabilityLiveHub>,
}

impl fmt::Debug for ServiceState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ServiceState { .. }")
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct EventsQuery {
    after_sequence: Option<u64>,
}

async fn events_handler(
    State(state): State<Arc<ServiceState>>,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<EventsQuery>, QueryRejection>,
    headers: HeaderMap,
) -> Response {
    let Ok(Path(raw_run_id)) = path else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_REQUEST_BODY);
    };
    let Ok(run_id) = raw_run_id.parse::<ExplainabilityRunId>() else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_REQUEST_BODY);
    };
    let query_cursor = if headers.contains_key(LAST_EVENT_ID) {
        None
    } else {
        let Ok(Query(query)) = query else {
            return fixed_error(StatusCode::BAD_REQUEST, INVALID_REQUEST_BODY);
        };
        query.after_sequence
    };
    let Ok(last_seen_sequence) = resolve_cursor(&headers, query_cursor) else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_REQUEST_BODY);
    };

    let run = match state.store.get_run(&run_id).await {
        Ok(Some(run)) => run,
        Ok(None) => return fixed_error(StatusCode::NOT_FOUND, RUN_NOT_FOUND_BODY),
        Err(_) => return fixed_error(StatusCode::INTERNAL_SERVER_ERROR, SERVICE_UNAVAILABLE_BODY),
    };
    if last_seen_sequence > run.event_count {
        return fixed_error(StatusCode::CONFLICT, CURSOR_AHEAD_BODY);
    }

    let envelope_stream = envelope_stream(
        Arc::clone(&state.store),
        Arc::clone(&state.live_hub),
        run_id,
        last_seen_sequence,
    );
    let event_stream = envelope_stream
        .map(|result| result.and_then(|envelope| envelope_to_event(envelope.as_ref())));
    let mut response = Sse::new(event_stream)
        .keep_alive(KeepAlive::new().interval(KEEP_ALIVE_INTERVAL))
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-cache"),
    );
    response
}

fn fixed_error(status: StatusCode, body: &'static str) -> Response {
    (status, body).into_response()
}

fn resolve_cursor(
    headers: &HeaderMap,
    query_cursor: Option<u64>,
) -> Result<u64, ExplainabilitySseRequestError> {
    match headers.get(LAST_EVENT_ID) {
        Some(value) => parse_last_event_id(
            value
                .to_str()
                .map_err(|_| ExplainabilitySseRequestError::InvalidCursor)?,
        ),
        None => Ok(query_cursor.unwrap_or(0)),
    }
}

fn parse_last_event_id(value: &str) -> Result<u64, ExplainabilitySseRequestError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ExplainabilitySseRequestError::InvalidCursor);
    }
    value
        .parse()
        .map_err(|_| ExplainabilitySseRequestError::InvalidCursor)
}

fn envelope_to_event(
    envelope: &ExplainabilityEnvelope,
) -> Result<Event, ExplainabilitySseStreamError> {
    Event::default()
        .event(SSE_EVENT_NAME)
        .id(envelope.sequence().to_string())
        .json_data(envelope)
        .map_err(|_| ExplainabilitySseStreamError::Serialization)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
enum ExplainabilitySseRequestError {
    #[error("invalid explainability event cursor")]
    InvalidCursor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
enum ExplainabilitySseStreamError {
    #[error("explainability SSE store replay failed")]
    Store,
    #[error("explainability SSE sequence invariant failed")]
    SequenceInvariant,
    #[error("explainability SSE serialization failed")]
    Serialization,
}

fn envelope_stream(
    store: Arc<dyn ExplainabilityStore>,
    live_hub: Arc<ExplainabilityLiveHub>,
    run_id: ExplainabilityRunId,
    last_seen_sequence: u64,
) -> impl Stream<Item = Result<Arc<ExplainabilityEnvelope>, ExplainabilitySseStreamError>> + Send {
    stream::unfold(
        EnvelopeStreamState::new(store, live_hub, run_id, last_seen_sequence),
        |mut state| async move { state.next_envelope().await.map(|item| (item, state)) },
    )
}

struct EnvelopeStreamState {
    store: Arc<dyn ExplainabilityStore>,
    live_hub: Arc<ExplainabilityLiveHub>,
    run_id: ExplainabilityRunId,
    last_seen_sequence: u64,
    phase: StreamPhase,
    pending: VecDeque<Arc<ExplainabilityEnvelope>>,
    #[cfg(test)]
    recovery_subscriptions: u64,
}

impl fmt::Debug for EnvelopeStreamState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EnvelopeStreamState { .. }")
    }
}

impl EnvelopeStreamState {
    fn new(
        store: Arc<dyn ExplainabilityStore>,
        live_hub: Arc<ExplainabilityLiveHub>,
        run_id: ExplainabilityRunId,
        last_seen_sequence: u64,
    ) -> Self {
        let phase = match live_hub.subscribe(&run_id) {
            Some(subscription) => StreamPhase::CatchUp {
                target_sequence: subscription.snapshot_sequence(),
                subscription,
            },
            None => StreamPhase::FinalCatchUp,
        };
        Self {
            store,
            live_hub,
            run_id,
            last_seen_sequence,
            phase,
            pending: VecDeque::new(),
            #[cfg(test)]
            recovery_subscriptions: 0,
        }
    }

    async fn next_envelope(
        &mut self,
    ) -> Option<Result<Arc<ExplainabilityEnvelope>, ExplainabilitySseStreamError>> {
        loop {
            if let Some(envelope) = self.pending.pop_front() {
                self.last_seen_sequence = envelope.sequence();
                return Some(Ok(envelope));
            }

            let phase = mem::replace(&mut self.phase, StreamPhase::Done);
            match phase {
                StreamPhase::CatchUp {
                    target_sequence,
                    subscription,
                } => {
                    if self.last_seen_sequence >= target_sequence {
                        self.phase = StreamPhase::Live { subscription };
                    } else {
                        self.phase = StreamPhase::CatchUp {
                            target_sequence,
                            subscription,
                        };
                        match self.load_store_page().await {
                            Ok(false) => {
                                return Some(
                                    self.fail(ExplainabilitySseStreamError::SequenceInvariant),
                                );
                            }
                            Ok(true) => {}
                            Err(error) => return Some(self.fail(error)),
                        }
                    }
                }
                StreamPhase::Live { mut subscription } => match subscription.recv().await {
                    Ok(envelope) => {
                        if envelope.record.run_id != self.run_id {
                            return Some(
                                self.fail(ExplainabilitySseStreamError::SequenceInvariant),
                            );
                        }
                        if envelope.sequence() <= self.last_seen_sequence {
                            self.phase = StreamPhase::Live { subscription };
                            continue;
                        }
                        let Some(expected) = self.last_seen_sequence.checked_add(1) else {
                            return Some(
                                self.fail(ExplainabilitySseStreamError::SequenceInvariant),
                            );
                        };
                        if envelope.sequence() == expected {
                            self.last_seen_sequence = envelope.sequence();
                            self.phase = StreamPhase::Live { subscription };
                            return Some(Ok(envelope));
                        }
                        self.recover_live_gap();
                    }
                    Err(ExplainabilityLiveRecvError::Lagged { .. }) => self.recover_live_gap(),
                    Err(ExplainabilityLiveRecvError::Closed) => {
                        self.phase = StreamPhase::FinalCatchUp;
                    }
                    Err(_) => {
                        return Some(self.fail(ExplainabilitySseStreamError::SequenceInvariant));
                    }
                },
                StreamPhase::FinalCatchUp => {
                    self.phase = StreamPhase::FinalCatchUp;
                    match self.load_store_page().await {
                        Ok(false) => {
                            self.phase = StreamPhase::Done;
                            return None;
                        }
                        Ok(true) => {
                            if self.pending.len() < SSE_REPLAY_PAGE_SIZE as usize {
                                self.phase = StreamPhase::DoneAfterPending;
                            }
                        }
                        Err(error) => return Some(self.fail(error)),
                    }
                }
                StreamPhase::DoneAfterPending | StreamPhase::Done => {
                    self.phase = StreamPhase::Done;
                    return None;
                }
            }
        }
    }

    fn recover_live_gap(&mut self) {
        #[cfg(test)]
        {
            self.recovery_subscriptions = self.recovery_subscriptions.saturating_add(1);
        }
        self.phase = match self.live_hub.subscribe(&self.run_id) {
            Some(subscription) => StreamPhase::CatchUp {
                target_sequence: subscription.snapshot_sequence(),
                subscription,
            },
            None => StreamPhase::FinalCatchUp,
        };
    }

    async fn load_store_page(&mut self) -> Result<bool, ExplainabilitySseStreamError> {
        let query = EventQuery::new()
            .after_sequence(self.last_seen_sequence)
            .with_limit(SSE_REPLAY_PAGE_SIZE)
            .map_err(|_| ExplainabilitySseStreamError::Store)?;
        let page = self
            .store
            .load_events(&self.run_id, &query)
            .await
            .map_err(|_| ExplainabilitySseStreamError::Store)?;
        if page.len() > SSE_REPLAY_PAGE_SIZE as usize {
            return Err(ExplainabilitySseStreamError::SequenceInvariant);
        }
        if page.is_empty() {
            return Ok(false);
        }

        let mut expected = Some(
            self.last_seen_sequence
                .checked_add(1)
                .ok_or(ExplainabilitySseStreamError::SequenceInvariant)?,
        );
        for envelope in page {
            let expected_sequence =
                expected.ok_or(ExplainabilitySseStreamError::SequenceInvariant)?;
            if envelope.record.run_id != self.run_id || envelope.sequence() != expected_sequence {
                return Err(ExplainabilitySseStreamError::SequenceInvariant);
            }
            expected = expected_sequence.checked_add(1);
            self.pending.push_back(Arc::new(envelope));
        }
        Ok(true)
    }

    fn fail(
        &mut self,
        error: ExplainabilitySseStreamError,
    ) -> Result<Arc<ExplainabilityEnvelope>, ExplainabilitySseStreamError> {
        self.pending.clear();
        self.phase = StreamPhase::Done;
        Err(error)
    }
}

enum StreamPhase {
    CatchUp {
        target_sequence: u64,
        subscription: ExplainabilityLiveSubscription,
    },
    Live {
        subscription: ExplainabilityLiveSubscription,
    },
    FinalCatchUp,
    DoneAfterPending,
    Done,
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, error::Error, num::NonZeroUsize, sync::Arc};

    use chrono::{TimeZone, Utc};
    use futures_util::{StreamExt, TryStreamExt};
    use graphloom::explainability::{
        ExplainabilityContentMode, ExplainabilityEnvelope, ExplainabilityEvent,
        ExplainabilityLiveHub, ExplainabilityLiveHubOptions, ExplainabilityRecord,
        ExplainabilityRun, ExplainabilityRunId, ExplainabilityRunKind, ExplainabilityRunStatus,
        ExplainabilityStore, ExplainabilityStoreError, InMemoryExplainabilityStore, RunCompleted,
        RunFailed, RunStarted, StoreExplainabilityOptions, StoreExplainabilityRecorder,
    };

    use super::{
        EnvelopeStreamState, ExplainabilitySseStreamError, SSE_REPLAY_PAGE_SIZE, StreamPhase,
        envelope_stream, parse_last_event_id,
    };

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

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
                "sse-span".parse().expect("span id"),
                None,
                ExplainabilityEvent::RunStarted(RunStarted::new(
                    ExplainabilityRunKind::Query,
                    ExplainabilityContentMode::Metadata,
                )),
            ),
        )
        .expect("envelope")
    }

    fn record(id: &ExplainabilityRunId) -> Arc<ExplainabilityRecord> {
        Arc::new(envelope(id, 1).record)
    }

    fn record_with_event(
        id: &ExplainabilityRunId,
        event: ExplainabilityEvent,
    ) -> Arc<ExplainabilityRecord> {
        Arc::new(ExplainabilityRecord::new(
            id.clone(),
            Utc.with_ymd_and_hms(2026, 8, 8, 9, 0, 0)
                .single()
                .expect("timestamp"),
            "sse-span".parse().expect("span id"),
            None,
            event,
        ))
    }

    async fn create_recorder(
        id: &ExplainabilityRunId,
        capacity: usize,
    ) -> TestResult<(
        Arc<InMemoryExplainabilityStore>,
        Arc<ExplainabilityLiveHub>,
        StoreExplainabilityRecorder,
    )> {
        let store = Arc::new(InMemoryExplainabilityStore::new());
        let hub = Arc::new(ExplainabilityLiveHub::new(
            ExplainabilityLiveHubOptions::new()
                .with_channel_capacity(NonZeroUsize::new(capacity).ok_or("capacity")?),
        ));
        let recorder = StoreExplainabilityRecorder::new_with_live_hub(
            Arc::clone(&store) as Arc<dyn ExplainabilityStore>,
            Arc::clone(&hub),
            StoreExplainabilityOptions::new(),
        )?;
        recorder.create_run(run(id)).await?;
        Ok((store, hub, recorder))
    }

    async fn persist_records(
        recorder: &StoreExplainabilityRecorder,
        id: &ExplainabilityRunId,
        count: usize,
        barrier_id: &str,
    ) -> TestResult {
        let sink = recorder.sink();
        for _ in 0..count {
            sink.emit(record(id)).await?;
        }
        recorder.create_run(run(&run_id(barrier_id))).await?;
        Ok(())
    }

    #[test]
    fn test_should_parse_only_strict_ascii_decimal_last_event_ids() {
        for (input, expected) in [
            ("0", 0),
            ("1", 1),
            ("42", 42),
            ("18446744073709551615", u64::MAX),
        ] {
            assert_eq!(parse_last_event_id(input), Ok(expected));
        }
        for invalid in [
            "",
            "-1",
            "+1",
            " 1",
            "1 ",
            "1.0",
            "abc",
            "18446744073709551616",
        ] {
            assert!(
                parse_last_event_id(invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn test_should_keep_all_stream_error_messages_payload_free() {
        for (error, expected) in [
            (
                ExplainabilitySseStreamError::Store,
                "explainability SSE store replay failed",
            ),
            (
                ExplainabilitySseStreamError::SequenceInvariant,
                "explainability SSE sequence invariant failed",
            ),
            (
                ExplainabilitySseStreamError::Serialization,
                "explainability SSE serialization failed",
            ),
        ] {
            let message = error.to_string();
            assert_eq!(message, expected);
            assert!(!message.contains("SSE_EVENT_SECRET_SENTINEL"));
        }
    }

    #[tokio::test]
    async fn test_should_replay_historical_run_across_multiple_bounded_pages() -> TestResult {
        let id = run_id("historical-130");
        let store = Arc::new(InMemoryExplainabilityStore::new());
        store.create_run(run(&id)).await?;
        let events = (1..=130)
            .map(|sequence| envelope(&id, sequence))
            .collect::<Vec<_>>();
        store.append_events(&events).await?;
        let hub = Arc::new(ExplainabilityLiveHub::new(
            ExplainabilityLiveHubOptions::new(),
        ));

        let received = envelope_stream(
            Arc::clone(&store) as Arc<dyn ExplainabilityStore>,
            Arc::clone(&hub),
            id.clone(),
            0,
        )
        .map(|item| item.map(|event| event.sequence()))
        .try_collect::<Vec<_>>()
        .await?;
        assert_eq!(received, (1..=130).collect::<Vec<_>>());

        let resumed = envelope_stream(store as Arc<dyn ExplainabilityStore>, hub, id, 63)
            .map(|item| item.map(|event| event.sequence()))
            .try_collect::<Vec<_>>()
            .await?;
        assert_eq!(resumed, (64..=130).collect::<Vec<_>>());
        Ok(())
    }

    #[tokio::test]
    async fn test_should_catch_up_to_initial_snapshot_without_gap() -> TestResult {
        let id = run_id("initial-snapshot");
        let (store, hub, recorder) = create_recorder(&id, 16).await?;
        persist_records(&recorder, &id, 3, "initial-barrier").await?;
        let mut state = EnvelopeStreamState::new(
            Arc::clone(&store) as Arc<dyn ExplainabilityStore>,
            Arc::clone(&hub),
            id.clone(),
            0,
        );
        let mut received = Vec::new();
        for _ in 0..3 {
            received.push(state.next_envelope().await.ok_or("event")??.sequence());
        }
        assert_eq!(received, vec![1, 2, 3]);
        assert!(matches!(
            state.phase,
            StreamPhase::Live { .. } | StreamPhase::CatchUp { .. }
        ));
        recorder.sink().finish_run(&id).await?;
        recorder.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_should_deduplicate_store_live_overlap() -> TestResult {
        let id = run_id("overlap");
        let (store, hub, recorder) = create_recorder(&id, 16).await?;
        let subscription = hub.subscribe(&id).ok_or("subscription")?;
        persist_records(&recorder, &id, 2, "overlap-barrier").await?;
        let mut state = EnvelopeStreamState {
            store: Arc::clone(&store) as Arc<dyn ExplainabilityStore>,
            live_hub: Arc::clone(&hub),
            run_id: id.clone(),
            last_seen_sequence: 1,
            phase: StreamPhase::Live { subscription },
            pending: VecDeque::new(),
            recovery_subscriptions: 0,
        };
        assert_eq!(state.next_envelope().await.ok_or("event")??.sequence(), 2);
        assert_eq!(state.recovery_subscriptions, 0);
        recorder.sink().finish_run(&id).await?;
        recorder.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_should_resubscribe_with_new_snapshot_after_lag() -> TestResult {
        let id = run_id("lag-recovery");
        let (store, hub, recorder) = create_recorder(&id, 1).await?;
        let subscription = hub.subscribe(&id).ok_or("subscription")?;
        persist_records(&recorder, &id, 5, "lag-barrier").await?;
        let mut state = EnvelopeStreamState {
            store: Arc::clone(&store) as Arc<dyn ExplainabilityStore>,
            live_hub: Arc::clone(&hub),
            run_id: id.clone(),
            last_seen_sequence: 0,
            phase: StreamPhase::Live { subscription },
            pending: VecDeque::new(),
            recovery_subscriptions: 0,
        };
        let mut received = Vec::new();
        for _ in 0..5 {
            received.push(state.next_envelope().await.ok_or("event")??.sequence());
        }
        assert_eq!(received, vec![1, 2, 3, 4, 5]);
        assert_eq!(state.recovery_subscriptions, 1);
        assert!(matches!(
            state.phase,
            StreamPhase::CatchUp {
                target_sequence: 5,
                ..
            }
        ));
        recorder.sink().finish_run(&id).await?;
        recorder.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_should_recover_live_sequence_gap_before_sending_jump() -> TestResult {
        let id = run_id("gap-recovery");
        let (store, hub, recorder) = create_recorder(&id, 16).await?;
        let mut subscription = hub.subscribe(&id).ok_or("subscription")?;
        persist_records(&recorder, &id, 3, "gap-barrier").await?;
        assert_eq!(subscription.recv().await?.sequence(), 1);
        let mut state = EnvelopeStreamState {
            store: Arc::clone(&store) as Arc<dyn ExplainabilityStore>,
            live_hub: Arc::clone(&hub),
            run_id: id.clone(),
            last_seen_sequence: 0,
            phase: StreamPhase::Live { subscription },
            pending: VecDeque::new(),
            recovery_subscriptions: 0,
        };
        assert_eq!(state.next_envelope().await.ok_or("event")??.sequence(), 1);
        assert_eq!(state.recovery_subscriptions, 1);
        recorder.sink().finish_run(&id).await?;
        recorder.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_should_final_catch_up_after_live_closed() -> TestResult {
        let id = run_id("closed-catch-up");
        let (store, hub, recorder) = create_recorder(&id, 16).await?;
        let mut state = EnvelopeStreamState::new(
            Arc::clone(&store) as Arc<dyn ExplainabilityStore>,
            hub,
            id.clone(),
            0,
        );
        persist_records(&recorder, &id, 1, "closed-barrier").await?;
        assert_eq!(state.next_envelope().await.ok_or("event")??.sequence(), 1);
        recorder.sink().finish_run(&id).await?;
        store.append_events(&[envelope(&id, 2)]).await?;
        assert_eq!(state.next_envelope().await.ok_or("event")??.sequence(), 2);
        assert!(state.next_envelope().await.is_none());
        recorder.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_should_not_end_live_stream_for_terminal_business_events() -> TestResult {
        let id = run_id("completed-event-is-data");
        let (store, hub, recorder) = create_recorder(&id, 16).await?;
        let mut state =
            EnvelopeStreamState::new(store as Arc<dyn ExplainabilityStore>, hub, id.clone(), 0);
        recorder
            .sink()
            .emit(record_with_event(
                &id,
                ExplainabilityEvent::RunCompleted(RunCompleted::new(1)),
            ))
            .await?;
        recorder
            .sink()
            .emit(record_with_event(
                &id,
                ExplainabilityEvent::RunFailed(RunFailed::new(
                    "safe_kind".to_owned(),
                    "safe message".to_owned(),
                )),
            ))
            .await?;
        recorder.sink().emit(record(&id)).await?;
        recorder
            .create_run(run(&run_id("completed-event-barrier")))
            .await?;

        assert!(matches!(
            state.next_envelope().await.ok_or("event")??.record.event,
            ExplainabilityEvent::RunCompleted(_)
        ));
        assert!(matches!(
            state.next_envelope().await.ok_or("event")??.record.event,
            ExplainabilityEvent::RunFailed(_)
        ));
        assert_eq!(state.next_envelope().await.ok_or("event")??.sequence(), 3);
        recorder.sink().finish_run(&id).await?;
        recorder.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_should_fail_when_snapshot_boundary_cannot_be_replayed() -> TestResult {
        let id = run_id("unreachable-snapshot");
        let (store, hub, recorder) = create_recorder(&id, 16).await?;
        let subscription = hub.subscribe(&id).ok_or("subscription")?;
        let mut state = EnvelopeStreamState {
            store: store as Arc<dyn ExplainabilityStore>,
            live_hub: hub,
            run_id: id.clone(),
            last_seen_sequence: 0,
            phase: StreamPhase::CatchUp {
                target_sequence: 1,
                subscription,
            },
            pending: VecDeque::new(),
            recovery_subscriptions: 0,
        };
        assert!(matches!(
            state.next_envelope().await,
            Some(Err(ExplainabilitySseStreamError::SequenceInvariant))
        ));
        recorder.sink().finish_run(&id).await?;
        recorder.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_should_terminate_with_safe_error_when_store_replay_fails() -> TestResult {
        let store: Arc<dyn ExplainabilityStore> = Arc::new(FailingLoadStore);
        let hub = Arc::new(ExplainabilityLiveHub::new(
            ExplainabilityLiveHubOptions::new(),
        ));
        let mut state = EnvelopeStreamState::new(store, hub, run_id("store-failure"), 0);
        let error = state.next_envelope().await.ok_or("stream item")?;
        let Err(error) = error else {
            return Err("expected store replay error".into());
        };
        assert_eq!(error.to_string(), "explainability SSE store replay failed");
        assert!(!error.to_string().contains("SSE_EVENT_SECRET_SENTINEL"));
        assert!(state.next_envelope().await.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_should_reject_wrong_run_gap_duplicate_and_out_of_order_store_pages() -> TestResult
    {
        let requested = run_id("requested");
        let wrong = run_id("wrong");
        for page in [
            vec![envelope(&wrong, 1)],
            vec![envelope(&requested, 2)],
            vec![envelope(&requested, 1), envelope(&requested, 1)],
            vec![envelope(&requested, 1), envelope(&requested, 3)],
            (1..=u64::from(SSE_REPLAY_PAGE_SIZE) + 1)
                .map(|sequence| envelope(&requested, sequence))
                .collect(),
        ] {
            let store = Arc::new(CorruptStore { page });
            let hub = Arc::new(ExplainabilityLiveHub::new(
                ExplainabilityLiveHubOptions::new(),
            ));
            let mut state = EnvelopeStreamState::new(
                store as Arc<dyn ExplainabilityStore>,
                hub,
                requested.clone(),
                0,
            );
            assert!(matches!(
                state.next_envelope().await,
                Some(Err(ExplainabilitySseStreamError::SequenceInvariant))
            ));
        }
        Ok(())
    }

    #[derive(Debug)]
    struct CorruptStore {
        page: Vec<ExplainabilityEnvelope>,
    }

    #[derive(Debug)]
    struct FailingLoadStore;

    #[async_trait::async_trait]
    impl ExplainabilityStore for FailingLoadStore {
        async fn create_run(
            &self,
            _run: ExplainabilityRun,
        ) -> Result<(), ExplainabilityStoreError> {
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
            _completion: graphloom::explainability::RunCompletion,
        ) -> Result<(), ExplainabilityStoreError> {
            Ok(())
        }

        async fn get_run(
            &self,
            _run_id: &ExplainabilityRunId,
        ) -> Result<Option<ExplainabilityRun>, ExplainabilityStoreError> {
            Ok(None)
        }

        async fn list_runs(
            &self,
            _query: &graphloom::explainability::RunQuery,
        ) -> Result<Vec<ExplainabilityRun>, ExplainabilityStoreError> {
            Ok(Vec::new())
        }

        async fn load_events(
            &self,
            _run_id: &ExplainabilityRunId,
            _query: &graphloom::explainability::EventQuery,
        ) -> Result<Vec<ExplainabilityEnvelope>, ExplainabilityStoreError> {
            Err(ExplainabilityStoreError::InvalidLimit {
                kind: "SSE_EVENT_SECRET_SENTINEL",
                limit: 0,
                min: 1,
                max: 1,
            })
        }

        async fn delete_run(
            &self,
            _run_id: &ExplainabilityRunId,
        ) -> Result<(), ExplainabilityStoreError> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl ExplainabilityStore for CorruptStore {
        async fn create_run(
            &self,
            _run: ExplainabilityRun,
        ) -> Result<(), graphloom::explainability::ExplainabilityStoreError> {
            Ok(())
        }

        async fn append_events(
            &self,
            _events: &[ExplainabilityEnvelope],
        ) -> Result<(), graphloom::explainability::ExplainabilityStoreError> {
            Ok(())
        }

        async fn complete_run(
            &self,
            _completion: graphloom::explainability::RunCompletion,
        ) -> Result<(), graphloom::explainability::ExplainabilityStoreError> {
            Ok(())
        }

        async fn get_run(
            &self,
            _run_id: &ExplainabilityRunId,
        ) -> Result<Option<ExplainabilityRun>, graphloom::explainability::ExplainabilityStoreError>
        {
            Ok(None)
        }

        async fn list_runs(
            &self,
            _query: &graphloom::explainability::RunQuery,
        ) -> Result<Vec<ExplainabilityRun>, graphloom::explainability::ExplainabilityStoreError>
        {
            Ok(Vec::new())
        }

        async fn load_events(
            &self,
            _run_id: &ExplainabilityRunId,
            _query: &graphloom::explainability::EventQuery,
        ) -> Result<Vec<ExplainabilityEnvelope>, graphloom::explainability::ExplainabilityStoreError>
        {
            Ok(self.page.clone())
        }

        async fn delete_run(
            &self,
            _run_id: &ExplainabilityRunId,
        ) -> Result<(), graphloom::explainability::ExplainabilityStoreError> {
            Ok(())
        }
    }
}
