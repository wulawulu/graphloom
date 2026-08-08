//! Bounded, best-effort realtime fan-out for persisted explainability envelopes.
//!
//! One broadcast channel is allocated per live-active run. The persistence writer registers a
//! run only after Store creation succeeds, publishes the same envelope only after durable append
//! and sequence commit, and closes the channel at the recorder lifecycle boundary. Historical
//! recovery remains the responsibility of [`ExplainabilityStore`](super::ExplainabilityStore).

use std::{
    fmt,
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use dashmap::DashMap;
use thiserror::Error;
use tokio::sync::broadcast;

use super::{ExplainabilityEnvelope, ExplainabilityRunId};

const DEFAULT_CHANNEL_CAPACITY: usize = 256;

/// Configuration for an [`ExplainabilityLiveHub`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ExplainabilityLiveHubOptions {
    channel_capacity: NonZeroUsize,
}

impl ExplainabilityLiveHubOptions {
    /// Create options with a realtime broadcast ring capacity of 256 envelopes per active run.
    #[must_use]
    pub fn new() -> Self {
        Self {
            channel_capacity: NonZeroUsize::new(DEFAULT_CHANNEL_CAPACITY)
                .unwrap_or(NonZeroUsize::MIN),
        }
    }

    /// Override the bounded realtime broadcast ring capacity for each active run.
    #[must_use]
    pub const fn with_channel_capacity(mut self, channel_capacity: NonZeroUsize) -> Self {
        self.channel_capacity = channel_capacity;
        self
    }

    /// Return the realtime broadcast ring capacity allocated to each active run.
    #[must_use]
    pub const fn channel_capacity(&self) -> NonZeroUsize {
        self.channel_capacity
    }
}

impl Default for ExplainabilityLiveHubOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Best-effort realtime fan-out of successfully persisted explainability envelopes.
///
/// The Hub performs no I/O, owns no history or Store, allocates no sequence, and starts no task.
/// Its map contains only currently live-active runs. Slow subscribers receive
/// [`ExplainabilityLiveRecvError::Lagged`] without applying backpressure to persistence.
#[non_exhaustive]
pub struct ExplainabilityLiveHub {
    options: ExplainabilityLiveHubOptions,
    runs: DashMap<ExplainabilityRunId, Arc<LiveRunChannel>>,
}

impl fmt::Debug for ExplainabilityLiveHub {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExplainabilityLiveHub { .. }")
    }
}

impl ExplainabilityLiveHub {
    /// Create an empty Hub without requiring a Tokio runtime or starting background work.
    #[must_use]
    pub fn new(options: ExplainabilityLiveHubOptions) -> Self {
        Self {
            options,
            runs: DashMap::new(),
        }
    }

    /// Subscribe to a currently live-active run.
    ///
    /// Returns `None` for an unknown or closed run. This method never creates or reopens a run.
    /// The receiver is registered before the committed sequence snapshot is read, closing the
    /// subscribe-versus-replay race. The snapshot is a Store catch-up boundary, not a delivery
    /// acknowledgement: envelopes committed without a receiver may have been discarded live.
    #[must_use]
    pub fn subscribe(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Option<ExplainabilityLiveSubscription> {
        let channel = self.runs.get(run_id)?;
        let receiver = channel.sender.subscribe();
        // Acquire pairs with publish's Release store. Registering the receiver first guarantees
        // that a concurrent post-snapshot publish is observable through this receiver.
        let snapshot_sequence = channel.last_sequence.load(Ordering::Acquire);
        Some(ExplainabilityLiveSubscription {
            run_id: run_id.clone(),
            snapshot_sequence,
            receiver,
        })
    }

    pub(crate) fn register_run(&self, run_id: ExplainabilityRunId) {
        self.runs.entry(run_id).or_insert_with(|| {
            let (sender, _) = broadcast::channel(self.options.channel_capacity().get());
            Arc::new(LiveRunChannel {
                sender,
                last_sequence: AtomicU64::new(0),
            })
        });
    }

    pub(crate) fn publish(&self, envelope: Arc<ExplainabilityEnvelope>) {
        let Some(channel) = self.runs.get(&envelope.record.run_id) else {
            return;
        };
        // The Store and writer already validated and committed the sequence. This value is only
        // the replay boundary, so live delivery success must not control whether it advances.
        channel
            .last_sequence
            .store(envelope.sequence(), Ordering::Release);
        let _ = channel.sender.send(envelope);
    }

    pub(crate) fn close_run(&self, run_id: &ExplainabilityRunId) {
        self.runs.remove(run_id);
    }
}

#[derive(Debug)]
struct LiveRunChannel {
    sender: broadcast::Sender<Arc<ExplainabilityEnvelope>>,
    last_sequence: AtomicU64,
}

/// Receiver for one live-active run's persisted envelopes.
///
/// Debug output is deliberately opaque so run identity and event payloads cannot leak.
pub struct ExplainabilityLiveSubscription {
    run_id: ExplainabilityRunId,
    snapshot_sequence: u64,
    receiver: broadcast::Receiver<Arc<ExplainabilityEnvelope>>,
}

impl fmt::Debug for ExplainabilityLiveSubscription {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExplainabilityLiveSubscription { .. }")
    }
}

impl ExplainabilityLiveSubscription {
    /// Return the run observed by this subscription.
    #[must_use]
    pub const fn run_id(&self) -> &ExplainabilityRunId {
        &self.run_id
    }

    /// Return the latest sequence committed before or during subscription setup.
    ///
    /// This Store catch-up boundary does not assert that any subscriber received those envelopes
    /// live. Sequence zero means the run is active but has no committed envelope yet.
    #[must_use]
    pub const fn snapshot_sequence(&self) -> u64 {
        self.snapshot_sequence
    }

    /// Receive the next persisted envelope retained for this subscriber.
    ///
    /// # Errors
    ///
    /// Returns [`ExplainabilityLiveRecvError::Lagged`] when this subscriber falls behind its
    /// per-run ring buffer, or [`ExplainabilityLiveRecvError::Closed`] after the buffered prefix
    /// is drained and the run's sender has been removed.
    pub async fn recv(
        &mut self,
    ) -> Result<Arc<ExplainabilityEnvelope>, ExplainabilityLiveRecvError> {
        self.receiver.recv().await.map_err(Into::into)
    }
}

/// Recoverable receive state for an explainability live subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ExplainabilityLiveRecvError {
    /// The subscriber fell behind the bounded per-run broadcast ring.
    #[error("the explainability live subscriber lagged behind")]
    Lagged {
        /// Number of envelopes skipped by this receiver.
        skipped: u64,
    },
    /// The current realtime run channel ended.
    #[error("the explainability live run is closed")]
    Closed,
}

impl From<broadcast::error::RecvError> for ExplainabilityLiveRecvError {
    fn from(error: broadcast::error::RecvError) -> Self {
        match error {
            broadcast::error::RecvError::Lagged(skipped) => Self::Lagged { skipped },
            broadcast::error::RecvError::Closed => Self::Closed,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, str::FromStr, sync::Arc};

    use chrono::Utc;

    use super::{
        DEFAULT_CHANNEL_CAPACITY, ExplainabilityLiveHub, ExplainabilityLiveHubOptions,
        ExplainabilityLiveRecvError,
    };
    use crate::explainability::{
        ExplainabilityEnvelope, ExplainabilityEvent, ExplainabilityQueryMethod,
        ExplainabilityRecord, ExplainabilityRunId, ExplainabilitySpanId, QueryStarted,
    };

    fn run_id(value: &str) -> ExplainabilityRunId {
        value.parse().expect("run id")
    }

    fn envelope(run_id: &ExplainabilityRunId, sequence: u64) -> Arc<ExplainabilityEnvelope> {
        Arc::new(
            ExplainabilityEnvelope::new(
                sequence,
                ExplainabilityRecord::new(
                    run_id.clone(),
                    Utc::now(),
                    ExplainabilitySpanId::from_str("live-span").expect("span id"),
                    None,
                    ExplainabilityEvent::QueryStarted(QueryStarted::new(
                        ExplainabilityQueryMethod::Local,
                    )),
                ),
            )
            .expect("envelope"),
        )
    }

    fn hub_with_capacity(capacity: usize) -> ExplainabilityLiveHub {
        ExplainabilityLiveHub::new(
            ExplainabilityLiveHubOptions::new().with_channel_capacity(
                NonZeroUsize::new(capacity).expect("non-zero fixture capacity"),
            ),
        )
    }

    #[test]
    fn test_should_default_channel_capacity_to_256_and_construct_without_runtime() {
        let options = ExplainabilityLiveHubOptions::new();
        assert_eq!(options.channel_capacity().get(), DEFAULT_CHANNEL_CAPACITY);
        assert_eq!(
            ExplainabilityLiveHubOptions::default()
                .channel_capacity()
                .get(),
            DEFAULT_CHANNEL_CAPACITY
        );
        assert_eq!(
            format!("{:?}", ExplainabilityLiveHub::new(options)),
            "ExplainabilityLiveHub { .. }"
        );
    }

    #[test]
    fn test_should_not_subscribe_or_create_an_unknown_run() {
        let hub = ExplainabilityLiveHub::new(ExplainabilityLiveHubOptions::new());
        let id = run_id("unknown");
        assert!(hub.subscribe(&id).is_none());
        assert!(hub.subscribe(&id).is_none());
    }

    #[test]
    fn test_should_register_idempotently_and_snapshot_zero_before_events() {
        let hub = ExplainabilityLiveHub::new(ExplainabilityLiveHubOptions::new());
        let id = run_id("registered");
        hub.register_run(id.clone());
        let subscription = hub.subscribe(&id).expect("active run");
        hub.register_run(id.clone());
        assert_eq!(subscription.run_id(), &id);
        assert_eq!(subscription.snapshot_sequence(), 0);
    }

    #[tokio::test]
    async fn test_should_publish_same_arc_and_advance_snapshot() {
        let hub = ExplainabilityLiveHub::new(ExplainabilityLiveHubOptions::new());
        let id = run_id("fan-out");
        hub.register_run(id.clone());
        let mut first = hub.subscribe(&id).expect("first");
        let mut second = hub.subscribe(&id).expect("second");
        let published = envelope(&id, 1);
        hub.publish(Arc::clone(&published));
        let first_event = first.recv().await.expect("first event");
        let second_event = second.recv().await.expect("second event");
        assert!(Arc::ptr_eq(&published, &first_event));
        assert!(Arc::ptr_eq(&first_event, &second_event));
        assert_eq!(hub.subscribe(&id).expect("later").snapshot_sequence(), 1);
    }

    #[test]
    fn test_should_advance_snapshot_without_subscribers_without_replay() {
        let hub = ExplainabilityLiveHub::new(ExplainabilityLiveHubOptions::new());
        let id = run_id("no-subscriber");
        hub.register_run(id.clone());
        hub.publish(envelope(&id, 1));
        let mut subscription = hub.subscribe(&id).expect("active");
        assert_eq!(subscription.snapshot_sequence(), 1);
        assert!(subscription.receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_should_report_lag_without_affecting_future_publish()
    -> Result<(), ExplainabilityLiveRecvError> {
        let hub = hub_with_capacity(1);
        let id = run_id("lag");
        hub.register_run(id.clone());
        let mut subscription = hub.subscribe(&id).expect("active");
        for sequence in 1..=3 {
            hub.publish(envelope(&id, sequence));
        }
        assert!(matches!(
            subscription.recv().await,
            Err(ExplainabilityLiveRecvError::Lagged { skipped: 2 })
        ));
        assert_eq!(subscription.recv().await?.sequence(), 3);
        hub.publish(envelope(&id, 4));
        assert_eq!(subscription.recv().await?.sequence(), 4);
        Ok(())
    }

    #[tokio::test]
    async fn test_should_isolate_run_channels_and_close_after_buffer_drain()
    -> Result<(), ExplainabilityLiveRecvError> {
        let hub = hub_with_capacity(1);
        let run_a = run_id("run-a");
        let run_b = run_id("run-b");
        hub.register_run(run_a.clone());
        hub.register_run(run_b.clone());
        let mut run_b_subscription = hub.subscribe(&run_b).expect("run b");
        for sequence in 1..=10 {
            hub.publish(envelope(&run_a, sequence));
        }
        hub.publish(envelope(&run_b, 1));
        hub.close_run(&run_b);
        assert_eq!(run_b_subscription.recv().await?.sequence(), 1);
        assert_eq!(
            run_b_subscription.recv().await,
            Err(ExplainabilityLiveRecvError::Closed)
        );
        assert!(hub.subscribe(&run_b).is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_should_keep_errors_and_debug_free_of_payloads() {
        const QUERY_SECRET: &str = "LIVE_QUERY_SECRET_SENTINEL";
        const EVENT_SECRET: &str = "LIVE_EVENT_SECRET_SENTINEL";

        let hub = hub_with_capacity(1);
        let id = run_id("safe-debug");
        hub.register_run(id.clone());
        let mut subscription = hub.subscribe(&id).expect("active");
        let mut record = envelope(&id, 1).record.clone();
        if let ExplainabilityEvent::QueryStarted(event) = &mut record.event {
            event.query = Some(format!("{QUERY_SECRET}-{EVENT_SECRET}"));
        }
        hub.publish(Arc::new(
            ExplainabilityEnvelope::new(1, record).expect("secret envelope"),
        ));
        hub.publish(envelope(&id, 2));
        let error = subscription.recv().await.expect_err("must lag");
        let output = format!("{error:?} {error} {subscription:?} {hub:?}");
        assert!(!output.contains(QUERY_SECRET));
        assert!(!output.contains(EVENT_SECRET));
    }
}
