//! Synchronous, object-safe explainability event consumers.

use std::sync::Arc;

use super::ExplainabilityRecord;

/// Fast synchronous consumer of explainability business records.
///
/// Implementations must return quickly, must not perform blocking I/O on the calling thread, and
/// must not panic. Persistence and network adapters should enqueue the borrowed record into a
/// bounded channel for a dedicated writer. Adapter errors and flush lifecycles are intentionally
/// outside this foundational contract.
pub trait ExplainabilitySink: Send + Sync + std::fmt::Debug {
    /// Observe one immutable business record.
    fn emit(&self, record: &ExplainabilityRecord);
}

/// Reusable sink that performs no work.
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

impl ExplainabilitySink for NoopExplainabilitySink {
    fn emit(&self, _record: &ExplainabilityRecord) {}
}

/// Ordered fan-out across zero or more explainability sinks.
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
}

impl ExplainabilitySink for ExplainabilitySinkChain {
    fn emit(&self, record: &ExplainabilityRecord) {
        for sink in &self.sinks {
            sink.emit(record);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        str::FromStr,
        sync::{Arc, Mutex, MutexGuard},
    };

    use chrono::Utc;

    use super::{ExplainabilitySink, ExplainabilitySinkChain, NoopExplainabilitySink};
    use crate::explainability::{
        ExplainabilityContractError, ExplainabilityEvent, ExplainabilityQueryMethod,
        ExplainabilityRecord, ExplainabilityRunId, ExplainabilitySpanId, QueryStarted,
    };

    #[derive(Debug)]
    struct RecordingSink {
        name: &'static str,
        calls: Arc<Mutex<Vec<(&'static str, ExplainabilityRecord)>>>,
    }

    impl ExplainabilitySink for RecordingSink {
        fn emit(&self, record: &ExplainabilityRecord) {
            lock_recovering_poison(&self.calls).push((self.name, record.clone()));
        }
    }

    fn lock_recovering_poison<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        match mutex.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn sample_record() -> Result<ExplainabilityRecord, ExplainabilityContractError> {
        Ok(ExplainabilityRecord::new(
            ExplainabilityRunId::from_str("run-1")?,
            Utc::now(),
            ExplainabilitySpanId::from_str("span-1")?,
            None,
            ExplainabilityEvent::QueryStarted(QueryStarted::new(ExplainabilityQueryMethod::Local)),
        ))
    }

    #[test]
    fn test_should_treat_noop_and_empty_chain_as_zero_side_effects()
    -> Result<(), ExplainabilityContractError> {
        let record = sample_record()?;
        NoopExplainabilitySink::new().emit(&record);
        let chain = ExplainabilitySinkChain::default();
        assert!(chain.is_empty());
        chain.emit(&record);
        assert_eq!(record.run_id.as_str(), "run-1");
        Ok(())
    }

    #[test]
    fn test_should_fan_out_in_registration_order_without_mutating_record()
    -> Result<(), ExplainabilityContractError> {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let first: Arc<dyn ExplainabilitySink> = Arc::new(RecordingSink {
            name: "first",
            calls: Arc::clone(&calls),
        });
        let second: Arc<dyn ExplainabilitySink> = Arc::new(RecordingSink {
            name: "second",
            calls: Arc::clone(&calls),
        });
        let original = sample_record()?;
        let chain = ExplainabilitySinkChain::new(vec![first, second]);
        assert_eq!(chain.len(), 2);
        chain.emit(&original);

        let observed = lock_recovering_poison(&calls);
        assert_eq!(observed.len(), 2);
        assert_eq!(observed.first().map(|call| call.0), Some("first"));
        assert_eq!(observed.get(1).map(|call| call.0), Some("second"));
        assert!(observed.iter().all(|(_, record)| record == &original));
        Ok(())
    }

    #[test]
    fn test_should_support_single_sink_chain() -> Result<(), ExplainabilityContractError> {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let sink: Arc<dyn ExplainabilitySink> = Arc::new(RecordingSink {
            name: "only",
            calls: Arc::clone(&calls),
        });
        let chain = ExplainabilitySinkChain::new(vec![sink]);
        chain.emit(&sample_record()?);
        assert_eq!(lock_recovering_poison(&calls).len(), 1);
        Ok(())
    }
}
