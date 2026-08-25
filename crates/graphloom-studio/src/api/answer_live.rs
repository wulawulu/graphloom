//! Bounded, in-memory transport for live Studio Query answers.

use std::{
    collections::VecDeque,
    fmt,
    num::NonZeroUsize,
    sync::{Arc, Mutex},
};

use dashmap::{DashMap, mapref::entry::Entry};
use graphloom::explainability::ExplainabilityRunId;
use serde::Serialize;
use tokio::sync::broadcast;

const CHANNEL_CAPACITY: usize = 64;

/// Current lifecycle of an answer retained by the live transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum LiveAnswerStatus {
    /// The Core Query stream is still being consumed.
    #[serde(rename = "streaming")]
    Running,
    /// The canonical result is ready.
    Completed,
    /// Query execution ended without a canonical result.
    Failed,
}

impl LiveAnswerStatus {
    pub(super) const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

/// Authoritative current snapshot for one live/recent answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LiveAnswerState {
    pub(super) sequence: u64,
    pub(super) text: String,
    pub(super) status: LiveAnswerStatus,
}

impl Default for LiveAnswerState {
    fn default() -> Self {
        Self {
            sequence: 0,
            text: String::new(),
            status: LiveAnswerStatus::Running,
        }
    }
}

/// Browser-facing answer event. Error details intentionally never cross this boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum QueryAnswerEvent {
    /// Complete current state sent at connection and lag recovery boundaries.
    Snapshot {
        sequence: u64,
        text: String,
        status: LiveAnswerStatus,
    },
    /// One unmodified Core `QueryEvent::Token` delta.
    Delta { sequence: u64, delta: String },
    /// Canonical result is ready at the result endpoint.
    Completed { sequence: u64 },
    /// Query execution failed without a canonical result.
    Failed { sequence: u64 },
}

impl QueryAnswerEvent {
    pub(super) const fn sequence(&self) -> u64 {
        match self {
            Self::Snapshot { sequence, .. }
            | Self::Delta { sequence, .. }
            | Self::Completed { sequence }
            | Self::Failed { sequence } => *sequence,
        }
    }
}

struct LiveRun {
    state: LiveAnswerState,
    sender: broadcast::Sender<QueryAnswerEvent>,
}

impl fmt::Debug for LiveRun {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LiveRun { .. }")
    }
}

/// Per-run answer hub with race-free snapshots and bounded terminal retention.
pub(super) struct QueryAnswerLiveHub {
    terminal_capacity: NonZeroUsize,
    runs: DashMap<ExplainabilityRunId, LiveRun>,
    terminal_order: Mutex<VecDeque<ExplainabilityRunId>>,
}

impl fmt::Debug for QueryAnswerLiveHub {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("QueryAnswerLiveHub { .. }")
    }
}

impl QueryAnswerLiveHub {
    pub(super) fn new(terminal_capacity: NonZeroUsize) -> Self {
        Self {
            terminal_capacity,
            runs: DashMap::new(),
            terminal_order: Mutex::new(VecDeque::new()),
        }
    }

    pub(super) fn create(&self, run_id: ExplainabilityRunId) -> bool {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
        match self.runs.entry(run_id) {
            Entry::Occupied(_) => false,
            Entry::Vacant(entry) => {
                entry.insert(LiveRun {
                    state: LiveAnswerState::default(),
                    sender,
                });
                true
            }
        }
    }

    pub(super) fn append(&self, run_id: &ExplainabilityRunId, delta: String) -> bool {
        let Some(mut run) = self.runs.get_mut(run_id) else {
            return false;
        };
        if run.state.status.is_terminal() {
            return false;
        }
        let Some(sequence) = run.state.sequence.checked_add(1) else {
            return false;
        };
        run.state.sequence = sequence;
        run.state.text.push_str(&delta);
        let event = QueryAnswerEvent::Delta {
            sequence: run.state.sequence,
            delta,
        };
        let _receiver_count = run.sender.send(event);
        true
    }

    pub(super) fn complete(&self, run_id: &ExplainabilityRunId, canonical_text: String) -> bool {
        self.terminate(run_id, LiveAnswerStatus::Completed, Some(canonical_text))
    }

    pub(super) fn fail(&self, run_id: &ExplainabilityRunId) -> bool {
        self.terminate(run_id, LiveAnswerStatus::Failed, None)
    }

    fn terminate(
        &self,
        run_id: &ExplainabilityRunId,
        status: LiveAnswerStatus,
        canonical_text: Option<String>,
    ) -> bool {
        let Some(mut run) = self.runs.get_mut(run_id) else {
            return false;
        };
        if run.state.status.is_terminal() {
            return false;
        }
        let Some(sequence) = run.state.sequence.checked_add(1) else {
            return false;
        };
        run.state.sequence = sequence;
        if let Some(text) = canonical_text {
            run.state.text = text;
        }
        run.state.status = status;
        let event = match status {
            LiveAnswerStatus::Completed => QueryAnswerEvent::Completed {
                sequence: run.state.sequence,
            },
            LiveAnswerStatus::Failed => QueryAnswerEvent::Failed {
                sequence: run.state.sequence,
            },
            LiveAnswerStatus::Running => return false,
        };
        let _receiver_count = run.sender.send(event);
        drop(run);
        self.retain_terminal(run_id.clone());
        true
    }

    fn retain_terminal(&self, run_id: ExplainabilityRunId) {
        let Ok(mut order) = self.terminal_order.lock() else {
            self.runs.remove(&run_id);
            return;
        };
        order.retain(|existing| existing != &run_id);
        order.push_back(run_id);
        while order.len() > self.terminal_capacity.get() {
            let Some(oldest) = order.pop_front() else {
                break;
            };
            self.runs.remove(&oldest);
        }
    }

    /// Subscribe before capturing state, preventing a mutation between receiver creation and the
    /// snapshot. Broadcast events at or below the snapshot sequence are safe to discard.
    pub(super) fn subscribe(
        self: &Arc<Self>,
        run_id: &ExplainabilityRunId,
    ) -> Option<QueryAnswerSubscription> {
        let run = self.runs.get(run_id)?;
        let receiver = run.sender.subscribe();
        let snapshot = snapshot_event(&run.state);
        drop(run);
        Some(QueryAnswerSubscription {
            hub: Arc::clone(self),
            run_id: run_id.clone(),
            snapshot,
            receiver,
        })
    }

    fn snapshot(&self, run_id: &ExplainabilityRunId) -> Option<QueryAnswerEvent> {
        self.runs.get(run_id).map(|run| snapshot_event(&run.state))
    }
}

fn snapshot_event(state: &LiveAnswerState) -> QueryAnswerEvent {
    QueryAnswerEvent::Snapshot {
        sequence: state.sequence,
        text: state.text.clone(),
        status: state.status,
    }
}

/// Race-free receiver which can recover from broadcast lag using the hub snapshot.
pub(super) struct QueryAnswerSubscription {
    hub: Arc<QueryAnswerLiveHub>,
    run_id: ExplainabilityRunId,
    snapshot: QueryAnswerEvent,
    receiver: broadcast::Receiver<QueryAnswerEvent>,
}

impl fmt::Debug for QueryAnswerSubscription {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("QueryAnswerSubscription { .. }")
    }
}

impl QueryAnswerSubscription {
    pub(super) fn initial_snapshot(&self) -> QueryAnswerEvent {
        self.snapshot.clone()
    }

    pub(super) async fn recv(&mut self) -> Result<QueryAnswerEvent, QueryAnswerRecvError> {
        loop {
            match self.receiver.recv().await {
                Ok(event) if event.sequence() > self.snapshot.sequence() => {
                    self.snapshot = event.clone();
                    return Ok(event);
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let Some(snapshot) = self.hub.snapshot(&self.run_id) else {
                        return Err(QueryAnswerRecvError::Closed);
                    };
                    self.snapshot = snapshot.clone();
                    return Ok(snapshot);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(QueryAnswerRecvError::Closed);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QueryAnswerRecvError {
    Closed,
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, sync::Arc};

    use tokio::sync::Barrier;

    use super::{CHANNEL_CAPACITY, LiveAnswerStatus, QueryAnswerEvent, QueryAnswerLiveHub};

    fn run_id(value: &str) -> graphloom::explainability::ExplainabilityRunId {
        value.parse().expect("test run id")
    }

    #[tokio::test]
    async fn test_should_snapshot_and_sequence_answer_mutations() {
        let hub = Arc::new(QueryAnswerLiveHub::new(NonZeroUsize::MIN));
        let id = run_id("answer-sequence");
        assert!(hub.create(id.clone()));
        let mut before = hub.subscribe(&id).expect("subscription");
        assert_eq!(before.initial_snapshot().sequence(), 0);
        assert!(hub.append(&id, "王婆".to_owned()));
        assert!(hub.append(&id, "首先".to_owned()));
        assert_eq!(before.recv().await.expect("first").sequence(), 1);
        assert_eq!(before.recv().await.expect("second").sequence(), 2);

        let mut late = hub.subscribe(&id).expect("late subscription");
        assert_eq!(
            late.initial_snapshot(),
            QueryAnswerEvent::Snapshot {
                sequence: 2,
                text: "王婆首先".to_owned(),
                status: LiveAnswerStatus::Running,
            }
        );
        assert!(hub.append(&id, "继续".to_owned()));
        assert_eq!(
            late.recv().await.expect("post-reconnect delta"),
            QueryAnswerEvent::Delta {
                sequence: 3,
                delta: "继续".to_owned(),
            }
        );
        assert!(hub.complete(&id, "王婆首先继续".to_owned()));
        assert_eq!(
            hub.subscribe(&id).expect("terminal").initial_snapshot(),
            QueryAnswerEvent::Snapshot {
                sequence: 4,
                text: "王婆首先继续".to_owned(),
                status: LiveAnswerStatus::Completed,
            }
        );
    }

    #[tokio::test]
    async fn test_should_recover_lag_with_authoritative_snapshot() {
        let hub = Arc::new(QueryAnswerLiveHub::new(NonZeroUsize::MIN));
        let id = run_id("answer-lag");
        assert!(hub.create(id.clone()));
        let mut subscription = hub.subscribe(&id).expect("subscription");
        for _ in 0..=CHANNEL_CAPACITY {
            assert!(hub.append(&id, "x".to_owned()));
        }
        assert_eq!(
            subscription.recv().await.expect("snapshot"),
            QueryAnswerEvent::Snapshot {
                sequence: 65,
                text: "x".repeat(65),
                status: LiveAnswerStatus::Running,
            }
        );
    }

    #[tokio::test]
    async fn test_should_not_lose_mutation_racing_with_snapshot_subscription() {
        let hub = Arc::new(QueryAnswerLiveHub::new(
            NonZeroUsize::new(128).expect("non-zero retention"),
        ));
        for index in 0..100 {
            let id = run_id(&format!("answer-race-{index}"));
            assert!(hub.create(id.clone()));
            let barrier = Arc::new(Barrier::new(2));
            let writer_hub = Arc::clone(&hub);
            let writer_id = id.clone();
            let writer_barrier = Arc::clone(&barrier);
            let writer = tokio::spawn(async move {
                writer_barrier.wait().await;
                writer_hub.append(&writer_id, "A".to_owned())
            });
            barrier.wait().await;
            let mut subscription = hub.subscribe(&id).expect("racing subscription");
            let snapshot = subscription.initial_snapshot();
            assert!(writer.await.expect("writer task"));
            match snapshot {
                QueryAnswerEvent::Snapshot {
                    sequence: 0,
                    text,
                    status: LiveAnswerStatus::Running,
                } => {
                    assert!(text.is_empty());
                    assert_eq!(
                        subscription.recv().await.expect("racing delta"),
                        QueryAnswerEvent::Delta {
                            sequence: 1,
                            delta: "A".to_owned(),
                        }
                    );
                }
                QueryAnswerEvent::Snapshot {
                    sequence: 1,
                    text,
                    status: LiveAnswerStatus::Running,
                } => assert_eq!(text, "A"),
                event => panic!("unexpected racing snapshot: {event:?}"),
            }
        }
    }

    #[test]
    fn test_should_isolate_runs_fail_safely_and_evict_old_terminals() {
        let hub = QueryAnswerLiveHub::new(NonZeroUsize::MIN);
        let first = run_id("answer-first");
        let second = run_id("answer-second");
        assert!(hub.create(first.clone()));
        assert!(hub.create(second.clone()));
        assert!(hub.append(&first, "A".to_owned()));
        assert!(hub.append(&second, "B".to_owned()));
        assert!(hub.fail(&first));
        assert!(hub.fail(&second));
        assert!(Arc::new(hub).subscribe(&first).is_none());
    }

    #[test]
    fn test_should_reconcile_stream_mismatch_only_in_terminal_snapshot() {
        let hub = Arc::new(QueryAnswerLiveHub::new(NonZeroUsize::MIN));
        let id = run_id("answer-mismatch");
        assert!(hub.create(id.clone()));
        assert!(hub.append(&id, "A".to_owned()));
        assert!(hub.complete(&id, "AB".to_owned()));
        assert_eq!(
            hub.subscribe(&id).expect("terminal").initial_snapshot(),
            QueryAnswerEvent::Snapshot {
                sequence: 2,
                text: "AB".to_owned(),
                status: LiveAnswerStatus::Completed,
            }
        );
    }
}
