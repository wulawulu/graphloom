//! Public integration tests for post-persistence explainability live delivery.

use std::{error::Error, sync::Arc};

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
#[cfg(feature = "sqlite-store")]
use graphloom::explainability::SqliteExplainabilityStore;
use graphloom::explainability::{
    EventQuery, ExplainabilityContentMode, ExplainabilityEnvelope, ExplainabilityEvent,
    ExplainabilityLiveHub, ExplainabilityLiveHubOptions, ExplainabilityLiveRecvError,
    ExplainabilityQueryMethod, ExplainabilityRecord, ExplainabilityRun, ExplainabilityRunId,
    ExplainabilityRunKind, ExplainabilityRunStatus, ExplainabilitySinkError, ExplainabilityStore,
    ExplainabilityStoreError, InMemoryExplainabilityStore, RunCompleted, RunCompletion, RunFailed,
    RunQuery, RunStarted, StoreExplainabilityError, StoreExplainabilityOptions,
    StoreExplainabilityRecorder,
};
#[cfg(feature = "sqlite-store")]
use tempfile::TempDir;
use tokio::sync::Semaphore;

type TestResult = Result<(), Box<dyn Error>>;

fn run_id(value: &str) -> ExplainabilityRunId {
    value.parse().expect("run id")
}

fn timestamp(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 8, hour, 0, 0)
        .single()
        .expect("timestamp")
}

fn query_run(id: ExplainabilityRunId) -> ExplainabilityRun {
    let mut run = ExplainabilityRun::new(id, ExplainabilityRunKind::Query, timestamp(8));
    run.status = ExplainabilityRunStatus::Running;
    run.query_method = Some(ExplainabilityQueryMethod::Local);
    run
}

fn record(
    id: ExplainabilityRunId,
    event_timestamp: DateTime<Utc>,
    event: ExplainabilityEvent,
) -> Arc<ExplainabilityRecord> {
    Arc::new(ExplainabilityRecord::new(
        id,
        event_timestamp,
        "live-span".parse().expect("span id"),
        None,
        event,
    ))
}

fn started_record(
    id: ExplainabilityRunId,
    event_timestamp: DateTime<Utc>,
) -> Arc<ExplainabilityRecord> {
    record(
        id,
        event_timestamp,
        ExplainabilityEvent::RunStarted(RunStarted::new(
            ExplainabilityRunKind::Query,
            ExplainabilityContentMode::Metadata,
        )),
    )
}

fn recorder_with_hub(
    store: Arc<dyn ExplainabilityStore>,
    hub: Arc<ExplainabilityLiveHub>,
) -> StoreExplainabilityRecorder {
    StoreExplainabilityRecorder::new_with_live_hub(store, hub, StoreExplainabilityOptions::new())
        .expect("recorder")
}

#[derive(Debug)]
struct BlockingAppendStore {
    inner: InMemoryExplainabilityStore,
    append_entered: Arc<Semaphore>,
    append_release: Arc<Semaphore>,
}

impl BlockingAppendStore {
    fn new() -> Self {
        Self {
            inner: InMemoryExplainabilityStore::new(),
            append_entered: Arc::new(Semaphore::new(0)),
            append_release: Arc::new(Semaphore::new(0)),
        }
    }
}

#[async_trait]
impl ExplainabilityStore for BlockingAppendStore {
    async fn create_run(&self, run: ExplainabilityRun) -> Result<(), ExplainabilityStoreError> {
        self.inner.create_run(run).await
    }

    async fn append_events(
        &self,
        events: &[ExplainabilityEnvelope],
    ) -> Result<(), ExplainabilityStoreError> {
        self.append_entered.add_permits(1);
        let _permit = self
            .append_release
            .acquire()
            .await
            .expect("test gate remains open");
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

#[derive(Debug)]
struct FailingAppendStore {
    inner: InMemoryExplainabilityStore,
}

#[async_trait]
impl ExplainabilityStore for FailingAppendStore {
    async fn create_run(&self, run: ExplainabilityRun) -> Result<(), ExplainabilityStoreError> {
        self.inner.create_run(run).await
    }

    async fn append_events(
        &self,
        events: &[ExplainabilityEnvelope],
    ) -> Result<(), ExplainabilityStoreError> {
        let run_id = events
            .first()
            .map(|event| event.record.run_id.clone())
            .unwrap_or_else(|| run_id("missing"));
        Err(ExplainabilityStoreError::RunNotFound { run_id })
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

#[test]
fn test_should_match_runtime_unavailable_behavior_with_live_hub() {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new(),
    ));
    let result = StoreExplainabilityRecorder::new_with_live_hub(
        store,
        hub,
        StoreExplainabilityOptions::new(),
    );
    assert!(matches!(
        result,
        Err(StoreExplainabilityError::RuntimeUnavailable)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_should_publish_only_after_store_commit_and_sequence_commit() -> TestResult {
    let store = Arc::new(BlockingAppendStore::new());
    let hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new(),
    ));
    let recorder = recorder_with_hub(
        Arc::clone(&store) as Arc<dyn ExplainabilityStore>,
        Arc::clone(&hub),
    );
    let id = run_id("commit-boundary");
    recorder.create_run(query_run(id.clone())).await?;
    let mut first = hub
        .subscribe(&id)
        .expect("registered after create acknowledgement");
    recorder
        .sink()
        .emit(started_record(id.clone(), timestamp(12)))
        .await?;
    let _entered = store.append_entered.acquire().await?;

    let second = hub.subscribe(&id).expect("still active");
    assert_eq!(second.snapshot_sequence(), 0);
    store.append_release.add_permits(1);
    assert_eq!(first.recv().await?.sequence(), 1);
    assert_eq!(
        hub.subscribe(&id)
            .expect("still active")
            .snapshot_sequence(),
        1
    );

    recorder.sink().finish_run(&id).await?;
    recorder.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_should_preserve_store_live_identity_order_and_finish_lifecycle() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new(),
    ));
    let recorder = recorder_with_hub(Arc::clone(&store), Arc::clone(&hub));
    let id = run_id("parity");
    recorder.create_run(query_run(id.clone())).await?;
    let mut subscription = hub.subscribe(&id).expect("active");
    let records = [
        started_record(id.clone(), timestamp(13)),
        started_record(id.clone(), timestamp(9)),
        started_record(id.clone(), timestamp(11)),
    ];
    for item in records {
        recorder.sink().emit(item).await?;
    }
    recorder.sink().finish_run(&id).await?;

    let mut live = Vec::new();
    for _ in 0..3 {
        live.push(subscription.recv().await?.as_ref().clone());
    }
    assert_eq!(
        subscription.recv().await,
        Err(ExplainabilityLiveRecvError::Closed)
    );
    assert!(hub.subscribe(&id).is_none());
    let persisted = store.load_events(&id, &EventQuery::new()).await?;
    assert_eq!(live, persisted);
    assert_eq!(
        live.iter()
            .map(ExplainabilityEnvelope::sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(live[0].record.timestamp, timestamp(13));
    assert_eq!(live[1].record.timestamp, timestamp(9));
    assert_eq!(live[2].record.timestamp, timestamp(11));
    let running = store.get_run(&id).await?.expect("run");
    assert_eq!(running.status, ExplainabilityRunStatus::Running);
    assert_eq!(running.completed_at, None);

    recorder
        .complete_run(RunCompletion::new(
            id.clone(),
            ExplainabilityRunStatus::Completed,
            timestamp(14),
        )?)
        .await?;
    assert!(hub.subscribe(&id).is_none());
    recorder.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_should_not_close_live_channels_from_terminal_events() -> TestResult {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new(),
    ));
    let recorder = recorder_with_hub(store, Arc::clone(&hub));
    let completed_id = run_id("completed-event");
    let failed_id = run_id("failed-event");
    recorder.create_run(query_run(completed_id.clone())).await?;
    recorder.create_run(query_run(failed_id.clone())).await?;
    let mut completed_subscription = hub.subscribe(&completed_id).expect("active");
    let mut failed_subscription = hub.subscribe(&failed_id).expect("active");
    recorder
        .sink()
        .emit(record(
            completed_id.clone(),
            timestamp(10),
            ExplainabilityEvent::RunCompleted(RunCompleted::new(1)),
        ))
        .await?;
    recorder
        .sink()
        .emit(record(
            failed_id.clone(),
            timestamp(10),
            ExplainabilityEvent::RunFailed(RunFailed::new(
                "safe_kind".to_owned(),
                "safe message".to_owned(),
            )),
        ))
        .await?;
    assert_eq!(completed_subscription.recv().await?.sequence(), 1);
    assert_eq!(failed_subscription.recv().await?.sequence(), 1);
    assert!(hub.subscribe(&completed_id).is_some());
    assert!(hub.subscribe(&failed_id).is_some());

    recorder.sink().finish_run(&completed_id).await?;
    recorder.sink().finish_run(&failed_id).await?;
    recorder.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_should_close_active_channels_on_shutdown_without_completing_store_run() -> TestResult
{
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new(),
    ));
    let recorder = recorder_with_hub(Arc::clone(&store), Arc::clone(&hub));
    let id = run_id("shutdown-live");
    recorder.create_run(query_run(id.clone())).await?;
    let mut subscription = hub.subscribe(&id).expect("active");
    recorder
        .sink()
        .emit(started_record(id.clone(), timestamp(10)))
        .await?;
    recorder.shutdown().await?;

    assert_eq!(subscription.recv().await?.sequence(), 1);
    assert_eq!(
        subscription.recv().await,
        Err(ExplainabilityLiveRecvError::Closed)
    );
    assert!(hub.subscribe(&id).is_none());
    let run = store.get_run(&id).await?.expect("run");
    assert_eq!(run.status, ExplainabilityRunStatus::Running);
    assert_eq!(run.completed_at, None);
    assert_eq!(run.event_count, 1);
    Ok(())
}

#[tokio::test]
async fn test_should_close_channels_on_writer_failure_and_preserve_root_error() -> TestResult {
    let store = Arc::new(FailingAppendStore {
        inner: InMemoryExplainabilityStore::new(),
    });
    let hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new(),
    ));
    let recorder = recorder_with_hub(
        Arc::clone(&store) as Arc<dyn ExplainabilityStore>,
        Arc::clone(&hub),
    );
    let id = run_id("writer-failure-live");
    recorder.create_run(query_run(id.clone())).await?;
    let mut subscription = hub.subscribe(&id).expect("active");
    recorder
        .sink()
        .emit(started_record(id.clone(), timestamp(10)))
        .await?;
    assert_eq!(
        recorder.sink().finish_run(&id).await,
        Err(ExplainabilitySinkError::WriterFailed)
    );
    assert_eq!(
        subscription.recv().await,
        Err(ExplainabilityLiveRecvError::Closed)
    );
    assert!(hub.subscribe(&id).is_none());
    assert!(matches!(
        recorder.shutdown().await,
        Err(StoreExplainabilityError::Store {
            source: ExplainabilityStoreError::RunNotFound { .. },
            ..
        })
    ));
    assert_eq!(store.inner.get_run(&id).await?.expect("run").event_count, 0);
    Ok(())
}

#[cfg(feature = "sqlite-store")]
#[tokio::test]
async fn test_should_keep_sqlite_store_and_live_delivery_in_parity_after_reopen() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("live-hub.sqlite");
    let id = run_id("sqlite-live");
    let live = {
        let store: Arc<dyn ExplainabilityStore> =
            Arc::new(SqliteExplainabilityStore::open(&path).await?);
        let hub = Arc::new(ExplainabilityLiveHub::new(
            ExplainabilityLiveHubOptions::new(),
        ));
        let recorder = recorder_with_hub(Arc::clone(&store), Arc::clone(&hub));
        recorder.create_run(query_run(id.clone())).await?;
        let mut subscription = hub.subscribe(&id).expect("active");
        for hour in [13, 9, 11] {
            recorder
                .sink()
                .emit(started_record(id.clone(), timestamp(hour)))
                .await?;
        }
        recorder.sink().finish_run(&id).await?;
        let mut live = Vec::new();
        for _ in 0..3 {
            live.push(subscription.recv().await?.as_ref().clone());
        }
        assert_eq!(
            subscription.recv().await,
            Err(ExplainabilityLiveRecvError::Closed)
        );
        recorder
            .complete_run(RunCompletion::new(
                id.clone(),
                ExplainabilityRunStatus::Completed,
                timestamp(14),
            )?)
            .await?;
        assert_eq!(store.load_events(&id, &EventQuery::new()).await?, live);
        recorder.shutdown().await?;
        live
    };

    let reopened: Arc<dyn ExplainabilityStore> =
        Arc::new(SqliteExplainabilityStore::open(&path).await?);
    let persisted = reopened.load_events(&id, &EventQuery::new()).await?;
    assert_eq!(persisted, live);
    assert_eq!(
        persisted
            .iter()
            .map(ExplainabilityEnvelope::sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    let run = reopened.get_run(&id).await?.expect("run");
    assert_eq!(run.status, ExplainabilityRunStatus::Completed);
    assert_eq!(run.event_count, 3);
    assert_eq!(run.completed_at, Some(timestamp(14)));
    Ok(())
}
