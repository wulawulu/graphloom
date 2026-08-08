//! Business contract for persisting explainability runs and their event
//! history.
//!
//! The [`ExplainabilityStore`] trait is the host-side persistence contract that
//! Studio and future adapters use. It exposes only business semantics: runs are
//! created and completed explicitly, envelopes are appended with strict
//! per-run sequence continuity, and reads are bounded and deterministic. No
//! database connection, SQL, transaction, or storage-specific type crosses the
//! interface.
//!
//! [`InMemoryExplainabilityStore`] is the Version 1 reference implementation.
//! It serializes all mutations behind one write lock and is safe to share
//! across async tasks, but it is not a database; the next-stage `SQLite`
//! implementation must satisfy exactly the same contract without modifying the
//! trait.

use std::{collections::HashMap, fmt};

use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio::sync::RwLock;

use super::{
    ExplainabilityEnvelope, ExplainabilityQueryMethod, ExplainabilityRun, ExplainabilityRunId,
    ExplainabilityRunKind, ExplainabilityRunStatus,
};

/// Default page size for run history queries.
pub const DEFAULT_RUN_QUERY_LIMIT: u32 = 50;
/// Maximum page size for run history queries.
pub const MAX_RUN_QUERY_LIMIT: u32 = 200;
/// Default page size for event replay queries.
pub const DEFAULT_EVENT_QUERY_LIMIT: u32 = 500;
/// Maximum page size for event replay queries.
pub const MAX_EVENT_QUERY_LIMIT: u32 = 1000;

/// Stable failure categories of the explainability persistence contract.
///
/// Caller and business errors (`RunNotFound`, `SequenceConflict`,
/// `CompletionConflict`, ...) are distinct from backend errors (`Internal`).
/// Error messages may contain run IDs, sequences, and timestamps, but never
/// query text, prompt, context, response, event payloads, or secrets.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ExplainabilityStoreError {
    /// A run with the same identity already exists.
    #[error("run {run_id} already exists")]
    RunAlreadyExists {
        /// Conflicting run identity.
        run_id: ExplainabilityRunId,
    },
    /// The requested run does not exist.
    #[error("run {run_id} does not exist")]
    RunNotFound {
        /// Missing run identity.
        run_id: ExplainabilityRunId,
    },
    /// Run metadata violates a creation or state invariant.
    #[error("run {run_id} has invalid metadata: {reason}")]
    InvalidRunMetadata {
        /// Offending run identity.
        run_id: ExplainabilityRunId,
        /// Low-cardinality reason that never includes user content.
        reason: &'static str,
    },
    /// The run is in a state that does not permit the requested operation.
    #[error("run {run_id} has invalid state {state:?} for this operation")]
    InvalidRunState {
        /// Offending run identity.
        run_id: ExplainabilityRunId,
        /// Current lifecycle state.
        state: ExplainabilityRunStatus,
    },
    /// A terminal run rejected a new event batch.
    #[error("run {run_id} is already terminal")]
    RunAlreadyTerminal {
        /// Terminal run identity.
        run_id: ExplainabilityRunId,
    },
    /// One append batch contained envelopes from different runs.
    #[error("append batch mixes runs {first} and {second}")]
    MixedRunBatch {
        /// First run in the batch.
        first: ExplainabilityRunId,
        /// A different run found later in the batch.
        second: ExplainabilityRunId,
    },
    /// A batch sequence did not continue the stored sequence.
    #[error("sequence conflict for run {run_id}: expected {expected}, found {actual}")]
    SequenceConflict {
        /// Offending run identity.
        run_id: ExplainabilityRunId,
        /// Next required sequence.
        expected: u64,
        /// Sequence found in the batch.
        actual: u64,
    },
    /// The per-run sequence counter overflowed.
    #[error("sequence overflow for run {run_id}")]
    SequenceOverflow {
        /// Run whose sequence space is exhausted.
        run_id: ExplainabilityRunId,
    },
    /// A completion used a non-terminal status.
    #[error("completion status {status:?} is not terminal")]
    InvalidCompletionStatus {
        /// Non-terminal status.
        status: ExplainabilityRunStatus,
    },
    /// A completion timestamp preceded the run start.
    #[error("completion time {completed_at} is earlier than run start {started_at}")]
    InvalidCompletionTime {
        /// Offending run identity.
        run_id: ExplainabilityRunId,
        /// Completion timestamp that violated the invariant.
        completed_at: DateTime<Utc>,
        /// Run start timestamp.
        started_at: DateTime<Utc>,
    },
    /// A second completion disagreed with the stored terminal state.
    #[error("completion conflicts with the existing terminal state of run {run_id}")]
    CompletionConflict {
        /// Run whose terminal state is already fixed.
        run_id: ExplainabilityRunId,
    },
    /// A query limit fell outside the supported range.
    #[error("{kind} query limit {limit} must be within {min}..={max}")]
    InvalidLimit {
        /// Query category, such as `run history` or `event replay`.
        kind: &'static str,
        /// Rejected limit.
        limit: u32,
        /// Inclusive lower bound.
        min: u32,
        /// Inclusive upper bound.
        max: u32,
    },
    /// A backend implementation failed internally.
    #[error("internal explainability store failure during {operation}")]
    Internal {
        /// Operation being performed when the backend failed.
        operation: &'static str,
        /// Underlying backend error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// Terminal completion metadata for one run.
///
/// Construction rejects non-terminal statuses so callers cannot create an
/// invalid completion through the public API.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RunCompletion {
    run_id: ExplainabilityRunId,
    status: ExplainabilityRunStatus,
    completed_at: DateTime<Utc>,
}

impl RunCompletion {
    /// Create a terminal completion.
    ///
    /// # Errors
    ///
    /// Returns [`ExplainabilityStoreError::InvalidCompletionStatus`] when
    /// `status` is `Pending` or `Running`.
    pub fn new(
        run_id: ExplainabilityRunId,
        status: ExplainabilityRunStatus,
        completed_at: DateTime<Utc>,
    ) -> Result<Self, ExplainabilityStoreError> {
        if !is_terminal(status) {
            return Err(ExplainabilityStoreError::InvalidCompletionStatus { status });
        }
        Ok(Self {
            run_id,
            status,
            completed_at,
        })
    }

    /// Return the completed run identity.
    #[must_use]
    pub const fn run_id(&self) -> &ExplainabilityRunId {
        &self.run_id
    }

    /// Return the terminal completion status.
    #[must_use]
    pub const fn status(&self) -> ExplainabilityRunStatus {
        self.status
    }

    /// Return the completion timestamp.
    #[must_use]
    pub const fn completed_at(&self) -> DateTime<Utc> {
        self.completed_at
    }
}

/// Deterministic run-history page cursor.
///
/// A cursor addresses the exact `(started_at, run_id)` position of a previous
/// page. The next page contains runs strictly older than that position under
/// `started_at DESC, run_id DESC` ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RunListCursor {
    started_at: DateTime<Utc>,
    run_id: ExplainabilityRunId,
}

impl RunListCursor {
    /// Create a cursor from the run that ended a previous page.
    #[must_use]
    pub const fn new(started_at: DateTime<Utc>, run_id: ExplainabilityRunId) -> Self {
        Self { started_at, run_id }
    }

    /// Return the cursor start timestamp.
    #[must_use]
    pub const fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }

    /// Return the cursor run identity.
    #[must_use]
    pub const fn run_id(&self) -> &ExplainabilityRunId {
        &self.run_id
    }
}

/// Filtered, cursor-based run history query.
///
/// All filters are combined with AND. Results are ordered by
/// `started_at DESC, run_id DESC` and bounded by `limit`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RunQuery {
    kind: Option<ExplainabilityRunKind>,
    status: Option<ExplainabilityRunStatus>,
    query_method: Option<ExplainabilityQueryMethod>,
    before: Option<RunListCursor>,
    limit: u32,
}

impl RunQuery {
    /// Create an unfiltered query with the default limit.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            kind: None,
            status: None,
            query_method: None,
            before: None,
            limit: DEFAULT_RUN_QUERY_LIMIT,
        }
    }

    /// Filter by run kind.
    #[must_use]
    pub const fn kind(mut self, kind: ExplainabilityRunKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Filter by lifecycle status.
    #[must_use]
    pub const fn status(mut self, status: ExplainabilityRunStatus) -> Self {
        self.status = Some(status);
        self
    }

    /// Filter to query runs with the given query method.
    ///
    /// Non-query runs never match this filter.
    #[must_use]
    pub const fn query_method(mut self, query_method: ExplainabilityQueryMethod) -> Self {
        self.query_method = Some(query_method);
        self
    }

    /// Return runs strictly older than the cursor position.
    #[must_use]
    pub fn before(mut self, cursor: RunListCursor) -> Self {
        self.before = Some(cursor);
        self
    }

    /// Set the page size within `1..=MAX_RUN_QUERY_LIMIT`.
    ///
    /// # Errors
    ///
    /// Returns [`ExplainabilityStoreError::InvalidLimit`] when the limit is
    /// zero or exceeds [`MAX_RUN_QUERY_LIMIT`]. Invalid limits are never
    /// silently clamped.
    pub fn with_limit(mut self, limit: u32) -> Result<Self, ExplainabilityStoreError> {
        validate_limit(limit, 1, MAX_RUN_QUERY_LIMIT, "run history")?;
        self.limit = limit;
        Ok(self)
    }

    /// Return the run-kind filter.
    #[must_use]
    pub const fn kind_filter(&self) -> Option<ExplainabilityRunKind> {
        self.kind
    }

    /// Return the status filter.
    #[must_use]
    pub const fn status_filter(&self) -> Option<ExplainabilityRunStatus> {
        self.status
    }

    /// Return the query-method filter.
    #[must_use]
    pub const fn query_method_filter(&self) -> Option<ExplainabilityQueryMethod> {
        self.query_method
    }

    /// Return the cursor filter.
    #[must_use]
    pub const fn before_cursor(&self) -> Option<&RunListCursor> {
        self.before.as_ref()
    }

    /// Return the page size.
    #[must_use]
    pub const fn limit(&self) -> u32 {
        self.limit
    }
}

impl Default for RunQuery {
    fn default() -> Self {
        Self::new()
    }
}

/// Bounded event replay query for one run.
///
/// Events are returned in `sequence ASC` order, strictly after
/// `after_sequence`, with at most `limit` envelopes.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct EventQuery {
    after_sequence: Option<u64>,
    limit: u32,
}

impl EventQuery {
    /// Create a query starting at sequence 1 with the default limit.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            after_sequence: None,
            limit: DEFAULT_EVENT_QUERY_LIMIT,
        }
    }

    /// Return only envelopes with sequence strictly greater than `sequence`.
    #[must_use]
    pub const fn after_sequence(mut self, sequence: u64) -> Self {
        self.after_sequence = Some(sequence);
        self
    }

    /// Set the page size within `1..=MAX_EVENT_QUERY_LIMIT`.
    ///
    /// # Errors
    ///
    /// Returns [`ExplainabilityStoreError::InvalidLimit`] when the limit is
    /// zero or exceeds [`MAX_EVENT_QUERY_LIMIT`]. Invalid limits are never
    /// silently clamped.
    pub fn with_limit(mut self, limit: u32) -> Result<Self, ExplainabilityStoreError> {
        validate_limit(limit, 1, MAX_EVENT_QUERY_LIMIT, "event replay")?;
        self.limit = limit;
        Ok(self)
    }

    /// Return the exclusive lower sequence bound.
    #[must_use]
    pub const fn after_sequence_bound(&self) -> Option<u64> {
        self.after_sequence
    }

    /// Return the page size.
    #[must_use]
    pub const fn limit(&self) -> u32 {
        self.limit
    }
}

impl Default for EventQuery {
    fn default() -> Self {
        Self::new()
    }
}

/// Host-side persistence contract for explainability runs and events.
///
/// Implementations must keep `ExplainabilityRun.event_count` equal to the
/// number of successfully persisted envelopes, append batches all-or-nothing
/// with contiguous per-run sequences, and make completions and deletions
/// idempotent under the documented conflict rules. The trait is object-safe so
/// hosts can share `Arc<dyn ExplainabilityStore>` across async tasks.
#[async_trait::async_trait]
pub trait ExplainabilityStore: Send + Sync + fmt::Debug {
    /// Create a new run with `event_count == 0` and a non-terminal status.
    ///
    /// # Errors
    ///
    /// Returns `RunAlreadyExists`, `InvalidRunMetadata`, or `InvalidRunState`
    /// when the run violates a creation invariant.
    async fn create_run(&self, run: ExplainabilityRun) -> Result<(), ExplainabilityStoreError>;

    /// Atomically append one run's envelopes with contiguous sequences.
    ///
    /// An empty batch is a no-op. Any validation failure rejects the whole
    /// batch without changing `event_count` or stored events.
    ///
    /// # Errors
    ///
    /// Returns `MixedRunBatch`, `RunNotFound`, `RunAlreadyTerminal`,
    /// `SequenceConflict`, or `SequenceOverflow` when the batch is invalid.
    async fn append_events(
        &self,
        events: &[ExplainabilityEnvelope],
    ) -> Result<(), ExplainabilityStoreError>;

    /// Transition a run to a terminal status exactly once.
    ///
    /// Repeating the exact same terminal status and timestamp succeeds as an
    /// idempotent retry; any different terminal completion conflicts.
    ///
    /// # Errors
    ///
    /// Returns `RunNotFound`, `InvalidCompletionStatus`,
    /// `InvalidCompletionTime`, or `CompletionConflict` when the completion is
    /// invalid.
    async fn complete_run(&self, completion: RunCompletion)
    -> Result<(), ExplainabilityStoreError>;

    /// Return an owned copy of one run, or `None` when it does not exist.
    ///
    /// # Errors
    ///
    /// Returns an internal backend error when the store cannot read.
    async fn get_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<Option<ExplainabilityRun>, ExplainabilityStoreError>;

    /// List runs filtered by kind, status, query method, and cursor.
    ///
    /// Results are ordered by `started_at DESC, run_id DESC` and bounded by
    /// `query.limit()`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidLimit` or an internal backend error.
    async fn list_runs(
        &self,
        query: &RunQuery,
    ) -> Result<Vec<ExplainabilityRun>, ExplainabilityStoreError>;

    /// Load a bounded page of one run's envelopes in sequence order.
    ///
    /// # Errors
    ///
    /// Returns `RunNotFound` when the run does not exist, `InvalidLimit`, or
    /// an internal backend error.
    async fn load_events(
        &self,
        run_id: &ExplainabilityRunId,
        query: &EventQuery,
    ) -> Result<Vec<ExplainabilityEnvelope>, ExplainabilityStoreError>;

    /// Delete a run and all of its events atomically.
    ///
    /// Deleting a missing run succeeds.
    ///
    /// # Errors
    ///
    /// Returns an internal backend error when the store cannot delete.
    async fn delete_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<(), ExplainabilityStoreError>;
}

/// In-memory reference implementation of [`ExplainabilityStore`].
///
/// All mutations are validated and committed under one write lock, so a batch
/// never partially applies and concurrent writers cannot lose updates. Reads
/// clone owned DTOs and release the lock before returning. This store is a
/// reference and development backend, not a database.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct InMemoryExplainabilityStore {
    state: RwLock<MemoryState>,
}

impl InMemoryExplainabilityStore {
    /// Create an empty in-memory store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clone the metadata of every run matching `query`.
    ///
    /// The read lock is scoped to this function's lexical block and is
    /// released as soon as the owned metadata has been cloned; ordering and
    /// truncation happen outside the lock in [`order_and_limit_runs`].
    #[must_use]
    async fn collect_matching_runs(&self, query: &RunQuery) -> Vec<ExplainabilityRun> {
        {
            let state = self.state.read().await;
            state
                .runs
                .values()
                .filter(|run| query.kind_filter().is_none_or(|kind| run.kind == kind))
                .filter(|run| {
                    query
                        .status_filter()
                        .is_none_or(|status| run.status == status)
                })
                .filter(|run| {
                    query
                        .query_method_filter()
                        .is_none_or(|method| run.query_method == Some(method))
                })
                .filter(|run| {
                    query
                        .before_cursor()
                        .is_none_or(|cursor| is_strictly_older_than(run, cursor))
                })
                .cloned()
                .collect()
        }
    }
}

/// Shared mutable state of the in-memory store.
#[derive(Debug, Default)]
struct MemoryState {
    runs: HashMap<ExplainabilityRunId, ExplainabilityRun>,
    events: HashMap<ExplainabilityRunId, Vec<ExplainabilityEnvelope>>,
}

#[async_trait::async_trait]
impl ExplainabilityStore for InMemoryExplainabilityStore {
    async fn create_run(&self, run: ExplainabilityRun) -> Result<(), ExplainabilityStoreError> {
        validate_create_run(&run)?;
        let mut state = self.state.write().await;
        if state.runs.contains_key(&run.run_id) {
            return Err(ExplainabilityStoreError::RunAlreadyExists { run_id: run.run_id });
        }
        state.runs.insert(run.run_id.clone(), run);
        Ok(())
    }

    async fn append_events(
        &self,
        events: &[ExplainabilityEnvelope],
    ) -> Result<(), ExplainabilityStoreError> {
        let Some(first) = events.first() else {
            return Ok(());
        };
        let first_run = &first.record.run_id;
        if let Some(mixed) = events
            .iter()
            .find(|envelope| envelope.record.run_id != *first_run)
        {
            return Err(ExplainabilityStoreError::MixedRunBatch {
                first: first_run.clone(),
                second: mixed.record.run_id.clone(),
            });
        }

        let mut state = self.state.write().await;
        let run =
            state
                .runs
                .get_mut(first_run)
                .ok_or_else(|| ExplainabilityStoreError::RunNotFound {
                    run_id: first_run.clone(),
                })?;
        if is_terminal(run.status) {
            return Err(ExplainabilityStoreError::RunAlreadyTerminal {
                run_id: first_run.clone(),
            });
        }

        let mut next_sequence = run.event_count;
        let mut pending = Vec::with_capacity(events.len());
        for envelope in events {
            let expected = next_sequence.checked_add(1).ok_or_else(|| {
                ExplainabilityStoreError::SequenceOverflow {
                    run_id: first_run.clone(),
                }
            })?;
            if envelope.sequence() != expected {
                return Err(ExplainabilityStoreError::SequenceConflict {
                    run_id: first_run.clone(),
                    expected,
                    actual: envelope.sequence(),
                });
            }
            next_sequence = expected;
            pending.push(envelope.clone());
        }

        run.event_count = next_sequence;
        state
            .events
            .entry(first_run.clone())
            .or_default()
            .extend(pending);
        Ok(())
    }

    async fn complete_run(
        &self,
        completion: RunCompletion,
    ) -> Result<(), ExplainabilityStoreError> {
        if !is_terminal(completion.status()) {
            return Err(ExplainabilityStoreError::InvalidCompletionStatus {
                status: completion.status(),
            });
        }
        let mut state = self.state.write().await;
        let run = state.runs.get_mut(completion.run_id()).ok_or_else(|| {
            ExplainabilityStoreError::RunNotFound {
                run_id: completion.run_id().clone(),
            }
        })?;
        if completion.completed_at() < run.started_at {
            return Err(ExplainabilityStoreError::InvalidCompletionTime {
                run_id: run.run_id.clone(),
                completed_at: completion.completed_at(),
                started_at: run.started_at,
            });
        }
        if run.completed_at.is_some() || is_terminal(run.status) {
            if run.status == completion.status()
                && run.completed_at == Some(completion.completed_at())
            {
                return Ok(());
            }
            return Err(ExplainabilityStoreError::CompletionConflict {
                run_id: run.run_id.clone(),
            });
        }
        run.status = completion.status();
        run.completed_at = Some(completion.completed_at());
        Ok(())
    }

    async fn get_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<Option<ExplainabilityRun>, ExplainabilityStoreError> {
        let state = self.state.read().await;
        Ok(state.runs.get(run_id).cloned())
    }

    async fn list_runs(
        &self,
        query: &RunQuery,
    ) -> Result<Vec<ExplainabilityRun>, ExplainabilityStoreError> {
        validate_limit(query.limit(), 1, MAX_RUN_QUERY_LIMIT, "run history")?;
        let mut runs = self.collect_matching_runs(query).await;
        order_and_limit_runs(&mut runs, query.limit());
        Ok(runs)
    }

    async fn load_events(
        &self,
        run_id: &ExplainabilityRunId,
        query: &EventQuery,
    ) -> Result<Vec<ExplainabilityEnvelope>, ExplainabilityStoreError> {
        validate_limit(query.limit(), 1, MAX_EVENT_QUERY_LIMIT, "event replay")?;
        let state = self.state.read().await;
        if !state.runs.contains_key(run_id) {
            return Err(ExplainabilityStoreError::RunNotFound {
                run_id: run_id.clone(),
            });
        }
        let after = query.after_sequence_bound().unwrap_or(0);
        Ok(state
            .events
            .get(run_id)
            .into_iter()
            .flatten()
            .filter(|envelope| envelope.sequence() > after)
            .take(limit_to_usize(query.limit()))
            .cloned()
            .collect())
    }

    async fn delete_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<(), ExplainabilityStoreError> {
        let mut state = self.state.write().await;
        state.runs.remove(run_id);
        state.events.remove(run_id);
        Ok(())
    }
}

/// Sort cloned run metadata and apply the page limit.
///
/// Runs are ordered by `started_at DESC, run_id DESC` and truncated to
/// `limit`. This is pure CPU work and must never hold the store lock.
fn order_and_limit_runs(runs: &mut Vec<ExplainabilityRun>, limit: u32) {
    runs.sort_by(|left, right| {
        right
            .started_at
            .cmp(&left.started_at)
            .then_with(|| right.run_id.as_str().cmp(left.run_id.as_str()))
    });
    runs.truncate(limit_to_usize(limit));
}

/// Validate creation invariants for a new run.
pub(super) fn validate_create_run(run: &ExplainabilityRun) -> Result<(), ExplainabilityStoreError> {
    if run.event_count != 0 {
        return Err(ExplainabilityStoreError::InvalidRunMetadata {
            run_id: run.run_id.clone(),
            reason: "initial event_count must be 0",
        });
    }
    if run.completed_at.is_some() {
        return Err(ExplainabilityStoreError::InvalidRunMetadata {
            run_id: run.run_id.clone(),
            reason: "a new run cannot already be completed",
        });
    }
    if is_terminal(run.status) {
        return Err(ExplainabilityStoreError::InvalidRunState {
            run_id: run.run_id.clone(),
            state: run.status,
        });
    }
    if run.kind != ExplainabilityRunKind::Query && run.query_method.is_some() {
        return Err(ExplainabilityStoreError::InvalidRunMetadata {
            run_id: run.run_id.clone(),
            reason: "query_method is only valid for query runs",
        });
    }
    Ok(())
}

/// Return whether a status is terminal.
pub(super) const fn is_terminal(status: ExplainabilityRunStatus) -> bool {
    matches!(
        status,
        ExplainabilityRunStatus::Completed
            | ExplainabilityRunStatus::Failed
            | ExplainabilityRunStatus::Cancelled
    )
}

/// Validate a query limit against a documented inclusive range.
pub(super) fn validate_limit(
    limit: u32,
    min: u32,
    max: u32,
    kind: &'static str,
) -> Result<(), ExplainabilityStoreError> {
    if (min..=max).contains(&limit) {
        Ok(())
    } else {
        Err(ExplainabilityStoreError::InvalidLimit {
            kind,
            limit,
            min,
            max,
        })
    }
}

/// Return whether a run sorts strictly after a cursor in history order.
///
/// History order is `started_at DESC, run_id DESC`; "after the cursor" means
/// the run is strictly older: smaller `started_at`, or equal `started_at`
/// with a smaller `run_id`.
fn is_strictly_older_than(run: &ExplainabilityRun, cursor: &RunListCursor) -> bool {
    run.started_at < cursor.started_at()
        || (run.started_at == cursor.started_at() && run.run_id.as_str() < cursor.run_id().as_str())
}

/// Convert a validated `u32` limit to `usize`.
///
/// `u32` always fits `usize` on every supported `GraphLoom` target; the
/// fallback only keeps the conversion total for the type system.
fn limit_to_usize(limit: u32) -> usize {
    usize::try_from(limit).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::{
        DEFAULT_EVENT_QUERY_LIMIT, DEFAULT_RUN_QUERY_LIMIT, EventQuery, RunListCursor, RunQuery,
        is_strictly_older_than, is_terminal, order_and_limit_runs, validate_limit,
    };
    use crate::explainability::{
        ExplainabilityRun, ExplainabilityRunId, ExplainabilityRunKind, ExplainabilityRunStatus,
        ExplainabilityStore, ExplainabilityStoreError, InMemoryExplainabilityStore,
    };

    fn run_id(value: &str) -> ExplainabilityRunId {
        value.parse().expect("run id")
    }

    fn timestamp(hour: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc
            .with_ymd_and_hms(2026, 1, 1, hour, 0, 0)
            .unwrap()
    }

    #[test]
    fn test_should_classify_terminal_statuses() {
        assert!(is_terminal(ExplainabilityRunStatus::Completed));
        assert!(is_terminal(ExplainabilityRunStatus::Failed));
        assert!(is_terminal(ExplainabilityRunStatus::Cancelled));
        assert!(!is_terminal(ExplainabilityRunStatus::Pending));
        assert!(!is_terminal(ExplainabilityRunStatus::Running));
    }

    #[test]
    fn test_should_validate_query_limits_at_boundaries() {
        assert!(validate_limit(1, 1, 200, "run history").is_ok());
        assert!(validate_limit(200, 1, 200, "run history").is_ok());
        assert!(matches!(
            validate_limit(0, 1, 200, "run history"),
            Err(ExplainabilityStoreError::InvalidLimit { .. })
        ));
        assert!(matches!(
            validate_limit(201, 1, 200, "run history"),
            Err(ExplainabilityStoreError::InvalidLimit { .. })
        ));
    }

    #[test]
    fn test_should_compare_runs_strictly_after_cursor() {
        let cursor = RunListCursor::new(timestamp(10), run_id("run-b"));
        let older_time =
            ExplainabilityRun::new(run_id("run-c"), ExplainabilityRunKind::Query, timestamp(9));
        assert!(is_strictly_older_than(&older_time, &cursor));

        let same_time_smaller_id =
            ExplainabilityRun::new(run_id("run-a"), ExplainabilityRunKind::Query, timestamp(10));
        assert!(is_strictly_older_than(&same_time_smaller_id, &cursor));

        let newer =
            ExplainabilityRun::new(run_id("run-a"), ExplainabilityRunKind::Query, timestamp(11));
        assert!(!is_strictly_older_than(&newer, &cursor));

        let same_position =
            ExplainabilityRun::new(run_id("run-b"), ExplainabilityRunKind::Query, timestamp(10));
        assert!(!is_strictly_older_than(&same_position, &cursor));
    }

    #[test]
    fn test_should_default_query_limits() {
        assert_eq!(RunQuery::new().limit(), DEFAULT_RUN_QUERY_LIMIT);
        assert_eq!(EventQuery::new().limit(), DEFAULT_EVENT_QUERY_LIMIT);
        assert_eq!(EventQuery::new().after_sequence_bound(), None);
    }

    #[test]
    fn test_should_reject_invalid_query_limits_in_constructors() {
        assert!(matches!(
            RunQuery::new().with_limit(0),
            Err(ExplainabilityStoreError::InvalidLimit { .. })
        ));
        assert!(matches!(
            RunQuery::new().with_limit(201),
            Err(ExplainabilityStoreError::InvalidLimit { .. })
        ));
        assert!(matches!(
            EventQuery::new().with_limit(0),
            Err(ExplainabilityStoreError::InvalidLimit { .. })
        ));
        assert!(matches!(
            EventQuery::new().with_limit(1001),
            Err(ExplainabilityStoreError::InvalidLimit { .. })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_should_release_read_lock_before_ordering_runs() {
        let store = InMemoryExplainabilityStore::new();
        for index in 0..100 {
            let mut run = ExplainabilityRun::new(
                run_id(&format!("run-{index:03}")),
                ExplainabilityRunKind::Query,
                timestamp(index % 24),
            );
            run.status = ExplainabilityRunStatus::Running;
            store.create_run(run).await.expect("create run");
        }
        let query = RunQuery::new().with_limit(7).expect("limit");

        let collected = store.collect_matching_runs(&query).await;
        let write_lock = store.state.try_write();
        assert!(
            write_lock.is_ok(),
            "collect_matching_runs must release the read lock before returning"
        );
        drop(write_lock);

        store
            .delete_run(&run_id("missing-run"))
            .await
            .expect("writer must proceed while sorting is pending");

        let mut ordered = collected.clone();
        order_and_limit_runs(&mut ordered, query.limit());
        let listed = store.list_runs(&query).await.expect("list runs");
        assert_eq!(
            ordered, listed,
            "collect + order_and_limit must match list_runs exactly"
        );
    }
}
