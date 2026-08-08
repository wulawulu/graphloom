//! SQLite explainability store contract and persistence tests.
//!
//! The same `ExplainabilityStore` Version 1 scenario functions used by the
//! in-memory contract binary run here against a real temporary database file.
//! Additional tests cover reopen persistence, schema initialization and
//! rejection, foreign-key cascades, cross-instance concurrency, corruption
//! handling, checked integer conversions, parameterization, and content
//! safety.

#![cfg(feature = "sqlite-store")]

#[path = "support/explainability_store_contract.rs"]
mod contract;

use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::{DateTime, Utc};
use graphloom::explainability::{
    EventQuery, ExplainabilityContentMode, ExplainabilityEnvelope, ExplainabilityEvent,
    ExplainabilityQueryMethod, ExplainabilityRecord, ExplainabilityRun, ExplainabilityRunId,
    ExplainabilityRunKind, ExplainabilityRunStatus, ExplainabilityStore, ExplainabilityStoreError,
    InMemoryExplainabilityStore, QueryStarted, RunCompleted, RunCompletion, RunListCursor,
    RunQuery, RunStarted, SqliteExplainabilityStore,
};
use rusqlite::{Connection, params, types::Value};
use tempfile::TempDir;
use tokio::sync::Barrier;

type TestResult = Result<(), Box<dyn Error>>;

struct StoreFixture {
    store: Arc<dyn ExplainabilityStore>,
    path: PathBuf,
    _tempdir: TempDir,
}

async fn open_fixture() -> Result<StoreFixture, Box<dyn Error>> {
    let tempdir = TempDir::new()?;
    let path = tempdir.path().join("explainability.sqlite");
    let store = SqliteExplainabilityStore::open(&path).await?;
    Ok(StoreFixture {
        store: Arc::new(store),
        path,
        _tempdir: tempdir,
    })
}

fn raw(path: &Path) -> Connection {
    Connection::open(path).expect("raw connection")
}

fn timestamp(day: u32, hour: u32) -> DateTime<Utc> {
    contract::timestamp(day, hour)
}

fn envelope(
    id: ExplainabilityRunId,
    sequence: u64,
    timestamp: DateTime<Utc>,
    event: ExplainabilityEvent,
) -> ExplainabilityEnvelope {
    ExplainabilityEnvelope::new(
        sequence,
        ExplainabilityRecord::new(id, timestamp, contract::span_id("sqlite-span"), None, event),
    )
    .expect("envelope")
}

fn simple_envelope(id: ExplainabilityRunId, sequence: u64) -> ExplainabilityEnvelope {
    envelope(
        id,
        sequence,
        timestamp(1, 10),
        ExplainabilityEvent::RunStarted(RunStarted::new(
            ExplainabilityRunKind::Query,
            ExplainabilityContentMode::Metadata,
        )),
    )
}

macro_rules! sqlite_contract {
    ($test:ident, $scenario:ident) => {
        #[tokio::test]
        async fn $test() -> TestResult {
            let fixture = open_fixture().await?;
            contract::$scenario(fixture.store.as_ref()).await
        }
    };
}

sqlite_contract!(
    test_should_create_run_and_return_owned_copy,
    test_should_create_run_and_return_owned_copy
);
sqlite_contract!(
    test_should_reject_duplicate_run_without_overwriting,
    test_should_reject_duplicate_run_without_overwriting
);
sqlite_contract!(
    test_should_reject_nonzero_initial_event_count,
    test_should_reject_nonzero_initial_event_count
);
sqlite_contract!(
    test_should_reject_terminal_initial_statuses_and_completion_time,
    test_should_reject_terminal_initial_statuses_and_completion_time
);
sqlite_contract!(
    test_should_reject_query_method_on_non_query_run,
    test_should_reject_query_method_on_non_query_run
);
sqlite_contract!(
    test_should_append_contiguous_sequences_and_derive_event_count,
    test_should_append_contiguous_sequences_and_derive_event_count
);
sqlite_contract!(
    test_should_reject_non_contiguous_sequences_without_partial_write,
    test_should_reject_non_contiguous_sequences_without_partial_write
);
sqlite_contract!(
    test_should_reject_mixed_run_batch_without_touching_either_run,
    test_should_reject_mixed_run_batch_without_touching_either_run
);
sqlite_contract!(
    test_should_not_partially_commit_an_invalid_batch,
    test_should_not_partially_commit_an_invalid_batch
);
sqlite_contract!(
    test_should_treat_empty_batch_as_noop,
    test_should_treat_empty_batch_as_noop
);
sqlite_contract!(
    test_should_complete_run_with_terminal_statuses,
    test_should_complete_run_with_terminal_statuses
);
sqlite_contract!(
    test_should_reject_completion_before_run_start,
    test_should_reject_completion_before_run_start
);
sqlite_contract!(
    test_should_allow_exact_completion_retry,
    test_should_allow_exact_completion_retry
);
sqlite_contract!(
    test_should_reject_conflicting_completion,
    test_should_reject_conflicting_completion
);
sqlite_contract!(
    test_should_reject_append_after_terminal_without_changes,
    test_should_reject_append_after_terminal_without_changes
);
sqlite_contract!(
    test_should_page_events_by_sequence,
    test_should_page_events_by_sequence
);
sqlite_contract!(
    test_should_distinguish_missing_run_from_empty_events,
    test_should_distinguish_missing_run_from_empty_events
);
sqlite_contract!(
    test_should_order_run_history_by_start_then_id_descending,
    test_should_order_run_history_by_start_then_id_descending
);
sqlite_contract!(
    test_should_page_run_history_with_cursor_without_duplicates_or_gaps,
    test_should_page_run_history_with_cursor_without_duplicates_or_gaps
);
sqlite_contract!(
    test_should_filter_runs_by_kind_status_and_query_method,
    test_should_filter_runs_by_kind_status_and_query_method
);
sqlite_contract!(
    test_should_delete_run_and_events_idempotently,
    test_should_delete_run_and_events_idempotently
);
sqlite_contract!(
    test_should_isolate_runs_during_lifecycle,
    test_should_isolate_runs_during_lifecycle
);
sqlite_contract!(
    test_should_ignore_event_timestamps_in_replay_order,
    test_should_ignore_event_timestamps_in_replay_order
);
sqlite_contract!(
    test_should_match_jsonl_round_trip_envelopes,
    test_should_match_jsonl_round_trip_envelopes
);
sqlite_contract!(
    test_should_keep_error_messages_free_of_run_content,
    test_should_keep_error_messages_free_of_run_content
);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_should_serialize_concurrent_same_run_appends() -> TestResult {
    let fixture = open_fixture().await?;
    contract::test_should_serialize_concurrent_same_run_appends(fixture.store).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_should_linearize_append_and_complete_race() -> TestResult {
    let fixture = open_fixture().await?;
    contract::test_should_linearize_append_and_complete_race(fixture.store).await
}

#[tokio::test]
async fn test_should_reject_completion_with_non_terminal_status() -> TestResult {
    contract::test_should_reject_completion_with_non_terminal_status().await
}

#[test]
fn test_should_enforce_query_limits() {
    contract::test_should_enforce_query_limits();
}

#[tokio::test]
async fn test_should_support_arc_dyn_store_lifecycle() -> TestResult {
    let fixture = open_fixture().await?;
    contract::test_should_support_arc_dyn_store_lifecycle(fixture.store).await
}

#[tokio::test]
async fn test_should_persist_run_events_and_completion_across_reopen() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("explainability.sqlite");
    let run_id = contract::run_id("persist-run");
    let started_at = timestamp(1, 8);
    let completed_at = timestamp(1, 9);
    let mut run = ExplainabilityRun::new(run_id.clone(), ExplainabilityRunKind::Query, started_at);
    run.status = ExplainabilityRunStatus::Running;
    run.query = Some("reopen query".to_owned());
    run.query_method = Some(ExplainabilityQueryMethod::Local);
    run.compatibility_profile = Some("compat-v1".to_owned());
    let events = vec![
        envelope(
            run_id.clone(),
            1,
            timestamp(1, 10),
            ExplainabilityEvent::RunStarted(RunStarted::new(
                ExplainabilityRunKind::Query,
                ExplainabilityContentMode::Metadata,
            )),
        ),
        {
            let mut query_started = QueryStarted::new(ExplainabilityQueryMethod::Local);
            query_started.query = Some("reopen query".to_owned());
            envelope(
                run_id.clone(),
                2,
                timestamp(1, 11),
                ExplainabilityEvent::QueryStarted(query_started),
            )
        },
        envelope(
            run_id.clone(),
            3,
            timestamp(1, 12),
            ExplainabilityEvent::RunCompleted(RunCompleted::new(12)),
        ),
    ];
    {
        let store = SqliteExplainabilityStore::open(&path).await?;
        store.create_run(run.clone()).await?;
        store.append_events(&events).await?;
        store
            .complete_run(RunCompletion::new(
                run_id.clone(),
                ExplainabilityRunStatus::Completed,
                completed_at,
            )?)
            .await?;
    }

    let store = SqliteExplainabilityStore::open(&path).await?;
    let stored = store.get_run(&run_id).await?.expect("run");
    let mut expected = run;
    expected.status = ExplainabilityRunStatus::Completed;
    expected.completed_at = Some(completed_at);
    expected.event_count = 3;
    assert_eq!(stored, expected);
    assert_eq!(
        store.load_events(&run_id, &EventQuery::new()).await?,
        events
    );
    let listed = store.list_runs(&RunQuery::new()).await?;
    assert_eq!(listed, vec![stored]);
    Ok(())
}

#[tokio::test]
async fn test_should_reopen_same_database_idempotently_without_reset() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("explainability.sqlite");
    let run_id = contract::run_id("idempotent-run");
    let mut run = contract::query_run(
        run_id.clone(),
        timestamp(1, 8),
        ExplainabilityRunStatus::Running,
        None,
    );
    run.query = Some("kept query".to_owned());
    {
        let store = SqliteExplainabilityStore::open(&path).await?;
        store.create_run(run.clone()).await?;
        store
            .append_events(&[simple_envelope(run_id.clone(), 1)])
            .await?;
    }
    for _ in 0..3 {
        let store = SqliteExplainabilityStore::open(&path).await?;
        let stored = store.get_run(&run_id).await?.expect("run");
        assert_eq!(stored.event_count, 1);
        assert_eq!(stored.query.as_deref(), Some("kept query"));
    }
    let connection = raw(&path);
    let version: i64 = connection.query_row(
        "SELECT schema_version FROM explainability_store_meta WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(version, 1);
    Ok(())
}

#[tokio::test]
async fn test_should_reject_future_schema_version_without_touching_database() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("future.sqlite");
    {
        let connection = raw(&path);
        connection.execute_batch(
            "CREATE TABLE explainability_store_meta (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                schema_version INTEGER NOT NULL
            );
            INSERT INTO explainability_store_meta (singleton, schema_version) VALUES (1, 2);",
        )?;
    }

    let error = SqliteExplainabilityStore::open(&path)
        .await
        .expect_err("future schema must be rejected");
    assert!(matches!(
        error,
        ExplainabilityStoreError::Internal {
            operation: "open SQLite explainability store",
            ..
        }
    ));

    let connection = raw(&path);
    let version: i64 = connection.query_row(
        "SELECT schema_version FROM explainability_store_meta",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(version, 2);
    let tables: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(tables, 1);
    Ok(())
}

#[tokio::test]
async fn test_should_reject_partial_schema_without_adopting_tables() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("partial.sqlite");
    {
        let connection = raw(&path);
        connection.execute_batch(
            "CREATE TABLE explainability_runs (
                run_id TEXT PRIMARY KEY,
                kind TEXT NOT NULL
            );",
        )?;
    }

    let error = SqliteExplainabilityStore::open(&path)
        .await
        .expect_err("partial schema must be rejected");
    assert!(matches!(
        error,
        ExplainabilityStoreError::Internal {
            operation: "open SQLite explainability store",
            ..
        }
    ));

    let connection = raw(&path);
    let meta: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = \
         'explainability_store_meta'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(meta, 0);
    let runs: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'explainability_runs'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(runs, 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_should_initialize_schema_atomically_when_opened_concurrently() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("concurrent-open.sqlite");
    let barrier = Arc::new(Barrier::new(3));

    let first_path = path.clone();
    let first_barrier = Arc::clone(&barrier);
    let first = tokio::spawn(async move {
        first_barrier.wait().await;
        SqliteExplainabilityStore::open(first_path).await
    });
    let second_path = path.clone();
    let second_barrier = Arc::clone(&barrier);
    let second = tokio::spawn(async move {
        second_barrier.wait().await;
        SqliteExplainabilityStore::open(second_path).await
    });
    barrier.wait().await;
    first.await??;
    second.await??;

    let store = SqliteExplainabilityStore::open(&path).await?;
    let run_id = contract::run_id("concurrent-open");
    store
        .create_run(contract::query_run(
            run_id,
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    let connection = raw(&path);
    let version: i64 = connection.query_row(
        "SELECT schema_version FROM explainability_store_meta",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(version, 1);
    Ok(())
}

#[tokio::test]
async fn test_should_cascade_delete_events_only_through_foreign_key() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("cascade.sqlite");
    let store = SqliteExplainabilityStore::open(&path).await?;
    let run_id = contract::run_id("cascade-run");
    store
        .create_run(contract::query_run(
            run_id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store
        .append_events(&[
            simple_envelope(run_id.clone(), 1),
            simple_envelope(run_id.clone(), 2),
            simple_envelope(run_id.clone(), 3),
        ])
        .await?;

    store.delete_run(&run_id).await?;

    let connection = raw(&path);
    let events: i64 = connection.query_row(
        "SELECT COUNT(*) FROM explainability_events WHERE run_id = ?1",
        params![run_id.as_str()],
        |row| row.get(0),
    )?;
    assert_eq!(events, 0);
    let runs: i64 =
        connection.query_row("SELECT COUNT(*) FROM explainability_runs", [], |row| {
            row.get(0)
        })?;
    assert_eq!(runs, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_should_serialize_cross_instance_append_race() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("append-race.sqlite");
    let store_a: Arc<dyn ExplainabilityStore> =
        Arc::new(SqliteExplainabilityStore::open(&path).await?);
    let store_b: Arc<dyn ExplainabilityStore> =
        Arc::new(SqliteExplainabilityStore::open(&path).await?);
    let run_id = contract::run_id("race-run");
    store_a
        .create_run(contract::query_run(
            run_id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store_a
        .append_events(&[simple_envelope(run_id.clone(), 1)])
        .await?;

    let barrier = Arc::new(Barrier::new(3));
    let contender = simple_envelope(run_id.clone(), 2);
    let task_a = {
        let store = Arc::clone(&store_a);
        let barrier = Arc::clone(&barrier);
        let contender = contender.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            store.append_events(&[contender]).await
        })
    };
    let task_b = {
        let store = Arc::clone(&store_b);
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            store.append_events(&[contender]).await
        })
    };
    barrier.wait().await;
    let result_a = task_a.await?;
    let result_b = task_b.await?;

    let successes = [&result_a, &result_b]
        .iter()
        .filter(|result| result.is_ok())
        .count();
    let conflicts = [&result_a, &result_b]
        .iter()
        .filter(|result| {
            matches!(
                result,
                Err(ExplainabilityStoreError::SequenceConflict {
                    expected: 3,
                    actual: 2,
                    ..
                })
            )
        })
        .count();
    assert_eq!(successes, 1);
    assert_eq!(conflicts, 1);
    assert_eq!(store_a.get_run(&run_id).await?.expect("run").event_count, 2);
    let events = store_a.load_events(&run_id, &EventQuery::new()).await?;
    assert_eq!(
        events
            .iter()
            .map(ExplainabilityEnvelope::sequence)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_should_linearize_cross_instance_append_and_complete() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("append-complete.sqlite");
    let store_a: Arc<dyn ExplainabilityStore> =
        Arc::new(SqliteExplainabilityStore::open(&path).await?);
    let store_b: Arc<dyn ExplainabilityStore> =
        Arc::new(SqliteExplainabilityStore::open(&path).await?);
    let run_id = contract::run_id("race-complete");
    store_a
        .create_run(contract::query_run(
            run_id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store_a
        .append_events(&[simple_envelope(run_id.clone(), 1)])
        .await?;
    let completed_at = timestamp(1, 9);
    let completion = RunCompletion::new(
        run_id.clone(),
        ExplainabilityRunStatus::Completed,
        completed_at,
    )?;
    let barrier = Arc::new(Barrier::new(3));
    let contender = simple_envelope(run_id.clone(), 2);

    let append_task = {
        let store = Arc::clone(&store_a);
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            store.append_events(&[contender]).await
        })
    };
    let complete_task = {
        let store = Arc::clone(&store_b);
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            store.complete_run(completion).await
        })
    };
    barrier.wait().await;
    let append_result = append_task.await?;
    let complete_result = complete_task.await?;

    let run = store_a.get_run(&run_id).await?.expect("run");
    let events = store_a.load_events(&run_id, &EventQuery::new()).await?;
    match (&append_result, &complete_result) {
        (Ok(()), Ok(())) => {
            assert_eq!(run.status, ExplainabilityRunStatus::Completed);
            assert_eq!(run.completed_at, Some(completed_at));
            assert_eq!(events.len(), 2);
        }
        (Err(ExplainabilityStoreError::RunAlreadyTerminal { .. }), Ok(())) => {
            assert_eq!(run.status, ExplainabilityRunStatus::Completed);
            assert_eq!(run.completed_at, Some(completed_at));
            assert_eq!(events.len(), 1);
        }
        (append, complete) => {
            panic!(
                "unexpected cross-instance append/complete linearization: append={append:?} \
                 complete={complete:?}"
            );
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_should_linearize_cross_instance_duplicate_create() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("create-race.sqlite");
    let store_a: Arc<dyn ExplainabilityStore> =
        Arc::new(SqliteExplainabilityStore::open(&path).await?);
    let store_b: Arc<dyn ExplainabilityStore> =
        Arc::new(SqliteExplainabilityStore::open(&path).await?);
    let run = contract::query_run(
        contract::run_id("duplicate-race"),
        timestamp(1, 8),
        ExplainabilityRunStatus::Running,
        None,
    );
    let barrier = Arc::new(Barrier::new(3));
    let task_a = {
        let store = Arc::clone(&store_a);
        let barrier = Arc::clone(&barrier);
        let run = run.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            store.create_run(run).await
        })
    };
    let task_b = {
        let store = Arc::clone(&store_b);
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            store.create_run(run).await
        })
    };
    barrier.wait().await;
    let result_a = task_a.await?;
    let result_b = task_b.await?;

    let successes = [&result_a, &result_b]
        .iter()
        .filter(|result| result.is_ok())
        .count();
    let already_exists = [&result_a, &result_b]
        .iter()
        .filter(|result| {
            matches!(
                result,
                Err(ExplainabilityStoreError::RunAlreadyExists { .. })
            )
        })
        .count();
    assert_eq!(successes, 1);
    assert_eq!(already_exists, 1);

    let connection = raw(&path);
    let runs: i64 =
        connection.query_row("SELECT COUNT(*) FROM explainability_runs", [], |row| {
            row.get(0)
        })?;
    assert_eq!(runs, 1);
    Ok(())
}

#[tokio::test]
async fn test_should_match_inmemory_run_pagination_with_tie_break() -> TestResult {
    let fixture = open_fixture().await?;
    let memory = Arc::new(InMemoryExplainabilityStore::new());
    for id in ["run-c", "run-b", "run-a"] {
        let run = contract::query_run(
            contract::run_id(id),
            timestamp(1, 10),
            ExplainabilityRunStatus::Running,
            None,
        );
        fixture.store.create_run(run.clone()).await?;
        memory.create_run(run).await?;
    }

    let sqlite_runs = fixture.store.list_runs(&RunQuery::new()).await?;
    let memory_runs = memory.list_runs(&RunQuery::new()).await?;
    assert_eq!(sqlite_runs, memory_runs);
    let ids = sqlite_runs
        .iter()
        .map(|run| run.run_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["run-c", "run-b", "run-a"]);
    Ok(())
}

#[tokio::test]
async fn test_should_match_inmemory_ordering_with_canonical_timestamps() -> TestResult {
    let fixture = open_fixture().await?;
    let memory = Arc::new(InMemoryExplainabilityStore::new());
    for (day, hour, id) in [
        (1, 8, "run-1"),
        (1, 9, "run-2"),
        (1, 9, "run-3"),
        (2, 1, "run-4"),
        (2, 1, "run-5"),
    ] {
        let run = contract::query_run(
            contract::run_id(id),
            timestamp(day, hour),
            ExplainabilityRunStatus::Running,
            None,
        );
        fixture.store.create_run(run.clone()).await?;
        memory.create_run(run).await?;
    }

    let query = RunQuery::new().with_limit(3)?;
    let sqlite_page = fixture.store.list_runs(&query).await?;
    let memory_page = memory.list_runs(&query).await?;
    assert_eq!(sqlite_page, memory_page);

    let cursor = RunListCursor::new(sqlite_page[2].started_at, sqlite_page[2].run_id.clone());
    let sqlite_next = fixture
        .store
        .list_runs(&RunQuery::new().with_limit(3)?.before(cursor.clone()))
        .await?;
    let memory_next = memory
        .list_runs(&RunQuery::new().with_limit(3)?.before(cursor))
        .await?;
    assert_eq!(sqlite_next, memory_next);
    Ok(())
}

#[tokio::test]
async fn test_should_reject_noncanonical_sqlite_timestamps() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("noncanonical.sqlite");
    let store = SqliteExplainabilityStore::open(&path).await?;
    let fixtures = [
        "2026-01-01T16:00:00.000000000+08:00",
        "2026-01-01T08:00:00Z",
        "2026-01-01T08:00:00.1Z",
        "2026-01-01T08:00:00.123456Z",
        "2026-01-01T08:00:00.000000000+00:00",
    ];
    for (index, started_at) in fixtures.into_iter().enumerate() {
        let run_id = contract::run_id(&format!("noncanonical-{index}"));
        store
            .create_run(contract::query_run(
                run_id.clone(),
                timestamp(1, 8),
                ExplainabilityRunStatus::Running,
                None,
            ))
            .await?;
        let connection = raw(&path);
        connection.execute(
            "UPDATE explainability_runs SET started_at = ?1 WHERE run_id = ?2",
            params![started_at, run_id.as_str()],
        )?;

        let read = store
            .get_run(&run_id)
            .await
            .expect_err("non-canonical timestamp must be rejected on read");
        assert!(
            matches!(
                read,
                ExplainabilityStoreError::Internal {
                    operation: "read explainability run",
                    ..
                }
            ),
            "case {index}: {read}"
        );
        let listed = store
            .list_runs(&RunQuery::new())
            .await
            .expect_err("non-canonical timestamp must be rejected on list");
        assert!(
            matches!(
                listed,
                ExplainabilityStoreError::Internal {
                    operation: "list explainability runs",
                    ..
                }
            ),
            "case {index}: {listed}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn test_should_fail_safely_when_appending_beyond_i64_max() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("overflow.sqlite");
    let store = SqliteExplainabilityStore::open(&path).await?;
    let run_id = contract::run_id("overflow-run");
    store
        .create_run(contract::query_run(
            run_id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store
        .append_events(&[simple_envelope(run_id.clone(), 1)])
        .await?;
    {
        let connection = raw(&path);
        connection.execute(
            "UPDATE explainability_runs SET event_count = ?1 WHERE run_id = ?2",
            params![i64::MAX, run_id.as_str()],
        )?;
    }

    let oversized = envelope(
        run_id.clone(),
        u64::try_from(i64::MAX).expect("fixture") + 1,
        timestamp(1, 10),
        ExplainabilityEvent::RunStarted(RunStarted::new(
            ExplainabilityRunKind::Query,
            ExplainabilityContentMode::Metadata,
        )),
    );
    let error = store
        .append_events(&[oversized])
        .await
        .expect_err("out-of-range sequence must fail safely");
    assert!(matches!(
        error,
        ExplainabilityStoreError::Internal {
            operation: "append explainability events",
            ..
        }
    ));

    let connection = raw(&path);
    let event_count: i64 = connection.query_row(
        "SELECT event_count FROM explainability_runs WHERE run_id = ?1",
        params![run_id.as_str()],
        |row| row.get(0),
    )?;
    assert_eq!(event_count, i64::MAX);
    let events: i64 = connection.query_row(
        "SELECT COUNT(*) FROM explainability_events WHERE run_id = ?1",
        params![run_id.as_str()],
        |row| row.get(0),
    )?;
    assert_eq!(events, 1);
    Ok(())
}

#[tokio::test]
async fn test_should_return_internal_for_corrupted_run_rows() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("corrupt-runs.sqlite");
    let store = SqliteExplainabilityStore::open(&path).await?;
    let cases: Vec<(&str, &str, Vec<Value>)> = vec![
        (
            "corrupt-1",
            "UPDATE explainability_runs SET event_count = ?1 WHERE run_id = ?2",
            vec![Value::Integer(-1), Value::Text("corrupt-1".to_owned())],
        ),
        (
            "corrupt-2",
            "UPDATE explainability_runs SET status = ?1 WHERE run_id = ?2",
            vec![
                Value::Text("future".to_owned()),
                Value::Text("corrupt-2".to_owned()),
            ],
        ),
        (
            "corrupt-3",
            "UPDATE explainability_runs SET kind = ?1 WHERE run_id = ?2",
            vec![
                Value::Text("future".to_owned()),
                Value::Text("corrupt-3".to_owned()),
            ],
        ),
        (
            "corrupt-4",
            "UPDATE explainability_runs SET query_method = ?1 WHERE run_id = ?2",
            vec![
                Value::Text("future".to_owned()),
                Value::Text("corrupt-4".to_owned()),
            ],
        ),
        (
            "corrupt-5",
            "UPDATE explainability_runs SET started_at = ?1 WHERE run_id = ?2",
            vec![
                Value::Text("not-a-time".to_owned()),
                Value::Text("corrupt-5".to_owned()),
            ],
        ),
        (
            "corrupt-6",
            "UPDATE explainability_runs SET status = ?1 WHERE run_id = ?2",
            vec![
                Value::Text("completed".to_owned()),
                Value::Text("corrupt-6".to_owned()),
            ],
        ),
        (
            "corrupt-7",
            "UPDATE explainability_runs SET status = ?1, completed_at = ?2 WHERE run_id = ?3",
            vec![
                Value::Text("running".to_owned()),
                Value::Text("2026-01-01T09:00:00.000000000Z".to_owned()),
                Value::Text("corrupt-7".to_owned()),
            ],
        ),
        (
            "corrupt-8",
            "UPDATE explainability_runs SET status = ?1, completed_at = ?2 WHERE run_id = ?3",
            vec![
                Value::Text("completed".to_owned()),
                Value::Text("2026-01-01T07:00:00.000000000Z".to_owned()),
                Value::Text("corrupt-8".to_owned()),
            ],
        ),
        (
            "corrupt-9",
            "UPDATE explainability_runs SET started_at = ?1 WHERE run_id = ?2",
            vec![
                Value::Text("2026-01-01T16:00:00.000000000+08:00".to_owned()),
                Value::Text("corrupt-9".to_owned()),
            ],
        ),
        (
            "corrupt-10",
            "UPDATE explainability_runs SET status = ?1, completed_at = ?2 WHERE run_id = ?3",
            vec![
                Value::Text("completed".to_owned()),
                Value::Text("2026-01-01T09:00:00.000000000+00:00".to_owned()),
                Value::Text("corrupt-10".to_owned()),
            ],
        ),
    ];

    for (index, (id, sql, values)) in cases.into_iter().enumerate() {
        let run_id = contract::run_id(id);
        store
            .create_run(contract::query_run(
                run_id.clone(),
                timestamp(1, 8),
                ExplainabilityRunStatus::Running,
                None,
            ))
            .await?;
        let connection = raw(&path);
        connection.execute_batch("PRAGMA ignore_check_constraints = ON;")?;
        connection.execute(sql, rusqlite::params_from_iter(values))?;
        let error = store
            .get_run(&run_id)
            .await
            .expect_err("corrupted row must be rejected");
        assert!(
            matches!(
                error,
                ExplainabilityStoreError::Internal {
                    operation: "read explainability run",
                    ..
                }
            ),
            "case {index} produced {error}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn test_should_reject_append_when_persisted_run_lifecycle_is_corrupted() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("corrupt-append.sqlite");
    let store = SqliteExplainabilityStore::open(&path).await?;
    let cases: Vec<(&str, &str, Vec<Value>)> = vec![
        (
            "write-corrupt-a",
            "UPDATE explainability_runs SET completed_at = ?1 WHERE run_id = ?2",
            vec![
                Value::Text("2026-01-01T09:00:00.000000000Z".to_owned()),
                Value::Text("write-corrupt-a".to_owned()),
            ],
        ),
        (
            "write-corrupt-b",
            "UPDATE explainability_runs SET status = ?1, completed_at = ?2 WHERE run_id = ?3",
            vec![
                Value::Text("completed".to_owned()),
                Value::Null,
                Value::Text("write-corrupt-b".to_owned()),
            ],
        ),
        (
            "write-corrupt-d",
            "UPDATE explainability_runs SET status = ?1, completed_at = ?2 WHERE run_id = ?3",
            vec![
                Value::Text("completed".to_owned()),
                Value::Text("2026-01-01T07:00:00.000000000Z".to_owned()),
                Value::Text("write-corrupt-d".to_owned()),
            ],
        ),
        (
            "write-corrupt-e",
            "UPDATE explainability_runs SET started_at = ?1 WHERE run_id = ?2",
            vec![
                Value::Text("2026-01-01T16:00:00.000000000+08:00".to_owned()),
                Value::Text("write-corrupt-e".to_owned()),
            ],
        ),
    ];

    for (index, (id, sql, values)) in cases.into_iter().enumerate() {
        let run_id = contract::run_id(id);
        store
            .create_run(contract::query_run(
                run_id.clone(),
                timestamp(1, 8),
                ExplainabilityRunStatus::Running,
                None,
            ))
            .await?;
        store
            .append_events(&[simple_envelope(run_id.clone(), 1)])
            .await?;
        let connection = raw(&path);
        connection.execute(sql, rusqlite::params_from_iter(values))?;

        let error = store
            .append_events(&[simple_envelope(run_id.clone(), 2)])
            .await
            .expect_err("corrupted run lifecycle must reject appends");
        assert!(
            matches!(
                error,
                ExplainabilityStoreError::Internal {
                    operation: "append explainability events",
                    ..
                }
            ),
            "case {index}: {error}"
        );

        let connection = raw(&path);
        let event_count: i64 = connection.query_row(
            "SELECT event_count FROM explainability_runs WHERE run_id = ?1",
            params![run_id.as_str()],
            |row| row.get(0),
        )?;
        assert_eq!(
            event_count, 1,
            "case {index}: event_count must stay unchanged"
        );
        let events: i64 = connection.query_row(
            "SELECT COUNT(*) FROM explainability_events WHERE run_id = ?1",
            params![run_id.as_str()],
            |row| row.get(0),
        )?;
        assert_eq!(events, 1, "case {index}: events must stay unchanged");
    }
    Ok(())
}

#[tokio::test]
async fn test_should_reject_completion_when_persisted_run_lifecycle_is_corrupted() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("corrupt-complete.sqlite");
    let store = SqliteExplainabilityStore::open(&path).await?;
    let cases: Vec<(&str, &str, Vec<Value>)> = vec![
        (
            "complete-corrupt-a",
            "UPDATE explainability_runs SET completed_at = ?1 WHERE run_id = ?2",
            vec![
                Value::Text("2026-01-01T09:00:00.000000000Z".to_owned()),
                Value::Text("complete-corrupt-a".to_owned()),
            ],
        ),
        (
            "complete-corrupt-b",
            "UPDATE explainability_runs SET status = ?1, completed_at = ?2 WHERE run_id = ?3",
            vec![
                Value::Text("completed".to_owned()),
                Value::Null,
                Value::Text("complete-corrupt-b".to_owned()),
            ],
        ),
        (
            "complete-corrupt-d",
            "UPDATE explainability_runs SET status = ?1, completed_at = ?2 WHERE run_id = ?3",
            vec![
                Value::Text("completed".to_owned()),
                Value::Text("2026-01-01T07:00:00.000000000Z".to_owned()),
                Value::Text("complete-corrupt-d".to_owned()),
            ],
        ),
        (
            "complete-corrupt-e",
            "UPDATE explainability_runs SET started_at = ?1 WHERE run_id = ?2",
            vec![
                Value::Text("2026-01-01T16:00:00.000000000+08:00".to_owned()),
                Value::Text("complete-corrupt-e".to_owned()),
            ],
        ),
    ];

    for (index, (id, sql, values)) in cases.into_iter().enumerate() {
        let run_id = contract::run_id(id);
        store
            .create_run(contract::query_run(
                run_id.clone(),
                timestamp(1, 8),
                ExplainabilityRunStatus::Running,
                None,
            ))
            .await?;
        let connection = raw(&path);
        connection.execute(sql, rusqlite::params_from_iter(values))?;

        let error = store
            .complete_run(RunCompletion::new(
                run_id.clone(),
                ExplainabilityRunStatus::Completed,
                timestamp(1, 9),
            )?)
            .await
            .expect_err("corrupted run lifecycle must reject completion");
        assert!(
            matches!(
                error,
                ExplainabilityStoreError::Internal {
                    operation: "complete explainability run",
                    ..
                }
            ),
            "case {index}: {error}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn test_should_return_internal_for_corrupted_event_rows() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("corrupt-events.sqlite");
    let store = SqliteExplainabilityStore::open(&path).await?;
    let run_id = contract::run_id("event-corrupt");
    store
        .create_run(contract::query_run(
            run_id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store
        .append_events(&[simple_envelope(run_id.clone(), 1)])
        .await?;

    let cases: Vec<(&str, Vec<Value>)> = vec![
        (
            "UPDATE explainability_events SET schema_version = ?1 WHERE run_id = ?2",
            vec![Value::Integer(2), Value::Text("event-corrupt".to_owned())],
        ),
        (
            "UPDATE explainability_events SET span_id = ?1 WHERE run_id = ?2",
            vec![
                Value::Text("bad/span".to_owned()),
                Value::Text("event-corrupt".to_owned()),
            ],
        ),
        (
            "UPDATE explainability_events SET timestamp = ?1 WHERE run_id = ?2",
            vec![
                Value::Text("not-a-time".to_owned()),
                Value::Text("event-corrupt".to_owned()),
            ],
        ),
        (
            "UPDATE explainability_events SET payload_json = ?1 WHERE run_id = ?2",
            vec![
                Value::Text("{invalid".to_owned()),
                Value::Text("event-corrupt".to_owned()),
            ],
        ),
        (
            "UPDATE explainability_events SET event_type = ?1 WHERE run_id = ?2",
            vec![
                Value::Text("other".to_owned()),
                Value::Text("event-corrupt".to_owned()),
            ],
        ),
    ];

    for (index, (sql, values)) in cases.into_iter().enumerate() {
        let connection = raw(&path);
        connection.execute_batch("PRAGMA ignore_check_constraints = ON;")?;
        connection.execute(sql, rusqlite::params_from_iter(values))?;
        let error = store
            .load_events(&run_id, &EventQuery::new())
            .await
            .expect_err("corrupted event row must be rejected");
        assert!(
            matches!(
                error,
                ExplainabilityStoreError::Internal {
                    operation: "load explainability events",
                    ..
                }
            ),
            "case {index} produced {error}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn test_should_parameterize_all_business_values() -> TestResult {
    let fixture = open_fixture().await?;
    let run_id = contract::run_id("inject-run");
    let mut run = contract::query_run(
        run_id.clone(),
        timestamp(1, 8),
        ExplainabilityRunStatus::Running,
        Some(ExplainabilityQueryMethod::Local),
    );
    run.query = Some("'; DROP TABLE explainability_runs; --".to_owned());
    run.compatibility_profile = Some("profile'; DROP TABLE explainability_events; --".to_owned());
    fixture.store.create_run(run.clone()).await?;

    let stored = fixture.store.get_run(&run_id).await?.expect("run");
    assert_eq!(
        stored.query.as_deref(),
        Some("'; DROP TABLE explainability_runs; --")
    );
    assert_eq!(
        stored.compatibility_profile.as_deref(),
        Some("profile'; DROP TABLE explainability_events; --")
    );

    let connection = raw(fixture.path());
    let runs: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'explainability_runs'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(runs, 1);
    fixture
        .store
        .append_events(&[simple_envelope(run_id.clone(), 1)])
        .await?;
    assert_eq!(
        fixture
            .store
            .load_events(&run_id, &EventQuery::new())
            .await?
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn test_should_keep_db_path_and_payloads_out_of_errors_and_debug() -> TestResult {
    const PATH_SENTINEL: &str = "SQLITE_DB_PATH_SECRET_SENTINEL";
    const QUERY_SENTINEL: &str = "SQLITE_QUERY_SECRET_SENTINEL";
    const EVENT_SENTINEL: &str = "SQLITE_EVENT_SECRET_SENTINEL";

    let open_error =
        SqliteExplainabilityStore::open(PathBuf::from("/nonexistent-parent").join(PATH_SENTINEL))
            .await
            .expect_err("open failure");
    assert!(!open_error.to_string().contains(PATH_SENTINEL));

    let directory = TempDir::new()?;
    let path = directory.path().join(PATH_SENTINEL).join("db.sqlite");
    tokio::fs::create_dir_all(path.parent().expect("parent")).await?;
    let store = SqliteExplainabilityStore::open(&path).await?;
    let debug = format!("{store:?}");
    assert!(!debug.contains(PATH_SENTINEL));

    let run_id = contract::run_id("content-safe");
    let mut run = contract::query_run(
        run_id.clone(),
        timestamp(1, 8),
        ExplainabilityRunStatus::Running,
        Some(ExplainabilityQueryMethod::Local),
    );
    run.query = Some(QUERY_SENTINEL.to_owned());
    store.create_run(run).await?;

    let mut query_started = QueryStarted::new(ExplainabilityQueryMethod::Local);
    query_started.query = Some(EVENT_SENTINEL.repeat(1_048_576 / EVENT_SENTINEL.len() + 1));
    let oversized = envelope(
        run_id.clone(),
        1,
        timestamp(1, 10),
        ExplainabilityEvent::QueryStarted(query_started),
    );
    let serialization_error = store
        .append_events(&[oversized])
        .await
        .expect_err("oversized event must fail");
    assert!(matches!(
        serialization_error,
        ExplainabilityStoreError::Internal {
            operation: "append explainability events",
            ..
        }
    ));
    let message = serialization_error.to_string();
    assert!(!message.contains(PATH_SENTINEL));
    assert!(!message.contains(QUERY_SENTINEL));
    assert!(!message.contains(EVENT_SENTINEL));
    Ok(())
}

#[tokio::test]
async fn test_should_match_jsonl_and_inmemory_envelope_round_trips() -> TestResult {
    let fixture = open_fixture().await?;
    let memory = Arc::new(InMemoryExplainabilityStore::new());
    let run_id = contract::run_id("three-way");
    let run = contract::query_run(
        run_id.clone(),
        timestamp(1, 8),
        ExplainabilityRunStatus::Running,
        None,
    );
    fixture.store.create_run(run.clone()).await?;
    memory.create_run(run).await?;
    let events = vec![
        envelope(
            run_id.clone(),
            1,
            timestamp(1, 13),
            ExplainabilityEvent::RunStarted(RunStarted::new(
                ExplainabilityRunKind::Query,
                ExplainabilityContentMode::Metadata,
            )),
        ),
        envelope(
            run_id.clone(),
            2,
            timestamp(1, 11),
            ExplainabilityEvent::QueryStarted(QueryStarted::new(ExplainabilityQueryMethod::Local)),
        ),
        envelope(
            run_id.clone(),
            3,
            timestamp(1, 12),
            ExplainabilityEvent::RunCompleted(RunCompleted::new(12)),
        ),
    ];
    fixture.store.append_events(&events).await?;
    memory.append_events(&events).await?;

    let mut jsonl = Vec::new();
    for event in &events {
        let mut line = serde_json::to_vec(event)?;
        line.push(b'\n');
        jsonl.push(serde_json::from_slice::<ExplainabilityEnvelope>(&line)?);
    }
    let sqlite_loaded = fixture
        .store
        .load_events(&run_id, &EventQuery::new())
        .await?;
    let memory_loaded = memory.load_events(&run_id, &EventQuery::new()).await?;
    assert_eq!(sqlite_loaded, memory_loaded);
    assert_eq!(sqlite_loaded, jsonl);
    Ok(())
}

#[tokio::test]
async fn test_should_create_real_persistent_database_file() -> TestResult {
    let directory = TempDir::new()?;
    let path = directory.path().join("persistent.sqlite");
    let store = SqliteExplainabilityStore::open(&path).await?;
    let run_id = contract::run_id("file-run");
    store
        .create_run(contract::query_run(
            run_id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store.append_events(&[simple_envelope(run_id, 1)]).await?;
    assert!(path.exists());
    let size = tokio::fs::metadata(&path).await?.len();
    assert!(size > 0);
    Ok(())
}

#[tokio::test]
async fn test_should_handle_after_sequence_beyond_sqlite_signed_range() -> TestResult {
    let fixture = open_fixture().await?;
    let run_id = contract::run_id("after-range");
    fixture
        .store
        .create_run(contract::query_run(
            run_id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    fixture
        .store
        .append_events(&[simple_envelope(run_id.clone(), 1)])
        .await?;
    let page = fixture
        .store
        .load_events(
            &run_id,
            &EventQuery::new().after_sequence(u64::try_from(i64::MAX).expect("fixture")),
        )
        .await?;
    assert!(page.is_empty());
    Ok(())
}

impl StoreFixture {
    fn path(&self) -> &Path {
        &self.path
    }
}
