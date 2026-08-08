//! Public contract tests for the Store-backed explainability persistence
//! writer.
//!
//! The recorder owns one bounded queue and one writer task; tests use
//! deterministic probe stores (semaphore-gated and failing wrappers around
//! `InMemoryExplainabilityStore`) so queue backpressure, persistence
//! barriers, and failure propagation are verified without sleeps or file
//! timing.

use std::{
    error::Error,
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering},
    },
};

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
#[cfg(feature = "sqlite-store")]
use graphloom::explainability::SqliteExplainabilityStore;
use graphloom::explainability::{
    EventQuery, ExplainabilityContentMode, ExplainabilityEnvelope, ExplainabilityEvent,
    ExplainabilityQueryMethod, ExplainabilityRecord, ExplainabilityRun, ExplainabilityRunId,
    ExplainabilityRunKind, ExplainabilityRunStatus, ExplainabilitySinkError, ExplainabilityStore,
    ExplainabilityStoreError, InMemoryExplainabilityStore, QueryStarted, RunCompleted,
    RunCompletion, RunFailed, RunStarted, StoreExplainabilityError, StoreExplainabilityOptions,
    StoreExplainabilityRecorder,
};
#[cfg(feature = "sqlite-store")]
use tempfile::TempDir;
use tokio::sync::{Barrier, Semaphore};

type TestResult = Result<(), Box<dyn Error>>;

const QUERY_SECRET_SENTINEL: &str = "STORE_WRITER_QUERY_SECRET_SENTINEL";
const EVENT_SECRET_SENTINEL: &str = "STORE_WRITER_EVENT_SECRET_SENTINEL";
const COMPAT_SECRET_SENTINEL: &str = "STORE_WRITER_COMPAT_SECRET_SENTINEL";

fn run_id(value: &str) -> ExplainabilityRunId {
    value.parse().expect("run id")
}

fn span_id(value: &str) -> graphloom::explainability::ExplainabilitySpanId {
    value.parse().expect("span id")
}

fn timestamp(day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 2, day + hour / 24, hour % 24, 0, 0)
        .single()
        .expect("timestamp")
}

fn query_run(id: ExplainabilityRunId, started_at: DateTime<Utc>) -> ExplainabilityRun {
    let mut run = ExplainabilityRun::new(id, ExplainabilityRunKind::Query, started_at);
    run.status = ExplainabilityRunStatus::Running;
    run.query_method = Some(ExplainabilityQueryMethod::Local);
    run
}

fn record(
    id: ExplainabilityRunId,
    timestamp: DateTime<Utc>,
    event: ExplainabilityEvent,
) -> Arc<ExplainabilityRecord> {
    Arc::new(ExplainabilityRecord::new(
        id,
        timestamp,
        span_id("writer-span"),
        None,
        event,
    ))
}

fn run_started_record(
    id: ExplainabilityRunId,
    timestamp: DateTime<Utc>,
) -> Arc<ExplainabilityRecord> {
    record(
        id,
        timestamp,
        ExplainabilityEvent::RunStarted(RunStarted::new(
            ExplainabilityRunKind::Query,
            ExplainabilityContentMode::Metadata,
        )),
    )
}

fn query_started_record(
    id: ExplainabilityRunId,
    timestamp: DateTime<Utc>,
) -> Arc<ExplainabilityRecord> {
    record(
        id,
        timestamp,
        ExplainabilityEvent::QueryStarted(QueryStarted::new(ExplainabilityQueryMethod::Local)),
    )
}

fn run_completed_record(
    id: ExplainabilityRunId,
    timestamp: DateTime<Utc>,
) -> Arc<ExplainabilityRecord> {
    record(
        id,
        timestamp,
        ExplainabilityEvent::RunCompleted(RunCompleted::new(12)),
    )
}

fn run_failed_record(
    id: ExplainabilityRunId,
    timestamp: DateTime<Utc>,
) -> Arc<ExplainabilityRecord> {
    record(
        id,
        timestamp,
        ExplainabilityEvent::RunFailed(RunFailed::new(
            "query_error".to_owned(),
            "safe message".to_owned(),
        )),
    )
}

async fn sequences(store: &dyn ExplainabilityStore, id: &ExplainabilityRunId) -> Vec<u64> {
    store
        .load_events(id, &EventQuery::new())
        .await
        .expect("load events")
        .into_iter()
        .map(|envelope| envelope.sequence())
        .collect()
}

/// Deterministic probe store gating `create_run` and `append_events`.
#[derive(Debug)]
struct BlockingExplainabilityStore {
    inner: InMemoryExplainabilityStore,
    create_entered: Option<Arc<Semaphore>>,
    create_release: Option<Arc<Semaphore>>,
    append_entered: Option<Arc<Semaphore>>,
    append_release: Option<Arc<Semaphore>>,
}

impl BlockingExplainabilityStore {
    fn with_append_gate() -> Self {
        Self {
            inner: InMemoryExplainabilityStore::new(),
            create_entered: None,
            create_release: None,
            append_entered: Some(Arc::new(Semaphore::new(0))),
            append_release: Some(Arc::new(Semaphore::new(0))),
        }
    }

    fn with_create_gate() -> Self {
        Self {
            inner: InMemoryExplainabilityStore::new(),
            create_entered: Some(Arc::new(Semaphore::new(0))),
            create_release: Some(Arc::new(Semaphore::new(0))),
            append_entered: None,
            append_release: None,
        }
    }

    async fn maybe_block(entered: &Option<Arc<Semaphore>>, release: &Option<Arc<Semaphore>>) {
        if let (Some(entered), Some(release)) = (entered, release) {
            entered.add_permits(1);
            let _permit = release
                .acquire()
                .await
                .expect("release semaphore must not close");
        }
    }

    fn inner_store(&self) -> &InMemoryExplainabilityStore {
        &self.inner
    }
}

#[async_trait]
impl ExplainabilityStore for BlockingExplainabilityStore {
    async fn create_run(&self, run: ExplainabilityRun) -> Result<(), ExplainabilityStoreError> {
        Self::maybe_block(&self.create_entered, &self.create_release).await;
        self.inner.create_run(run).await
    }

    async fn append_events(
        &self,
        events: &[ExplainabilityEnvelope],
    ) -> Result<(), ExplainabilityStoreError> {
        Self::maybe_block(&self.append_entered, &self.append_release).await;
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
        self.inner.get_run(run_id).await
    }

    async fn list_runs(
        &self,
        query: &graphloom::explainability::RunQuery,
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

/// Deterministic failure store with one-shot failure flags.
#[derive(Debug)]
struct FailingExplainabilityStore {
    inner: InMemoryExplainabilityStore,
    fail_create: AtomicBool,
    fail_append_on: AtomicUsize,
    append_count: AtomicUsize,
    fail_complete: AtomicBool,
}

impl FailingExplainabilityStore {
    fn new() -> Self {
        Self {
            inner: InMemoryExplainabilityStore::new(),
            fail_create: AtomicBool::new(false),
            fail_append_on: AtomicUsize::new(0),
            append_count: AtomicUsize::new(0),
            fail_complete: AtomicBool::new(false),
        }
    }

    fn inner_store(&self) -> &InMemoryExplainabilityStore {
        &self.inner
    }
}

#[async_trait]
impl ExplainabilityStore for FailingExplainabilityStore {
    async fn create_run(&self, run: ExplainabilityRun) -> Result<(), ExplainabilityStoreError> {
        if self.fail_create.load(AtomicOrdering::Acquire) {
            return Err(ExplainabilityStoreError::RunAlreadyExists { run_id: run.run_id });
        }
        self.inner.create_run(run).await
    }

    async fn append_events(
        &self,
        events: &[ExplainabilityEnvelope],
    ) -> Result<(), ExplainabilityStoreError> {
        let attempt = self.append_count.fetch_add(1, AtomicOrdering::AcqRel) + 1;
        if self.fail_append_on.load(AtomicOrdering::Acquire) == attempt {
            let run_id = events
                .first()
                .map(|envelope| envelope.record.run_id.clone())
                .unwrap_or_else(|| run_id("missing-run"));
            return Err(ExplainabilityStoreError::RunNotFound { run_id });
        }
        self.inner.append_events(events).await
    }

    async fn complete_run(
        &self,
        completion: RunCompletion,
    ) -> Result<(), ExplainabilityStoreError> {
        if self.fail_complete.load(AtomicOrdering::Acquire) {
            return Err(ExplainabilityStoreError::CompletionConflict {
                run_id: completion.run_id().clone(),
            });
        }
        self.inner.complete_run(completion).await
    }

    async fn get_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<Option<ExplainabilityRun>, ExplainabilityStoreError> {
        self.inner.get_run(run_id).await
    }

    async fn list_runs(
        &self,
        query: &graphloom::explainability::RunQuery,
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

fn small_options() -> StoreExplainabilityOptions {
    StoreExplainabilityOptions::new().with_queue_capacity(NonZeroUsize::new(1).expect("fixture"))
}

async fn shutdown_shared(recorder: Arc<StoreExplainabilityRecorder>) -> TestResult {
    Arc::try_unwrap(recorder)
        .expect("recorder must have a single strong reference at shutdown")
        .shutdown()
        .await?;
    Ok(())
}

#[tokio::test]
async fn test_should_persist_create_emit_finish_complete_lifecycle() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder = Arc::new(StoreExplainabilityRecorder::new(
        Arc::clone(&store),
        StoreExplainabilityOptions::new(),
    ));
    let id = run_id("lifecycle");
    let mut run = query_run(id.clone(), timestamp(1, 8));
    run.query = Some("user query".to_owned());
    run.compatibility_profile = Some("compat-v1".to_owned());
    recorder.create_run(run.clone()).await?;
    assert_eq!(store.get_run(&id).await?.expect("run").event_count, 0);

    let events = vec![
        run_started_record(id.clone(), timestamp(1, 13)),
        query_started_record(id.clone(), timestamp(1, 11)),
        run_completed_record(id.clone(), timestamp(1, 12)),
    ];
    for event in &events {
        recorder.sink().emit(Arc::clone(event)).await?;
    }
    recorder.sink().finish_run(&id).await?;
    let completed_at = timestamp(1, 9);
    recorder
        .complete_run(RunCompletion::new(
            id.clone(),
            ExplainabilityRunStatus::Completed,
            completed_at,
        )?)
        .await?;

    let stored = store.get_run(&id).await?.expect("run");
    let mut expected = run;
    expected.status = ExplainabilityRunStatus::Completed;
    expected.completed_at = Some(completed_at);
    expected.event_count = 3;
    assert_eq!(stored, expected);
    assert_eq!(sequences(store.as_ref(), &id).await, vec![1, 2, 3]);
    assert_eq!(
        store.load_events(&id, &EventQuery::new()).await?,
        events
            .iter()
            .enumerate()
            .map(|(index, event)| {
                ExplainabilityEnvelope::new(
                    u64::try_from(index + 1).expect("fixture"),
                    event.as_ref().clone(),
                )
                .expect("envelope")
            })
            .collect::<Vec<_>>()
    );
    shutdown_shared(recorder).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_preserve_run_metadata_from_create_input() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder =
        StoreExplainabilityRecorder::new(Arc::clone(&store), StoreExplainabilityOptions::new());
    let id = run_id("metadata");
    let mut run = query_run(id.clone(), timestamp(2, 8));
    run.query = Some(QUERY_SECRET_SENTINEL.to_owned());
    run.compatibility_profile = Some(COMPAT_SECRET_SENTINEL.to_owned());
    recorder.create_run(run.clone()).await?;
    assert_eq!(store.get_run(&id).await?, Some(run.clone()));
    recorder
        .sink()
        .emit(run_started_record(id.clone(), timestamp(2, 10)))
        .await?;
    recorder.sink().finish_run(&id).await?;
    recorder
        .complete_run(RunCompletion::new(
            id.clone(),
            ExplainabilityRunStatus::Completed,
            timestamp(2, 9),
        )?)
        .await?;
    let stored = store.get_run(&id).await?.expect("run");
    assert_eq!(stored.query.as_deref(), Some(QUERY_SECRET_SENTINEL));
    assert_eq!(
        stored.compatibility_profile.as_deref(),
        Some(COMPAT_SECRET_SENTINEL)
    );
    recorder.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_should_allocate_independent_sequences_per_run() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder = Arc::new(StoreExplainabilityRecorder::new(
        Arc::clone(&store),
        StoreExplainabilityOptions::new(),
    ));
    let run_a = run_id("seq-a");
    let run_b = run_id("seq-b");
    recorder
        .create_run(query_run(run_a.clone(), timestamp(3, 8)))
        .await?;
    recorder
        .create_run(query_run(run_b.clone(), timestamp(3, 8)))
        .await?;
    let sink = recorder.sink();
    sink.emit(run_started_record(run_a.clone(), timestamp(3, 10)))
        .await?;
    sink.emit(run_started_record(run_b.clone(), timestamp(3, 10)))
        .await?;
    sink.emit(run_started_record(run_a.clone(), timestamp(3, 11)))
        .await?;
    sink.emit(run_started_record(run_b.clone(), timestamp(3, 11)))
        .await?;
    sink.emit(run_started_record(run_a.clone(), timestamp(3, 12)))
        .await?;
    sink.finish_run(&run_a).await?;
    sink.finish_run(&run_b).await?;

    assert_eq!(sequences(store.as_ref(), &run_a).await, vec![1, 2, 3]);
    assert_eq!(sequences(store.as_ref(), &run_b).await, vec![1, 2]);
    assert_eq!(store.get_run(&run_a).await?.expect("run a").event_count, 3);
    assert_eq!(store.get_run(&run_b).await?.expect("run b").event_count, 2);
    shutdown_shared(recorder).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_preserve_queue_acceptance_order_ignoring_timestamps() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder = Arc::new(StoreExplainabilityRecorder::new(
        Arc::clone(&store),
        StoreExplainabilityOptions::new(),
    ));
    let id = run_id("order");
    recorder
        .create_run(query_run(id.clone(), timestamp(4, 8)))
        .await?;
    let sink = recorder.sink();
    sink.emit(run_started_record(id.clone(), timestamp(4, 13)))
        .await?;
    sink.emit(run_started_record(id.clone(), timestamp(4, 11)))
        .await?;
    sink.emit(run_started_record(id.clone(), timestamp(4, 12)))
        .await?;
    sink.finish_run(&id).await?;
    assert_eq!(sequences(store.as_ref(), &id).await, vec![1, 2, 3]);
    shutdown_shared(recorder).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_assign_sequence_from_writer_not_record() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder = Arc::new(StoreExplainabilityRecorder::new(
        Arc::clone(&store),
        StoreExplainabilityOptions::new(),
    ));
    let id = run_id("writer-sequence");
    recorder
        .create_run(query_run(id.clone(), timestamp(5, 8)))
        .await?;
    let duplicate = run_started_record(id.clone(), timestamp(5, 10));
    recorder.sink().emit(Arc::clone(&duplicate)).await?;
    recorder.sink().emit(Arc::clone(&duplicate)).await?;
    recorder.sink().finish_run(&id).await?;
    assert_eq!(sequences(store.as_ref(), &id).await, vec![1, 2]);
    shutdown_shared(recorder).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_should_serialize_concurrent_emits_without_gaps_or_duplicates() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder = Arc::new(StoreExplainabilityRecorder::new(
        Arc::clone(&store),
        StoreExplainabilityOptions::new(),
    ));
    let id = run_id("concurrent-emits");
    recorder
        .create_run(query_run(id.clone(), timestamp(6, 8)))
        .await?;
    let sink = recorder.sink();
    let mut tasks = Vec::new();
    for index in 0..100_u64 {
        let sink = Arc::clone(&sink);
        let id = id.clone();
        tasks.push(tokio::spawn(async move {
            sink.emit(run_started_record(
                id,
                timestamp(6, (index % 20) as u32 + 9),
            ))
            .await
        }));
    }
    for task in tasks {
        task.await??;
    }
    recorder.sink().finish_run(&id).await?;
    let mut stored = sequences(store.as_ref(), &id).await;
    stored.sort_unstable();
    assert_eq!(stored, (1..=100).collect::<Vec<_>>());
    assert_eq!(store.get_run(&id).await?.expect("run").event_count, 100);
    shutdown_shared(recorder).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_should_apply_bounded_backpressure_without_dropping_records() -> TestResult {
    let store = Arc::new(BlockingExplainabilityStore::with_append_gate());
    let recorder = Arc::new(StoreExplainabilityRecorder::new(
        Arc::clone(&store) as Arc<dyn ExplainabilityStore>,
        small_options(),
    ));
    let id = run_id("backpressure");
    recorder
        .create_run(query_run(id.clone(), timestamp(7, 8)))
        .await?;
    let sink = recorder.sink();
    sink.emit(run_started_record(id.clone(), timestamp(7, 10)))
        .await?;
    let append_entered = store.append_entered.as_ref().expect("gate").clone();
    let _entered = append_entered.acquire().await?;
    sink.emit(run_started_record(id.clone(), timestamp(7, 11)))
        .await?;

    let started = Arc::new(Semaphore::new(0));
    let done = Arc::new(Semaphore::new(0));
    let third_sink = Arc::clone(&sink);
    let third_record = run_started_record(id.clone(), timestamp(7, 12));
    let third_started = Arc::clone(&started);
    let third_done = Arc::clone(&done);
    let third = tokio::spawn(async move {
        third_started.add_permits(1);
        let result = third_sink.emit(third_record).await;
        third_done.add_permits(1);
        result
    });
    let _third_started = started.acquire().await?;
    assert!(
        done.try_acquire().is_err(),
        "third record must not complete while the queue is full and the writer is blocked"
    );
    store
        .append_release
        .as_ref()
        .expect("release")
        .add_permits(3);
    third.await??;
    recorder.sink().finish_run(&id).await?;
    assert_eq!(
        store
            .inner_store()
            .get_run(&id)
            .await?
            .expect("run")
            .event_count,
        3
    );
    assert_eq!(
        sequences(store.inner_store(), &id).await,
        vec![1, 2, 3],
        "no record may be silently dropped"
    );
    shutdown_shared(recorder).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_should_make_finish_run_a_persistence_barrier() -> TestResult {
    let store = Arc::new(BlockingExplainabilityStore::with_append_gate());
    let recorder = Arc::new(StoreExplainabilityRecorder::new(
        Arc::clone(&store) as Arc<dyn ExplainabilityStore>,
        StoreExplainabilityOptions::new(),
    ));
    let id = run_id("finish-barrier");
    recorder
        .create_run(query_run(id.clone(), timestamp(8, 8)))
        .await?;
    recorder
        .sink()
        .emit(run_started_record(id.clone(), timestamp(8, 10)))
        .await?;
    let append_entered = store.append_entered.as_ref().expect("gate").clone();
    let _entered = append_entered.acquire().await?;

    let finish_done = Arc::new(Semaphore::new(0));
    let finish_sink = recorder.sink();
    let finish_id = id.clone();
    let finish_done_probe = Arc::clone(&finish_done);
    let finish = tokio::spawn(async move {
        let result = finish_sink.finish_run(&finish_id).await;
        finish_done_probe.add_permits(1);
        result
    });
    assert!(
        finish_done.try_acquire().is_err(),
        "finish_run must not confirm before the accepted record is persisted"
    );
    store
        .append_release
        .as_ref()
        .expect("release")
        .add_permits(1);
    finish.await??;
    assert_eq!(
        store
            .inner_store()
            .get_run(&id)
            .await?
            .expect("run")
            .event_count,
        1
    );
    shutdown_shared(recorder).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_reject_emit_before_create() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder =
        StoreExplainabilityRecorder::new(Arc::clone(&store), StoreExplainabilityOptions::new());
    let id = run_id("emit-before-create");
    let error = recorder
        .sink()
        .emit(run_started_record(id.clone(), timestamp(9, 10)))
        .await
        .expect_err("emit before create must be rejected");
    assert_eq!(error, ExplainabilitySinkError::RecordNotAccepted);
    assert_eq!(store.get_run(&id).await?, None);
    assert!(matches!(
        store.load_events(&id, &EventQuery::new()).await,
        Err(ExplainabilityStoreError::RunNotFound { .. })
    ));
    recorder.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_should_reject_emit_while_create_is_in_flight() -> TestResult {
    let store = Arc::new(BlockingExplainabilityStore::with_create_gate());
    let recorder = Arc::new(StoreExplainabilityRecorder::new(
        Arc::clone(&store) as Arc<dyn ExplainabilityStore>,
        StoreExplainabilityOptions::new(),
    ));
    let id = run_id("emit-during-create");
    let create_recorder = Arc::clone(&recorder);
    let create_run = query_run(id.clone(), timestamp(10, 8));
    let create = tokio::spawn(async move { create_recorder.create_run(create_run).await });
    let create_entered = store.create_entered.as_ref().expect("gate").clone();
    let _entered = create_entered.acquire().await?;

    let error = recorder
        .sink()
        .emit(run_started_record(id.clone(), timestamp(10, 10)))
        .await
        .expect_err("emit during create must be rejected");
    assert_eq!(error, ExplainabilitySinkError::RecordNotAccepted);

    store
        .create_release
        .as_ref()
        .expect("release")
        .add_permits(1);
    create.await??;
    recorder
        .sink()
        .emit(run_started_record(id.clone(), timestamp(10, 11)))
        .await?;
    recorder.sink().finish_run(&id).await?;
    assert_eq!(sequences(store.inner_store(), &id).await, vec![1]);
    shutdown_shared(recorder).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_reject_emit_after_finish() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder =
        StoreExplainabilityRecorder::new(Arc::clone(&store), StoreExplainabilityOptions::new());
    let id = run_id("emit-after-finish");
    recorder
        .create_run(query_run(id.clone(), timestamp(11, 8)))
        .await?;
    recorder
        .sink()
        .emit(run_started_record(id.clone(), timestamp(11, 10)))
        .await?;
    recorder.sink().finish_run(&id).await?;
    let error = recorder
        .sink()
        .emit(run_started_record(id.clone(), timestamp(11, 11)))
        .await
        .expect_err("emit after finish must be rejected");
    assert_eq!(error, ExplainabilitySinkError::RecordNotAccepted);
    assert_eq!(sequences(store.as_ref(), &id).await, vec![1]);
    recorder.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_should_linearize_finish_and_emit_without_late_persistence() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder = Arc::new(StoreExplainabilityRecorder::new(
        Arc::clone(&store),
        StoreExplainabilityOptions::new(),
    ));
    let id = run_id("finish-emit-race");
    recorder
        .create_run(query_run(id.clone(), timestamp(12, 8)))
        .await?;
    let barrier = Arc::new(Barrier::new(3));
    let sink_a = recorder.sink();
    let emit_task = {
        let barrier = Arc::clone(&barrier);
        let sink = Arc::clone(&sink_a);
        let id = id.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            sink.emit(run_started_record(id, timestamp(12, 10))).await
        })
    };
    let finish_task = {
        let barrier = Arc::clone(&barrier);
        let sink = recorder.sink();
        let id = id.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            sink.finish_run(&id).await
        })
    };
    barrier.wait().await;
    let emit_result = emit_task.await?;
    let finish_result = finish_task.await?;
    let event_count = store.get_run(&id).await?.expect("run").event_count;
    match (&emit_result, &finish_result) {
        (Ok(()), Ok(())) => assert_eq!(event_count, 1),
        (Err(ExplainabilitySinkError::RecordNotAccepted), Ok(())) => assert_eq!(event_count, 0),
        (emit, finish) => panic!("unexpected linearization: emit={emit:?} finish={finish:?}"),
    }
    shutdown_shared(recorder).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_should_make_finish_idempotent() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder = Arc::new(StoreExplainabilityRecorder::new(
        Arc::clone(&store),
        StoreExplainabilityOptions::new(),
    ));
    let id = run_id("finish-idempotent");
    recorder
        .create_run(query_run(id.clone(), timestamp(13, 8)))
        .await?;
    recorder
        .sink()
        .emit(run_started_record(id.clone(), timestamp(13, 10)))
        .await?;
    recorder.sink().finish_run(&id).await?;
    recorder.sink().finish_run(&id).await?;

    let first = {
        let sink = recorder.sink();
        let id = id.clone();
        tokio::spawn(async move { sink.finish_run(&id).await })
    };
    let second = {
        let sink = recorder.sink();
        let id = id.clone();
        tokio::spawn(async move { sink.finish_run(&id).await })
    };
    first.await??;
    second.await??;
    shutdown_shared(recorder).await?;
    Ok(())
}

#[tokio::test]
async fn test_should_reject_complete_before_finish() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder =
        StoreExplainabilityRecorder::new(Arc::clone(&store), StoreExplainabilityOptions::new());
    let id = run_id("complete-before-finish");
    recorder
        .create_run(query_run(id.clone(), timestamp(14, 8)))
        .await?;
    recorder
        .sink()
        .emit(run_started_record(id.clone(), timestamp(14, 10)))
        .await?;
    let error = recorder
        .complete_run(RunCompletion::new(
            id.clone(),
            ExplainabilityRunStatus::Completed,
            timestamp(14, 9),
        )?)
        .await
        .expect_err("complete before finish must be rejected");
    assert!(matches!(error, StoreExplainabilityError::RunNotFinalized));
    let run = store.get_run(&id).await?.expect("run");
    assert_eq!(run.status, ExplainabilityRunStatus::Running);
    assert_eq!(run.completed_at, None);

    recorder.sink().finish_run(&id).await?;
    recorder
        .complete_run(RunCompletion::new(
            id.clone(),
            ExplainabilityRunStatus::Completed,
            timestamp(14, 9),
        )?)
        .await?;
    recorder.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_should_not_derive_run_status_from_events() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder =
        StoreExplainabilityRecorder::new(Arc::clone(&store), StoreExplainabilityOptions::new());
    let completed_run = run_id("event-completed");
    let failed_run = run_id("event-failed");
    recorder
        .create_run(query_run(completed_run.clone(), timestamp(15, 8)))
        .await?;
    recorder
        .create_run(query_run(failed_run.clone(), timestamp(15, 8)))
        .await?;
    recorder
        .sink()
        .emit(run_completed_record(
            completed_run.clone(),
            timestamp(15, 10),
        ))
        .await?;
    recorder
        .sink()
        .emit(run_failed_record(failed_run.clone(), timestamp(15, 10)))
        .await?;
    recorder.sink().finish_run(&completed_run).await?;
    recorder.sink().finish_run(&failed_run).await?;

    let after_events = store.get_run(&completed_run).await?.expect("completed");
    assert_eq!(after_events.status, ExplainabilityRunStatus::Running);
    assert_eq!(after_events.completed_at, None);
    let failed_after = store.get_run(&failed_run).await?.expect("failed");
    assert_eq!(failed_after.status, ExplainabilityRunStatus::Running);
    assert_eq!(failed_after.completed_at, None);

    recorder
        .complete_run(RunCompletion::new(
            completed_run.clone(),
            ExplainabilityRunStatus::Completed,
            timestamp(15, 9),
        )?)
        .await?;
    recorder
        .complete_run(RunCompletion::new(
            failed_run.clone(),
            ExplainabilityRunStatus::Failed,
            timestamp(15, 9),
        )?)
        .await?;
    assert_eq!(
        store.get_run(&completed_run).await?.expect("run").status,
        ExplainabilityRunStatus::Completed
    );
    assert_eq!(
        store.get_run(&failed_run).await?.expect("run").status,
        ExplainabilityRunStatus::Failed
    );
    recorder.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_should_apply_completion_retry_and_conflict() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder =
        StoreExplainabilityRecorder::new(Arc::clone(&store), StoreExplainabilityOptions::new());
    let id = run_id("completion-retry");
    recorder
        .create_run(query_run(id.clone(), timestamp(16, 8)))
        .await?;
    recorder
        .sink()
        .emit(run_started_record(id.clone(), timestamp(16, 10)))
        .await?;
    recorder.sink().finish_run(&id).await?;
    let first = timestamp(16, 9);
    recorder
        .complete_run(RunCompletion::new(
            id.clone(),
            ExplainabilityRunStatus::Completed,
            first,
        )?)
        .await?;
    recorder
        .complete_run(RunCompletion::new(
            id.clone(),
            ExplainabilityRunStatus::Completed,
            first,
        )?)
        .await?;

    let status_conflict = recorder
        .complete_run(RunCompletion::new(
            id.clone(),
            ExplainabilityRunStatus::Failed,
            first,
        )?)
        .await
        .expect_err("different terminal status must conflict");
    assert!(matches!(
        status_conflict,
        StoreExplainabilityError::Store {
            operation: graphloom::explainability::StoreExplainabilityOperation::CompleteRun,
            source: ExplainabilityStoreError::CompletionConflict { .. },
        }
    ));
    let time_conflict = recorder
        .complete_run(RunCompletion::new(
            id.clone(),
            ExplainabilityRunStatus::Completed,
            timestamp(16, 10),
        )?)
        .await
        .expect_err("different completion time must conflict");
    assert!(matches!(
        time_conflict,
        StoreExplainabilityError::Store { .. }
    ));
    let stored = store.get_run(&id).await?.expect("run");
    assert_eq!(stored.status, ExplainabilityRunStatus::Completed);
    assert_eq!(stored.completed_at, Some(first));

    let other = run_id("completion-other");
    recorder
        .create_run(query_run(other.clone(), timestamp(16, 8)))
        .await?;
    recorder.sink().finish_run(&other).await?;
    recorder
        .complete_run(RunCompletion::new(
            other.clone(),
            ExplainabilityRunStatus::Completed,
            timestamp(16, 9),
        )?)
        .await?;
    recorder.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_should_reject_duplicate_create_without_killing_writer() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder =
        StoreExplainabilityRecorder::new(Arc::clone(&store), StoreExplainabilityOptions::new());
    let run_a = run_id("duplicate-a");
    recorder
        .create_run(query_run(run_a.clone(), timestamp(17, 8)))
        .await?;
    let error = recorder
        .create_run(query_run(run_a.clone(), timestamp(17, 9)))
        .await
        .expect_err("duplicate create must be rejected");
    assert!(matches!(
        error,
        StoreExplainabilityError::RunAlreadyRegistered
    ));

    let run_b = run_id("duplicate-b");
    recorder
        .create_run(query_run(run_b.clone(), timestamp(17, 8)))
        .await?;
    recorder
        .sink()
        .emit(run_started_record(run_b.clone(), timestamp(17, 10)))
        .await?;
    recorder.sink().finish_run(&run_b).await?;
    assert_eq!(store.get_run(&run_b).await?.expect("run").event_count, 1);
    recorder.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_should_keep_writer_alive_after_create_failure() -> TestResult {
    let store = Arc::new(FailingExplainabilityStore::new());
    store.fail_create.store(true, AtomicOrdering::Release);
    let recorder = StoreExplainabilityRecorder::new(
        Arc::clone(&store) as Arc<dyn ExplainabilityStore>,
        StoreExplainabilityOptions::new(),
    );
    let run_a = run_id("create-failure-a");
    let error = recorder
        .create_run(query_run(run_a.clone(), timestamp(18, 8)))
        .await
        .expect_err("create failure must surface");
    assert!(matches!(
        error,
        StoreExplainabilityError::Store {
            operation: graphloom::explainability::StoreExplainabilityOperation::CreateRun,
            ..
        }
    ));
    store.fail_create.store(false, AtomicOrdering::Release);

    let run_b = run_id("create-failure-b");
    recorder
        .create_run(query_run(run_b.clone(), timestamp(18, 8)))
        .await?;
    recorder
        .sink()
        .emit(run_started_record(run_b.clone(), timestamp(18, 10)))
        .await?;
    recorder.sink().finish_run(&run_b).await?;
    assert_eq!(
        store
            .inner_store()
            .get_run(&run_b)
            .await?
            .expect("run")
            .event_count,
        1
    );
    recorder.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_should_fail_writer_on_append_failure_after_acceptance() -> TestResult {
    let store = Arc::new(FailingExplainabilityStore::new());
    let recorder = StoreExplainabilityRecorder::new(
        Arc::clone(&store) as Arc<dyn ExplainabilityStore>,
        StoreExplainabilityOptions::new(),
    );
    let id = run_id("append-failure");
    recorder
        .create_run(query_run(id.clone(), timestamp(19, 8)))
        .await?;
    store.fail_append_on.store(2, AtomicOrdering::Release);
    recorder
        .sink()
        .emit(run_started_record(id.clone(), timestamp(19, 10)))
        .await?;
    recorder
        .sink()
        .emit(run_started_record(id.clone(), timestamp(19, 11)))
        .await?;
    let finish_error = recorder
        .sink()
        .finish_run(&id)
        .await
        .expect_err("writer failure must surface through finish");
    assert_eq!(finish_error, ExplainabilitySinkError::WriterFailed);

    let shutdown_error = recorder
        .shutdown()
        .await
        .expect_err("shutdown must surface the root writer failure");
    assert!(matches!(
        shutdown_error,
        StoreExplainabilityError::Store {
            operation: graphloom::explainability::StoreExplainabilityOperation::AppendEvents,
            source: ExplainabilityStoreError::RunNotFound { .. },
        }
    ));
    assert_eq!(
        store
            .inner_store()
            .get_run(&id)
            .await?
            .expect("run")
            .event_count,
        1
    );
    assert_eq!(sequences(store.inner_store(), &id).await, vec![1]);
    Ok(())
}

#[tokio::test]
async fn test_should_fail_writer_on_external_terminal_mismatch() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder =
        StoreExplainabilityRecorder::new(Arc::clone(&store), StoreExplainabilityOptions::new());
    let id = run_id("external-terminal");
    recorder
        .create_run(query_run(id.clone(), timestamp(20, 8)))
        .await?;
    store
        .complete_run(RunCompletion::new(
            id.clone(),
            ExplainabilityRunStatus::Completed,
            timestamp(20, 9),
        )?)
        .await?;
    recorder
        .sink()
        .emit(run_started_record(id.clone(), timestamp(20, 10)))
        .await?;
    let finish_error = recorder
        .sink()
        .finish_run(&id)
        .await
        .expect_err("accepted record against terminal store run must fail the writer");
    assert_eq!(finish_error, ExplainabilitySinkError::WriterFailed);
    let shutdown_error = recorder.shutdown().await.expect_err("root failure");
    assert!(matches!(
        shutdown_error,
        StoreExplainabilityError::Store {
            operation: graphloom::explainability::StoreExplainabilityOperation::AppendEvents,
            source: ExplainabilityStoreError::RunAlreadyTerminal { .. },
        }
    ));
    assert_eq!(sequences(store.as_ref(), &id).await, Vec::<u64>::new());
    Ok(())
}

#[tokio::test]
async fn test_should_fail_writer_on_external_delete() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder =
        StoreExplainabilityRecorder::new(Arc::clone(&store), StoreExplainabilityOptions::new());
    let id = run_id("external-delete");
    recorder
        .create_run(query_run(id.clone(), timestamp(21, 8)))
        .await?;
    store.delete_run(&id).await?;
    recorder
        .sink()
        .emit(run_started_record(id.clone(), timestamp(21, 10)))
        .await?;
    let finish_error = recorder
        .sink()
        .finish_run(&id)
        .await
        .expect_err("accepted record against deleted store run must fail the writer");
    assert_eq!(finish_error, ExplainabilitySinkError::WriterFailed);
    let shutdown_error = recorder.shutdown().await.expect_err("root failure");
    assert!(matches!(
        shutdown_error,
        StoreExplainabilityError::Store {
            operation: graphloom::explainability::StoreExplainabilityOperation::AppendEvents,
            source: ExplainabilityStoreError::RunNotFound { .. },
        }
    ));
    Ok(())
}

#[tokio::test]
async fn test_should_drain_on_shutdown_without_implicit_terminal() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder =
        StoreExplainabilityRecorder::new(Arc::clone(&store), StoreExplainabilityOptions::new());
    let id = run_id("shutdown-prefix");
    recorder
        .create_run(query_run(id.clone(), timestamp(22, 8)))
        .await?;
    recorder
        .sink()
        .emit(run_started_record(id.clone(), timestamp(22, 10)))
        .await?;
    recorder
        .sink()
        .emit(run_started_record(id.clone(), timestamp(22, 11)))
        .await?;
    recorder.shutdown().await?;

    let run = store.get_run(&id).await?.expect("run");
    assert_eq!(run.status, ExplainabilityRunStatus::Running);
    assert_eq!(run.completed_at, None);
    assert_eq!(run.event_count, 2);
    assert_eq!(sequences(store.as_ref(), &id).await, vec![1, 2]);
    Ok(())
}

#[tokio::test]
async fn test_should_reject_sink_after_shutdown() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder =
        StoreExplainabilityRecorder::new(Arc::clone(&store), StoreExplainabilityOptions::new());
    let id = run_id("sink-after-shutdown");
    recorder
        .create_run(query_run(id.clone(), timestamp(23, 8)))
        .await?;
    recorder
        .sink()
        .emit(run_started_record(id.clone(), timestamp(23, 10)))
        .await?;
    recorder.sink().finish_run(&id).await?;
    let sink = recorder.sink();
    recorder.shutdown().await?;

    let emit_error = sink
        .emit(run_started_record(id.clone(), timestamp(23, 11)))
        .await
        .expect_err("emit after shutdown must fail");
    assert_eq!(emit_error, ExplainabilitySinkError::Closed);
    assert_eq!(sink.finish_run(&id).await, Ok(()));
    let missing = run_id("missing-after-shutdown");
    assert_eq!(
        sink.finish_run(&missing).await,
        Err(ExplainabilitySinkError::RunFinalizationFailed)
    );
    Ok(())
}

#[tokio::test]
async fn test_should_match_jsonl_envelope_round_trip() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let recorder =
        StoreExplainabilityRecorder::new(Arc::clone(&store), StoreExplainabilityOptions::new());
    let id = run_id("jsonl-parity");
    recorder
        .create_run(query_run(id.clone(), timestamp(24, 8)))
        .await?;
    let records = vec![
        run_started_record(id.clone(), timestamp(24, 13)),
        query_started_record(id.clone(), timestamp(24, 11)),
        run_completed_record(id.clone(), timestamp(24, 12)),
    ];
    for record in &records {
        recorder.sink().emit(Arc::clone(record)).await?;
    }
    recorder.sink().finish_run(&id).await?;

    let stored = store.load_events(&id, &EventQuery::new()).await?;
    let mut jsonl = Vec::new();
    for (index, record) in records.iter().enumerate() {
        let envelope = ExplainabilityEnvelope::new(
            u64::try_from(index + 1).expect("fixture"),
            record.as_ref().clone(),
        )?;
        let mut line = serde_json::to_vec(&envelope)?;
        line.push(b'\n');
        jsonl.push(serde_json::from_slice::<ExplainabilityEnvelope>(&line)?);
    }
    assert_eq!(stored, jsonl);
    recorder.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_should_keep_errors_and_debug_free_of_secrets() -> TestResult {
    let store = Arc::new(FailingExplainabilityStore::new());
    store.fail_create.store(true, AtomicOrdering::Release);
    let recorder = StoreExplainabilityRecorder::new(
        Arc::clone(&store) as Arc<dyn ExplainabilityStore>,
        StoreExplainabilityOptions::new(),
    );
    let id = run_id("content-safe");
    let mut run = query_run(id.clone(), timestamp(25, 8));
    run.query = Some(QUERY_SECRET_SENTINEL.to_owned());
    run.compatibility_profile = Some(COMPAT_SECRET_SENTINEL.to_owned());
    let create_error = recorder.create_run(run).await.expect_err("create failure");
    let message = create_error.to_string();
    assert!(!message.contains(QUERY_SECRET_SENTINEL));
    assert!(!message.contains(COMPAT_SECRET_SENTINEL));
    assert!(!message.contains(EVENT_SECRET_SENTINEL));
    store.fail_create.store(false, AtomicOrdering::Release);

    recorder
        .create_run(query_run(id.clone(), timestamp(25, 8)))
        .await?;
    store.fail_append_on.store(1, AtomicOrdering::Release);
    let mut event = QueryStarted::new(ExplainabilityQueryMethod::Local);
    event.query = Some(EVENT_SECRET_SENTINEL.to_owned());
    recorder
        .sink()
        .emit(record(
            id.clone(),
            timestamp(25, 10),
            ExplainabilityEvent::QueryStarted(event),
        ))
        .await?;
    let finish_error = recorder
        .sink()
        .finish_run(&id)
        .await
        .expect_err("writer failure");
    assert_eq!(finish_error, ExplainabilitySinkError::WriterFailed);
    assert!(!finish_error.to_string().contains(EVENT_SECRET_SENTINEL));
    let shutdown_error = recorder.shutdown().await.expect_err("root failure");
    let message = shutdown_error.to_string();
    assert!(!message.contains(QUERY_SECRET_SENTINEL));
    assert!(!message.contains(EVENT_SECRET_SENTINEL));
    assert!(!message.contains(COMPAT_SECRET_SENTINEL));
    Ok(())
}

#[cfg(feature = "sqlite-store")]
#[tokio::test]
async fn test_should_support_store_recorder_against_sqlite() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("writer.sqlite");
    let id = run_id("sqlite-writer");
    {
        let store: Arc<dyn ExplainabilityStore> =
            Arc::new(SqliteExplainabilityStore::open(&path).await?);
        let recorder =
            StoreExplainabilityRecorder::new(Arc::clone(&store), StoreExplainabilityOptions::new());
        recorder
            .create_run(query_run(id.clone(), timestamp(26, 8)))
            .await?;
        for sequence in 1..=3_u64 {
            recorder
                .sink()
                .emit(run_started_record(
                    id.clone(),
                    timestamp(26, sequence as u32 + 9),
                ))
                .await?;
        }
        recorder.sink().finish_run(&id).await?;
        recorder
            .complete_run(RunCompletion::new(
                id.clone(),
                ExplainabilityRunStatus::Completed,
                timestamp(26, 9),
            )?)
            .await?;
        recorder.shutdown().await?;
    }

    let reopened: Arc<dyn ExplainabilityStore> =
        Arc::new(SqliteExplainabilityStore::open(&path).await?);
    let run = reopened.get_run(&id).await?.expect("run");
    assert_eq!(run.status, ExplainabilityRunStatus::Completed);
    assert_eq!(run.event_count, 3);
    assert_eq!(sequences(reopened.as_ref(), &id).await, vec![1, 2, 3]);
    Ok(())
}
