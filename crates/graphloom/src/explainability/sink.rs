//! Reliable, asynchronous, object-safe explainability event consumers.
//!
//! The sink trait uses [`macro@async_trait`] because callers store implementations behind
//! `Arc<dyn ExplainabilitySink>` and therefore require object-safe async dispatch.

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use thiserror::Error;

use super::{ExplainabilityRecord, ExplainabilityRunId};

/// Sink operation being performed when an aggregate failure occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExplainabilitySinkOperation {
    /// Reliable acceptance of one record.
    Emit,
    /// Completion confirmation for one run.
    FinishRun,
}

impl fmt::Display for ExplainabilitySinkOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Emit => "record delivery",
            Self::FinishRun => "run finalization",
        })
    }
}

/// One indexed failure reported by an [`ExplainabilitySinkChain`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExplainabilitySinkFailure {
    sink_index: usize,
    error: ExplainabilitySinkError,
}

impl ExplainabilitySinkFailure {
    fn new(sink_index: usize, error: ExplainabilitySinkError) -> Self {
        Self { sink_index, error }
    }

    /// Return the zero-based registration index of the failed sink.
    #[must_use]
    pub const fn sink_index(&self) -> usize {
        self.sink_index
    }

    /// Return the error reported by the failed sink.
    #[must_use]
    pub const fn error(&self) -> &ExplainabilitySinkError {
        &self.error
    }
}

/// Safe, structured failure from reliable explainability delivery.
///
/// Variants deliberately carry no provider request data, credentials, or arbitrary diagnostic
/// strings, so their messages are safe to display. Adapters should map internal errors to the
/// narrowest stable category and retain sensitive diagnostics only in redacted operational logs.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ExplainabilitySinkError {
    /// A record could not be reliably accepted by the adapter.
    #[error("the explainability record was not accepted")]
    RecordNotAccepted,
    /// The sink or its bounded input queue has closed.
    #[error("the explainability sink is closed")]
    Closed,
    /// The sink is temporarily or permanently unavailable.
    #[error("the explainability sink is unavailable")]
    Unavailable,
    /// A persistence writer failed while processing accepted records.
    #[error("the explainability persistence writer failed")]
    WriterFailed,
    /// The sink could not confirm all required processing for a finished run.
    #[error("the explainability run could not be finalized")]
    RunFinalizationFailed,
    /// One or more sinks in an ordered chain failed.
    #[error("one or more explainability sinks failed during {operation}")]
    Chain {
        /// Operation attempted on every registered sink.
        operation: ExplainabilitySinkOperation,
        /// Failures in registration order, each with its stable sink index.
        failures: Vec<ExplainabilitySinkFailure>,
    },
}

/// Reliable consumer of immutable explainability business records.
///
/// Implementations may asynchronously wait for capacity in a bounded adapter queue, but must not
/// perform blocking file, database, or network I/O on the Tokio worker. A successful [`Self::emit`]
/// means the adapter has reliably accepted the exact record; it must never mean that a full queue
/// silently dropped the record. Implementations must return explicit errors and must not panic.
///
/// The trait uses [`macro@async_trait`] to remain object-safe for
/// `Arc<dyn ExplainabilitySink>` dynamic
/// dispatch.
#[async_trait]
pub trait ExplainabilitySink: Send + Sync + fmt::Debug {
    /// Reliably accept one immutable business record, applying asynchronous backpressure as needed.
    ///
    /// Success confirms adapter acceptance, not necessarily completion of persistence I/O.
    ///
    /// # Errors
    ///
    /// Returns an [`ExplainabilitySinkError`] if the record cannot be accepted without loss.
    async fn emit(&self, record: Arc<ExplainabilityRecord>) -> Result<(), ExplainabilitySinkError>;

    /// Confirm that all accepted records for `run_id` completed required processing.
    ///
    /// Calling this method declares that the caller will emit no more records for the run. It
    /// manages only delivery and persistence lifecycle: it neither changes run status nor creates
    /// a `RunCompleted` event.
    ///
    /// # Errors
    ///
    /// Returns an [`ExplainabilitySinkError`] if writer processing, persistence, or final flush
    /// confirmation fails.
    async fn finish_run(&self, run_id: &ExplainabilityRunId)
    -> Result<(), ExplainabilitySinkError>;
}

/// Reusable sink that performs no work and always confirms success.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct NoopExplainabilitySink;

impl NoopExplainabilitySink {
    /// Create a no-op explainability sink.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ExplainabilitySink for NoopExplainabilitySink {
    async fn emit(
        &self,
        _record: Arc<ExplainabilityRecord>,
    ) -> Result<(), ExplainabilitySinkError> {
        Ok(())
    }

    async fn finish_run(
        &self,
        _run_id: &ExplainabilityRunId,
    ) -> Result<(), ExplainabilitySinkError> {
        Ok(())
    }
}

/// Ordered, reliable fan-out across zero or more explainability sinks.
///
/// Every operation visits all sinks sequentially in registration order. Failures do not prevent
/// later sinks from receiving the record or finalization request; the returned aggregate retains
/// every failure and its zero-based sink index.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ExplainabilitySinkChain {
    sinks: Vec<Arc<dyn ExplainabilitySink>>,
}

impl ExplainabilitySinkChain {
    /// Create a chain preserving the supplied registration order.
    #[must_use]
    pub fn new(sinks: Vec<Arc<dyn ExplainabilitySink>>) -> Self {
        Self { sinks }
    }

    /// Return the number of registered sinks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sinks.len()
    }

    /// Return whether the chain has no registered sinks.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty()
    }

    fn aggregate(
        operation: ExplainabilitySinkOperation,
        failures: Vec<ExplainabilitySinkFailure>,
    ) -> Result<(), ExplainabilitySinkError> {
        if failures.is_empty() {
            Ok(())
        } else {
            Err(ExplainabilitySinkError::Chain {
                operation,
                failures,
            })
        }
    }
}

#[async_trait]
impl ExplainabilitySink for ExplainabilitySinkChain {
    async fn emit(&self, record: Arc<ExplainabilityRecord>) -> Result<(), ExplainabilitySinkError> {
        let mut failures = Vec::new();
        for (sink_index, sink) in self.sinks.iter().enumerate() {
            if let Err(error) = sink.emit(Arc::clone(&record)).await {
                failures.push(ExplainabilitySinkFailure::new(sink_index, error));
            }
        }
        Self::aggregate(ExplainabilitySinkOperation::Emit, failures)
    }

    async fn finish_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<(), ExplainabilitySinkError> {
        let mut failures = Vec::new();
        for (sink_index, sink) in self.sinks.iter().enumerate() {
            if let Err(error) = sink.finish_run(run_id).await {
                failures.push(ExplainabilitySinkFailure::new(sink_index, error));
            }
        }
        Self::aggregate(ExplainabilitySinkOperation::FinishRun, failures)
    }
}

#[cfg(test)]
mod tests {
    use std::{str::FromStr, sync::Arc};

    use chrono::Utc;

    use super::{ExplainabilitySink, ExplainabilitySinkChain, NoopExplainabilitySink};
    use crate::explainability::{
        ExplainabilityContractError, ExplainabilityEvent, ExplainabilityQueryMethod,
        ExplainabilityRecord, ExplainabilityRunId, ExplainabilitySpanId, QueryStarted,
    };

    fn sample_record() -> Result<Arc<ExplainabilityRecord>, ExplainabilityContractError> {
        Ok(Arc::new(ExplainabilityRecord::new(
            ExplainabilityRunId::from_str("run-1")?,
            Utc::now(),
            ExplainabilitySpanId::from_str("span-1")?,
            None,
            ExplainabilityEvent::QueryStarted(QueryStarted::new(ExplainabilityQueryMethod::Local)),
        )))
    }

    #[tokio::test]
    async fn test_should_treat_noop_and_empty_chain_as_success()
    -> Result<(), Box<dyn std::error::Error>> {
        let record = sample_record()?;
        let run_id = record.run_id.clone();
        let noop = NoopExplainabilitySink::new();
        noop.emit(Arc::clone(&record)).await?;
        noop.finish_run(&run_id).await?;

        let chain = ExplainabilitySinkChain::default();
        assert!(chain.is_empty());
        chain.emit(Arc::clone(&record)).await?;
        chain.finish_run(&run_id).await?;
        assert_eq!(record.run_id.as_str(), "run-1");
        Ok(())
    }
}
