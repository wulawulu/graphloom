//! Shared `ExplainabilityStore` Version 1 contract scenarios.
//!
//! Every scenario is executed against `InMemoryExplainabilityStore` and
//! `SqliteExplainabilityStore` so the persistent backend cannot drift from
//! the frozen reference semantics. The test binaries own the fixture
//! lifecycle; this module only asserts business behavior through the public
//! trait.

use std::{error::Error, sync::Arc};

use chrono::{DateTime, TimeZone, Utc};
use graphloom::explainability::{
    DEFAULT_EVENT_QUERY_LIMIT, DEFAULT_RUN_QUERY_LIMIT, EventQuery, ExplainabilityContentMode,
    ExplainabilityEnvelope, ExplainabilityEvent, ExplainabilityQueryMethod, ExplainabilityRecord,
    ExplainabilityRun, ExplainabilityRunId, ExplainabilityRunKind, ExplainabilityRunStatus,
    ExplainabilitySpanId, ExplainabilityStore, ExplainabilityStoreError, MAX_EVENT_QUERY_LIMIT,
    MAX_RUN_QUERY_LIMIT, RunCompletion, RunListCursor, RunQuery, RunStarted,
};
use tokio::sync::Barrier;

pub(crate) type TestResult = Result<(), Box<dyn Error>>;

const QUERY_SECRET_SENTINEL: &str = "STORE_QUERY_SECRET_SENTINEL";

pub(crate) fn run_id(value: &str) -> ExplainabilityRunId {
    value.parse().expect("run id")
}

pub(crate) fn span_id(value: &str) -> ExplainabilitySpanId {
    value.parse().expect("span id")
}

pub(crate) fn timestamp(day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, day, hour, 0, 0)
        .single()
        .expect("timestamp")
}

pub(crate) fn query_run(
    id: ExplainabilityRunId,
    started_at: DateTime<Utc>,
    status: ExplainabilityRunStatus,
    query_method: Option<ExplainabilityQueryMethod>,
) -> ExplainabilityRun {
    let mut run = ExplainabilityRun::new(id, ExplainabilityRunKind::Query, started_at);
    run.status = status;
    run.query_method = query_method;
    run
}

pub(crate) fn envelope(
    id: ExplainabilityRunId,
    sequence: u64,
    timestamp: DateTime<Utc>,
) -> ExplainabilityEnvelope {
    ExplainabilityEnvelope::new(
        sequence,
        ExplainabilityRecord::new(
            id,
            timestamp,
            span_id("store-span"),
            None,
            ExplainabilityEvent::RunStarted(RunStarted::new(
                ExplainabilityRunKind::Query,
                ExplainabilityContentMode::Metadata,
            )),
        ),
    )
    .expect("envelope")
}

pub(crate) fn envelope_with_sequence(
    id: ExplainabilityRunId,
    sequence: u64,
) -> ExplainabilityEnvelope {
    envelope(id, sequence, timestamp(1, 10))
}

pub(crate) async fn stored_events(
    store: &dyn ExplainabilityStore,
    id: &ExplainabilityRunId,
) -> Vec<ExplainabilityEnvelope> {
    store
        .load_events(id, &EventQuery::new())
        .await
        .expect("load events")
}

fn assert_error_display_has_no_run_content(error: &ExplainabilityStoreError) {
    let message = error.to_string();
    assert!(!message.contains(QUERY_SECRET_SENTINEL));
    assert!(!message.contains("prompt"));
    assert!(!message.contains("context"));
    assert!(!message.contains("response"));
}

pub(crate) async fn test_should_create_run_and_return_owned_copy(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let started_at = timestamp(1, 8);
    let run = query_run(
        run_id("run-1"),
        started_at,
        ExplainabilityRunStatus::Running,
        Some(ExplainabilityQueryMethod::Local),
    );

    store.create_run(run.clone()).await?;

    let stored = store.get_run(&run.run_id).await?.expect("run");
    assert_eq!(stored, run);
    assert_eq!(stored.event_count, 0);
    assert_eq!(stored.completed_at, None);
    Ok(())
}

pub(crate) async fn test_should_reject_duplicate_run_without_overwriting(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let id = run_id("duplicate");
    let original = query_run(
        id.clone(),
        timestamp(1, 8),
        ExplainabilityRunStatus::Running,
        Some(ExplainabilityQueryMethod::Local),
    );
    store.create_run(original.clone()).await?;

    let duplicate = query_run(
        id.clone(),
        timestamp(1, 9),
        ExplainabilityRunStatus::Pending,
        Some(ExplainabilityQueryMethod::Global),
    );
    let error = store
        .create_run(duplicate)
        .await
        .expect_err("duplicate run must be rejected");
    assert!(matches!(
        error,
        ExplainabilityStoreError::RunAlreadyExists { .. }
    ));
    assert_error_display_has_no_run_content(&error);
    assert_eq!(store.get_run(&id).await?, Some(original));
    Ok(())
}

pub(crate) async fn test_should_reject_nonzero_initial_event_count(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let id = run_id("precounted");
    let mut run = ExplainabilityRun::new(id.clone(), ExplainabilityRunKind::Query, timestamp(1, 8));
    run.event_count = 1;

    let error = store
        .create_run(run)
        .await
        .expect_err("pre-counted run must be rejected");
    assert!(matches!(
        error,
        ExplainabilityStoreError::InvalidRunMetadata { reason, .. }
            if reason == "initial event_count must be 0"
    ));
    assert_eq!(store.get_run(&id).await?, None);
    Ok(())
}

pub(crate) async fn test_should_reject_terminal_initial_statuses_and_completion_time(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    for status in [
        ExplainabilityRunStatus::Completed,
        ExplainabilityRunStatus::Failed,
        ExplainabilityRunStatus::Cancelled,
    ] {
        let id = run_id(&format!("terminal-{status:?}"));
        let run = query_run(id.clone(), timestamp(1, 8), status, None);
        let error = store
            .create_run(run)
            .await
            .expect_err("terminal initial status must be rejected");
        assert!(matches!(
            error,
            ExplainabilityStoreError::InvalidRunState { .. }
        ));
        assert_eq!(store.get_run(&id).await?, None);
    }

    let id = run_id("precompleted");
    let mut run = ExplainabilityRun::new(id.clone(), ExplainabilityRunKind::Query, timestamp(1, 8));
    run.completed_at = Some(timestamp(1, 9));
    let error = store
        .create_run(run)
        .await
        .expect_err("pre-completed run must be rejected");
    assert!(matches!(
        error,
        ExplainabilityStoreError::InvalidRunMetadata { reason, .. }
            if reason == "a new run cannot already be completed"
    ));
    Ok(())
}

pub(crate) async fn test_should_reject_query_method_on_non_query_run(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let id = run_id("index-with-method");
    let mut run = ExplainabilityRun::new(id.clone(), ExplainabilityRunKind::Index, timestamp(1, 8));
    run.query_method = Some(ExplainabilityQueryMethod::Local);

    let error = store
        .create_run(run)
        .await
        .expect_err("query method on non-query run must be rejected");
    assert!(matches!(
        error,
        ExplainabilityStoreError::InvalidRunMetadata { reason, .. }
            if reason == "query_method is only valid for query runs"
    ));
    assert_eq!(store.get_run(&id).await?, None);
    Ok(())
}

pub(crate) async fn test_should_append_contiguous_sequences_and_derive_event_count(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let id = run_id("contiguous");
    store
        .create_run(query_run(
            id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            Some(ExplainabilityQueryMethod::Local),
        ))
        .await?;
    let batch = vec![
        envelope_with_sequence(id.clone(), 1),
        envelope_with_sequence(id.clone(), 2),
        envelope_with_sequence(id.clone(), 3),
    ];

    store.append_events(&batch).await?;

    assert_eq!(store.get_run(&id).await?.expect("run").event_count, 3);
    let events = stored_events(store, &id).await;
    assert_eq!(
        events
            .iter()
            .map(ExplainabilityEnvelope::sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    Ok(())
}

pub(crate) async fn test_should_reject_non_contiguous_sequences_without_partial_write(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let id = run_id("sequences");
    store
        .create_run(query_run(
            id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;

    let cases: Vec<Vec<u64>> = vec![vec![2], vec![1, 3], vec![1, 1], vec![4, 5], vec![2, 4]];
    for sequences in cases {
        let batch = sequences
            .iter()
            .map(|sequence| envelope_with_sequence(id.clone(), *sequence))
            .collect::<Vec<_>>();
        let error = store
            .append_events(&batch)
            .await
            .expect_err("non-contiguous batch must be rejected");
        assert!(
            matches!(error, ExplainabilityStoreError::SequenceConflict { .. }),
            "unexpected error: {error}"
        );
        assert_error_display_has_no_run_content(&error);
        assert_eq!(
            store.get_run(&id).await?.expect("run").event_count,
            0,
            "failed batch must not change event_count for {sequences:?}"
        );
        assert!(
            stored_events(store, &id).await.is_empty(),
            "failed batch must not persist events for {sequences:?}"
        );
    }

    store
        .append_events(&[envelope_with_sequence(id.clone(), 1)])
        .await?;
    let error = store
        .append_events(&[envelope_with_sequence(id.clone(), 1)])
        .await
        .expect_err("duplicate old sequence must be rejected");
    assert!(matches!(
        error,
        ExplainabilityStoreError::SequenceConflict {
            expected: 2,
            actual: 1,
            ..
        }
    ));
    assert_eq!(store.get_run(&id).await?.expect("run").event_count, 1);
    assert_eq!(
        stored_events(store, &id)
            .await
            .iter()
            .map(ExplainabilityEnvelope::sequence)
            .collect::<Vec<_>>(),
        vec![1]
    );
    Ok(())
}

pub(crate) async fn test_should_reject_mixed_run_batch_without_touching_either_run(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let run_a = run_id("mixed-a");
    let run_b = run_id("mixed-b");
    store
        .create_run(query_run(
            run_a.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store
        .create_run(query_run(
            run_b.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    let batch = vec![
        envelope_with_sequence(run_a.clone(), 1),
        envelope_with_sequence(run_b.clone(), 1),
    ];

    let error = store
        .append_events(&batch)
        .await
        .expect_err("mixed batch must be rejected");
    assert!(matches!(
        error,
        ExplainabilityStoreError::MixedRunBatch {
            first,
            second,
        } if first == run_a && second == run_b
    ));
    for id in [&run_a, &run_b] {
        assert_eq!(store.get_run(id).await?.expect("run").event_count, 0);
        assert!(stored_events(store, id).await.is_empty());
    }
    Ok(())
}

pub(crate) async fn test_should_not_partially_commit_an_invalid_batch(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let id = run_id("atomic");
    store
        .create_run(query_run(
            id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store
        .append_events(&[envelope_with_sequence(id.clone(), 1)])
        .await?;

    let error = store
        .append_events(&[
            envelope_with_sequence(id.clone(), 2),
            envelope_with_sequence(id.clone(), 4),
        ])
        .await
        .expect_err("gap must reject the whole batch");
    assert!(matches!(
        error,
        ExplainabilityStoreError::SequenceConflict {
            expected: 3,
            actual: 4,
            ..
        }
    ));
    assert_eq!(store.get_run(&id).await?.expect("run").event_count, 1);
    let events = stored_events(store, &id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].sequence(), 1);
    Ok(())
}

pub(crate) async fn test_should_treat_empty_batch_as_noop(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let id = run_id("empty-batch");

    store.append_events(&[]).await?;
    assert_eq!(
        store.get_run(&id).await?,
        None,
        "empty batch must not create a run"
    );

    store
        .create_run(query_run(
            id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store
        .append_events(&[envelope_with_sequence(id.clone(), 1)])
        .await?;
    store.append_events(&[]).await?;
    assert_eq!(store.get_run(&id).await?.expect("run").event_count, 1);
    Ok(())
}

pub(crate) async fn test_should_complete_run_with_terminal_statuses(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    for (status, completed_at) in [
        (ExplainabilityRunStatus::Completed, timestamp(1, 9)),
        (ExplainabilityRunStatus::Failed, timestamp(1, 10)),
        (ExplainabilityRunStatus::Cancelled, timestamp(1, 11)),
    ] {
        let id = run_id(&format!("complete-{status:?}"));
        store
            .create_run(query_run(
                id.clone(),
                timestamp(1, 8),
                ExplainabilityRunStatus::Running,
                None,
            ))
            .await?;
        store
            .append_events(&[envelope_with_sequence(id.clone(), 1)])
            .await?;
        let completion = RunCompletion::new(id.clone(), status, completed_at)?;

        store.complete_run(completion).await?;

        let run = store.get_run(&id).await?.expect("run");
        assert_eq!(run.status, status);
        assert_eq!(run.completed_at, Some(completed_at));
        assert_eq!(run.event_count, 1, "completion must preserve event_count");
    }
    Ok(())
}

pub(crate) async fn test_should_reject_completion_before_run_start(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let id = run_id("bad-time");
    let started_at = timestamp(1, 8);
    store
        .create_run(query_run(
            id.clone(),
            started_at,
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    let completion = RunCompletion::new(
        id.clone(),
        ExplainabilityRunStatus::Completed,
        timestamp(1, 7),
    )?;

    let error = store
        .complete_run(completion)
        .await
        .expect_err("completion before start must be rejected");
    assert!(matches!(
        error,
        ExplainabilityStoreError::InvalidCompletionTime { .. }
    ));
    assert_error_display_has_no_run_content(&error);
    let run = store.get_run(&id).await?.expect("run");
    assert_eq!(run.status, ExplainabilityRunStatus::Running);
    assert_eq!(run.completed_at, None);
    Ok(())
}

pub(crate) async fn test_should_allow_exact_completion_retry(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let id = run_id("exact-retry");
    store
        .create_run(query_run(
            id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    let completed_at = timestamp(1, 9);
    let completion =
        RunCompletion::new(id.clone(), ExplainabilityRunStatus::Completed, completed_at)?;

    store.complete_run(completion.clone()).await?;
    store.complete_run(completion).await?;

    let run = store.get_run(&id).await?.expect("run");
    assert_eq!(run.status, ExplainabilityRunStatus::Completed);
    assert_eq!(run.completed_at, Some(completed_at));
    Ok(())
}

pub(crate) async fn test_should_reject_conflicting_completion(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let id = run_id("completion-conflict");
    store
        .create_run(query_run(
            id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    let completed_at = timestamp(1, 9);
    store
        .complete_run(RunCompletion::new(
            id.clone(),
            ExplainabilityRunStatus::Completed,
            completed_at,
        )?)
        .await?;

    let different_status = store
        .complete_run(RunCompletion::new(
            id.clone(),
            ExplainabilityRunStatus::Failed,
            completed_at,
        )?)
        .await
        .expect_err("different terminal status must conflict");
    assert!(matches!(
        different_status,
        ExplainabilityStoreError::CompletionConflict { .. }
    ));
    assert_error_display_has_no_run_content(&different_status);

    let different_time = store
        .complete_run(RunCompletion::new(
            id.clone(),
            ExplainabilityRunStatus::Completed,
            timestamp(1, 10),
        )?)
        .await
        .expect_err("different completion time must conflict");
    assert!(matches!(
        different_time,
        ExplainabilityStoreError::CompletionConflict { .. }
    ));

    let run = store.get_run(&id).await?.expect("run");
    assert_eq!(run.status, ExplainabilityRunStatus::Completed);
    assert_eq!(run.completed_at, Some(completed_at));
    Ok(())
}

pub(crate) async fn test_should_reject_completion_with_non_terminal_status() -> TestResult {
    let error = RunCompletion::new(
        run_id("pending"),
        ExplainabilityRunStatus::Running,
        timestamp(1, 9),
    )
    .expect_err("non-terminal completion must be rejected");
    assert!(matches!(
        error,
        ExplainabilityStoreError::InvalidCompletionStatus { .. }
    ));
    Ok(())
}

pub(crate) async fn test_should_reject_append_after_terminal_without_changes(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let id = run_id("terminal-append");
    store
        .create_run(query_run(
            id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store
        .append_events(&[envelope_with_sequence(id.clone(), 1)])
        .await?;
    let completed_at = timestamp(1, 9);
    store
        .complete_run(RunCompletion::new(
            id.clone(),
            ExplainabilityRunStatus::Completed,
            completed_at,
        )?)
        .await?;

    let error = store
        .append_events(&[envelope_with_sequence(id.clone(), 2)])
        .await
        .expect_err("terminal run must reject appends");
    assert!(matches!(
        error,
        ExplainabilityStoreError::RunAlreadyTerminal { .. }
    ));
    assert_error_display_has_no_run_content(&error);
    let run = store.get_run(&id).await?.expect("run");
    assert_eq!(run.event_count, 1);
    assert_eq!(run.status, ExplainabilityRunStatus::Completed);
    assert_eq!(run.completed_at, Some(completed_at));
    assert_eq!(stored_events(store, &id).await.len(), 1);
    Ok(())
}

pub(crate) async fn test_should_page_events_by_sequence(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let id = run_id("event-pages");
    store
        .create_run(query_run(
            id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    let batch = (1..=10)
        .map(|sequence| envelope_with_sequence(id.clone(), sequence))
        .collect::<Vec<_>>();
    store.append_events(&batch).await?;

    let first = store
        .load_events(&id, &EventQuery::new().with_limit(3)?)
        .await?;
    assert_eq!(
        first
            .iter()
            .map(ExplainabilityEnvelope::sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    let second = store
        .load_events(&id, &EventQuery::new().after_sequence(3).with_limit(3)?)
        .await?;
    assert_eq!(
        second
            .iter()
            .map(ExplainabilityEnvelope::sequence)
            .collect::<Vec<_>>(),
        vec![4, 5, 6]
    );
    let third = store
        .load_events(&id, &EventQuery::new().after_sequence(9).with_limit(3)?)
        .await?;
    assert_eq!(
        third
            .iter()
            .map(ExplainabilityEnvelope::sequence)
            .collect::<Vec<_>>(),
        vec![10]
    );
    let after_end = store
        .load_events(&id, &EventQuery::new().after_sequence(10).with_limit(3)?)
        .await?;
    assert!(after_end.is_empty());
    Ok(())
}

pub(crate) async fn test_should_distinguish_missing_run_from_empty_events(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let missing = run_id("missing");
    let error = store
        .load_events(&missing, &EventQuery::new())
        .await
        .expect_err("missing run must not look like an empty event page");
    assert!(matches!(
        error,
        ExplainabilityStoreError::RunNotFound { .. }
    ));

    let id = run_id("empty-events");
    store
        .create_run(query_run(
            id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    assert!(stored_events(store, &id).await.is_empty());
    Ok(())
}

pub(crate) async fn test_should_order_run_history_by_start_then_id_descending(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    store
        .create_run(query_run(
            run_id("run-c"),
            timestamp(1, 10),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store
        .create_run(query_run(
            run_id("run-b"),
            timestamp(1, 10),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store
        .create_run(query_run(
            run_id("run-a"),
            timestamp(1, 10),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store
        .create_run(query_run(
            run_id("run-d"),
            timestamp(1, 11),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;

    let runs = store.list_runs(&RunQuery::new()).await?;
    let ids = runs
        .iter()
        .map(|run| run.run_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["run-d", "run-c", "run-b", "run-a"]);
    Ok(())
}

pub(crate) async fn test_should_page_run_history_with_cursor_without_duplicates_or_gaps(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    for (day, id) in [
        (1, "run-1"),
        (2, "run-2"),
        (3, "run-3"),
        (4, "run-4"),
        (5, "run-5"),
    ] {
        store
            .create_run(query_run(
                run_id(id),
                timestamp(1, day),
                ExplainabilityRunStatus::Running,
                None,
            ))
            .await?;
    }

    let first_page = store.list_runs(&RunQuery::new().with_limit(2)?).await?;
    assert_eq!(
        first_page
            .iter()
            .map(|run| run.run_id.as_str())
            .collect::<Vec<_>>(),
        vec!["run-5", "run-4"]
    );
    let cursor = RunListCursor::new(first_page[1].started_at, first_page[1].run_id.clone());
    let second_page = store
        .list_runs(&RunQuery::new().with_limit(2)?.before(cursor.clone()))
        .await?;
    assert_eq!(
        second_page
            .iter()
            .map(|run| run.run_id.as_str())
            .collect::<Vec<_>>(),
        vec!["run-3", "run-2"]
    );
    let third_cursor = RunListCursor::new(second_page[1].started_at, second_page[1].run_id.clone());
    let third_page = store
        .list_runs(&RunQuery::new().with_limit(2)?.before(third_cursor))
        .await?;
    assert_eq!(
        third_page
            .iter()
            .map(|run| run.run_id.as_str())
            .collect::<Vec<_>>(),
        vec!["run-1"]
    );

    store
        .create_run(query_run(
            run_id("run-new"),
            timestamp(1, 6),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    let stable_second_page = store
        .list_runs(&RunQuery::new().with_limit(2)?.before(cursor))
        .await?;
    assert_eq!(
        stable_second_page
            .iter()
            .map(|run| run.run_id.as_str())
            .collect::<Vec<_>>(),
        vec!["run-3", "run-2"],
        "a newer run must not duplicate or skip the cursor page"
    );
    Ok(())
}

pub(crate) async fn test_should_filter_runs_by_kind_status_and_query_method(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    store
        .create_run(query_run(
            run_id("query-local-running"),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            Some(ExplainabilityQueryMethod::Local),
        ))
        .await?;
    for (id, query_method) in [
        ("query-global-completed", ExplainabilityQueryMethod::Global),
        ("query-local-completed", ExplainabilityQueryMethod::Local),
    ] {
        store
            .create_run(query_run(
                run_id(id),
                timestamp(1, 8),
                ExplainabilityRunStatus::Running,
                Some(query_method),
            ))
            .await?;
        store
            .complete_run(RunCompletion::new(
                run_id(id),
                ExplainabilityRunStatus::Completed,
                timestamp(1, 9),
            )?)
            .await?;
    }
    let mut index_run = ExplainabilityRun::new(
        run_id("index-running"),
        ExplainabilityRunKind::Index,
        timestamp(1, 8),
    );
    index_run.status = ExplainabilityRunStatus::Running;
    store.create_run(index_run).await?;

    let query_runs = store
        .list_runs(&RunQuery::new().kind(ExplainabilityRunKind::Query))
        .await?;
    assert_eq!(query_runs.len(), 3);

    let running = store
        .list_runs(&RunQuery::new().status(ExplainabilityRunStatus::Running))
        .await?;
    assert_eq!(running.len(), 2);

    let local = store
        .list_runs(&RunQuery::new().query_method(ExplainabilityQueryMethod::Local))
        .await?;
    let local_ids = local
        .iter()
        .map(|run| run.run_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        local_ids,
        vec!["query-local-running", "query-local-completed"]
    );

    let combined = store
        .list_runs(
            &RunQuery::new()
                .kind(ExplainabilityRunKind::Query)
                .status(ExplainabilityRunStatus::Completed)
                .query_method(ExplainabilityQueryMethod::Local),
        )
        .await?;
    assert_eq!(combined.len(), 1);
    assert_eq!(combined[0].run_id.as_str(), "query-local-completed");
    Ok(())
}

pub(crate) async fn test_should_delete_run_and_events_idempotently(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let deleted = run_id("deleted");
    let other = run_id("other");
    store
        .create_run(query_run(
            deleted.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store
        .append_events(&[envelope_with_sequence(deleted.clone(), 1)])
        .await?;
    store
        .create_run(query_run(
            other.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store
        .append_events(&[envelope_with_sequence(other.clone(), 1)])
        .await?;

    store.delete_run(&deleted).await?;

    assert_eq!(store.get_run(&deleted).await?, None);
    assert!(matches!(
        store.load_events(&deleted, &EventQuery::new()).await,
        Err(ExplainabilityStoreError::RunNotFound { .. })
    ));
    assert_eq!(
        store.get_run(&other).await?.expect("other run").event_count,
        1
    );
    assert_eq!(stored_events(store, &other).await.len(), 1);
    store.delete_run(&deleted).await?;
    Ok(())
}

pub(crate) async fn test_should_serialize_concurrent_same_run_appends(
    store: Arc<dyn ExplainabilityStore>,
) -> TestResult {
    let id = run_id("race-append");
    store
        .create_run(query_run(
            id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store
        .append_events(&[envelope_with_sequence(id.clone(), 1)])
        .await?;
    let barrier = Arc::new(Barrier::new(3));
    let contender = envelope_with_sequence(id.clone(), 2);

    let store_a = Arc::clone(&store);
    let barrier_a = Arc::clone(&barrier);
    let contender_a = contender.clone();
    let task_a = tokio::spawn(async move {
        barrier_a.wait().await;
        store_a.append_events(&[contender_a]).await
    });
    let store_b = Arc::clone(&store);
    let barrier_b = Arc::clone(&barrier);
    let task_b = tokio::spawn(async move {
        barrier_b.wait().await;
        store_b.append_events(&[contender]).await
    });
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
                Err(ExplainabilityStoreError::SequenceConflict { .. })
            )
        })
        .count();
    assert_eq!(successes, 1);
    assert_eq!(conflicts, 1);
    assert_eq!(store.get_run(&id).await?.expect("run").event_count, 2);
    assert_eq!(
        stored_events(store.as_ref(), &id)
            .await
            .iter()
            .map(ExplainabilityEnvelope::sequence)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    Ok(())
}

pub(crate) async fn test_should_linearize_append_and_complete_race(
    store: Arc<dyn ExplainabilityStore>,
) -> TestResult {
    let id = run_id("race-complete");
    store
        .create_run(query_run(
            id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store
        .append_events(&[envelope_with_sequence(id.clone(), 1)])
        .await?;
    let completed_at = timestamp(1, 9);
    let completion =
        RunCompletion::new(id.clone(), ExplainabilityRunStatus::Completed, completed_at)?;
    let barrier = Arc::new(Barrier::new(3));
    let contender = envelope_with_sequence(id.clone(), 2);

    let store_a = Arc::clone(&store);
    let barrier_a = Arc::clone(&barrier);
    let append_task = tokio::spawn(async move {
        barrier_a.wait().await;
        store_a.append_events(&[contender]).await
    });
    let store_b = Arc::clone(&store);
    let barrier_b = Arc::clone(&barrier);
    let complete_task = tokio::spawn(async move {
        barrier_b.wait().await;
        store_b.complete_run(completion).await
    });
    barrier.wait().await;
    let append_result = append_task.await?;
    let complete_result = complete_task.await?;

    let run = store.get_run(&id).await?.expect("run");
    let events = stored_events(store.as_ref(), &id).await;
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
                "unexpected append/complete linearization: append={append:?} complete={complete:?}"
            );
        }
    }
    assert_eq!(
        run.event_count,
        u64::try_from(events.len()).expect("event count"),
        "event_count must always equal the number of stored envelopes"
    );
    Ok(())
}

pub(crate) async fn test_should_isolate_runs_during_lifecycle(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let run_a = run_id("isolate-a");
    let run_b = run_id("isolate-b");
    store
        .create_run(query_run(
            run_a.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store
        .create_run(query_run(
            run_b.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store
        .append_events(&[envelope_with_sequence(run_a.clone(), 1)])
        .await?;
    store
        .append_events(&[envelope_with_sequence(run_b.clone(), 1)])
        .await?;
    store
        .complete_run(RunCompletion::new(
            run_a.clone(),
            ExplainabilityRunStatus::Completed,
            timestamp(1, 9),
        )?)
        .await?;

    let run_b_after = store.get_run(&run_b).await?.expect("run b");
    assert_eq!(run_b_after.status, ExplainabilityRunStatus::Running);
    assert_eq!(run_b_after.completed_at, None);
    assert_eq!(run_b_after.event_count, 1);
    assert_eq!(stored_events(store, &run_b).await.len(), 1);

    store.delete_run(&run_b).await?;
    let run_a_after = store.get_run(&run_a).await?.expect("run a");
    assert_eq!(run_a_after.status, ExplainabilityRunStatus::Completed);
    assert_eq!(run_a_after.event_count, 1);
    Ok(())
}

pub(crate) fn test_should_enforce_query_limits() {
    assert_eq!(RunQuery::new().limit(), DEFAULT_RUN_QUERY_LIMIT);
    assert_eq!(EventQuery::new().limit(), DEFAULT_EVENT_QUERY_LIMIT);
    assert_eq!(MAX_RUN_QUERY_LIMIT, 200);
    assert_eq!(MAX_EVENT_QUERY_LIMIT, 1000);

    for invalid in [0, MAX_RUN_QUERY_LIMIT + 1] {
        let error = RunQuery::new()
            .with_limit(invalid)
            .expect_err("invalid run limit must be rejected");
        assert!(matches!(
            error,
            ExplainabilityStoreError::InvalidLimit { .. }
        ));
    }
    for invalid in [0, MAX_EVENT_QUERY_LIMIT + 1] {
        let error = EventQuery::new()
            .with_limit(invalid)
            .expect_err("invalid event limit must be rejected");
        assert!(matches!(
            error,
            ExplainabilityStoreError::InvalidLimit { .. }
        ));
    }
}

pub(crate) async fn test_should_support_arc_dyn_store_lifecycle(
    store: Arc<dyn ExplainabilityStore>,
) -> TestResult {
    let id = run_id("dyn-lifecycle");
    store
        .create_run(query_run(
            id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            Some(ExplainabilityQueryMethod::Local),
        ))
        .await?;
    store
        .append_events(&[envelope_with_sequence(id.clone(), 1)])
        .await?;
    assert_eq!(store.get_run(&id).await?.expect("run").event_count, 1);
    let completed_at = timestamp(1, 9);
    store
        .complete_run(RunCompletion::new(
            id.clone(),
            ExplainabilityRunStatus::Completed,
            completed_at,
        )?)
        .await?;
    let runs = store.list_runs(&RunQuery::new()).await?;
    assert_eq!(runs.len(), 1);
    let events = store.load_events(&id, &EventQuery::new()).await?;
    assert_eq!(events.len(), 1);
    store.delete_run(&id).await?;
    assert_eq!(store.get_run(&id).await?, None);
    Ok(())
}

pub(crate) async fn test_should_ignore_event_timestamps_in_replay_order(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let id = run_id("timestamp-order");
    store
        .create_run(query_run(
            id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    store
        .append_events(&[
            envelope(id.clone(), 1, timestamp(1, 13)),
            envelope(id.clone(), 2, timestamp(1, 11)),
            envelope(id.clone(), 3, timestamp(1, 12)),
        ])
        .await?;

    let events = stored_events(store, &id).await;
    assert_eq!(
        events
            .iter()
            .map(ExplainabilityEnvelope::sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3],
        "replay order must follow sequence, never event timestamps"
    );
    Ok(())
}

pub(crate) async fn test_should_match_jsonl_round_trip_envelopes(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let id = run_id("jsonl-interop");
    store
        .create_run(query_run(
            id.clone(),
            timestamp(1, 8),
            ExplainabilityRunStatus::Running,
            None,
        ))
        .await?;
    let original = vec![
        envelope(id.clone(), 1, timestamp(1, 13)),
        envelope(id.clone(), 2, timestamp(1, 11)),
        envelope(id.clone(), 3, timestamp(1, 12)),
    ];
    store.append_events(&original).await?;

    let mut round_tripped = Vec::new();
    for envelope in &original {
        let mut line = serde_json::to_vec(envelope)?;
        line.push(b'\n');
        round_tripped.push(serde_json::from_slice::<ExplainabilityEnvelope>(&line)?);
    }

    let stored = stored_events(store, &id).await;
    assert_eq!(
        stored, round_tripped,
        "store must preserve the exact envelope contract"
    );
    for (stored, original) in stored.iter().zip(&original) {
        assert_eq!(stored.schema_version(), original.schema_version());
        assert_eq!(stored.sequence(), original.sequence());
        assert_eq!(stored.record, original.record);
    }
    Ok(())
}

pub(crate) async fn test_should_keep_error_messages_free_of_run_content(
    store: &dyn ExplainabilityStore,
) -> TestResult {
    let id = run_id("content-safe");
    let mut run = query_run(
        id.clone(),
        timestamp(1, 8),
        ExplainabilityRunStatus::Running,
        None,
    );
    run.query = Some(QUERY_SECRET_SENTINEL.to_owned());
    store.create_run(run).await?;

    let sequence_error = store
        .append_events(&[envelope_with_sequence(id.clone(), 2)])
        .await
        .expect_err("sequence error");
    assert_error_display_has_no_run_content(&sequence_error);

    let completion = RunCompletion::new(
        id.clone(),
        ExplainabilityRunStatus::Completed,
        timestamp(1, 9),
    )?;
    store.complete_run(completion).await?;
    let terminal_error = store
        .append_events(&[envelope_with_sequence(id.clone(), 2)])
        .await
        .expect_err("terminal error");
    assert_error_display_has_no_run_content(&terminal_error);
    Ok(())
}
