//! Public contract tests for the in-memory explainability store.
//!
//! These tests exercise [`InMemoryExplainabilityStore`] through the public
//! [`ExplainabilityStore`] trait from an external crate perspective and pin
//! the Version 1 business invariants. The exact same scenario functions are
//! executed against `SqliteExplainabilityStore` in
//! `explainability_sqlite_store.rs`; neither backend may relax an assertion.

#[path = "support/explainability_store_contract.rs"]
mod contract;

use std::{error::Error, sync::Arc};

use graphloom::explainability::{ExplainabilityStore, InMemoryExplainabilityStore};

type TestResult = Result<(), Box<dyn Error>>;

fn memory_store() -> Arc<dyn ExplainabilityStore> {
    Arc::new(InMemoryExplainabilityStore::new())
}

#[tokio::test]
async fn test_should_create_run_and_return_owned_copy() -> TestResult {
    contract::test_should_create_run_and_return_owned_copy(memory_store().as_ref()).await
}

#[tokio::test]
async fn test_should_reject_duplicate_run_without_overwriting() -> TestResult {
    contract::test_should_reject_duplicate_run_without_overwriting(memory_store().as_ref()).await
}

#[tokio::test]
async fn test_should_reject_nonzero_initial_event_count() -> TestResult {
    contract::test_should_reject_nonzero_initial_event_count(memory_store().as_ref()).await
}

#[tokio::test]
async fn test_should_reject_terminal_initial_statuses_and_completion_time() -> TestResult {
    contract::test_should_reject_terminal_initial_statuses_and_completion_time(
        memory_store().as_ref(),
    )
    .await
}

#[tokio::test]
async fn test_should_reject_query_method_on_non_query_run() -> TestResult {
    contract::test_should_reject_query_method_on_non_query_run(memory_store().as_ref()).await
}

#[tokio::test]
async fn test_should_append_contiguous_sequences_and_derive_event_count() -> TestResult {
    contract::test_should_append_contiguous_sequences_and_derive_event_count(
        memory_store().as_ref(),
    )
    .await
}

#[tokio::test]
async fn test_should_reject_non_contiguous_sequences_without_partial_write() -> TestResult {
    contract::test_should_reject_non_contiguous_sequences_without_partial_write(
        memory_store().as_ref(),
    )
    .await
}

#[tokio::test]
async fn test_should_reject_mixed_run_batch_without_touching_either_run() -> TestResult {
    contract::test_should_reject_mixed_run_batch_without_touching_either_run(
        memory_store().as_ref(),
    )
    .await
}

#[tokio::test]
async fn test_should_not_partially_commit_an_invalid_batch() -> TestResult {
    contract::test_should_not_partially_commit_an_invalid_batch(memory_store().as_ref()).await
}

#[tokio::test]
async fn test_should_treat_empty_batch_as_noop() -> TestResult {
    contract::test_should_treat_empty_batch_as_noop(memory_store().as_ref()).await
}

#[tokio::test]
async fn test_should_complete_run_with_terminal_statuses() -> TestResult {
    contract::test_should_complete_run_with_terminal_statuses(memory_store().as_ref()).await
}

#[tokio::test]
async fn test_should_reject_completion_before_run_start() -> TestResult {
    contract::test_should_reject_completion_before_run_start(memory_store().as_ref()).await
}

#[tokio::test]
async fn test_should_allow_exact_completion_retry() -> TestResult {
    contract::test_should_allow_exact_completion_retry(memory_store().as_ref()).await
}

#[tokio::test]
async fn test_should_reject_conflicting_completion() -> TestResult {
    contract::test_should_reject_conflicting_completion(memory_store().as_ref()).await
}

#[tokio::test]
async fn test_should_reject_completion_with_non_terminal_status() -> TestResult {
    contract::test_should_reject_completion_with_non_terminal_status().await
}

#[tokio::test]
async fn test_should_reject_append_after_terminal_without_changes() -> TestResult {
    contract::test_should_reject_append_after_terminal_without_changes(memory_store().as_ref())
        .await
}

#[tokio::test]
async fn test_should_page_events_by_sequence() -> TestResult {
    contract::test_should_page_events_by_sequence(memory_store().as_ref()).await
}

#[tokio::test]
async fn test_should_distinguish_missing_run_from_empty_events() -> TestResult {
    contract::test_should_distinguish_missing_run_from_empty_events(memory_store().as_ref()).await
}

#[tokio::test]
async fn test_should_order_run_history_by_start_then_id_descending() -> TestResult {
    contract::test_should_order_run_history_by_start_then_id_descending(memory_store().as_ref())
        .await
}

#[tokio::test]
async fn test_should_page_run_history_with_cursor_without_duplicates_or_gaps() -> TestResult {
    contract::test_should_page_run_history_with_cursor_without_duplicates_or_gaps(
        memory_store().as_ref(),
    )
    .await
}

#[tokio::test]
async fn test_should_filter_runs_by_kind_status_and_query_method() -> TestResult {
    contract::test_should_filter_runs_by_kind_status_and_query_method(memory_store().as_ref()).await
}

#[tokio::test]
async fn test_should_delete_run_and_events_idempotently() -> TestResult {
    contract::test_should_delete_run_and_events_idempotently(memory_store().as_ref()).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_should_serialize_concurrent_same_run_appends() -> TestResult {
    contract::test_should_serialize_concurrent_same_run_appends(memory_store()).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_should_linearize_append_and_complete_race() -> TestResult {
    contract::test_should_linearize_append_and_complete_race(memory_store()).await
}

#[tokio::test]
async fn test_should_isolate_runs_during_lifecycle() -> TestResult {
    contract::test_should_isolate_runs_during_lifecycle(memory_store().as_ref()).await
}

#[test]
fn test_should_enforce_query_limits() {
    contract::test_should_enforce_query_limits();
}

#[tokio::test]
async fn test_should_support_arc_dyn_store_lifecycle() -> TestResult {
    contract::test_should_support_arc_dyn_store_lifecycle(memory_store()).await
}

#[tokio::test]
async fn test_should_ignore_event_timestamps_in_replay_order() -> TestResult {
    contract::test_should_ignore_event_timestamps_in_replay_order(memory_store().as_ref()).await
}

#[tokio::test]
async fn test_should_match_jsonl_round_trip_envelopes() -> TestResult {
    contract::test_should_match_jsonl_round_trip_envelopes(memory_store().as_ref()).await
}

#[tokio::test]
async fn test_should_keep_error_messages_free_of_run_content() -> TestResult {
    contract::test_should_keep_error_messages_free_of_run_content(memory_store().as_ref()).await
}
