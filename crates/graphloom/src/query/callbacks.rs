//! Query callback contracts.

use std::sync::Arc;

use super::{MapSearchResult, QueryContext};

/// Callbacks invoked during Query orchestration.
///
/// Implementations must not panic. A callback panic follows Rust's normal panic
/// semantics and cannot be converted into a provider-neutral Query error.
pub trait QueryCallbacks: Send + Sync + std::fmt::Debug {
    /// Context construction completed.
    fn on_context(&self, _context: &QueryContext) {}
    /// Global map calls are about to start.
    fn on_map_response_start(&self, _contexts: &[String]) {}
    /// Global map calls completed.
    fn on_map_response_end(&self, _outputs: &[MapSearchResult]) {}
    /// A reduce completion is about to start.
    fn on_reduce_response_start(&self, _context: &str) {}
    /// A reduce completion completed.
    fn on_reduce_response_end(&self, _output: &str) {}
    /// One provider text delta arrived.
    fn on_llm_new_token(&self, _token: &str) {}
}

/// Callback implementation that performs no work.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct NoopQueryCallbacks;

impl QueryCallbacks for NoopQueryCallbacks {}

/// Ordered callback fan-out.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct QueryCallbackChain {
    callbacks: Vec<Arc<dyn QueryCallbacks>>,
}

impl QueryCallbackChain {
    /// Create a chain preserving callback order.
    #[must_use]
    pub fn new(callbacks: Vec<Arc<dyn QueryCallbacks>>) -> Self {
        Self { callbacks }
    }
}

impl QueryCallbacks for QueryCallbackChain {
    fn on_context(&self, context: &QueryContext) {
        for callback in &self.callbacks {
            callback.on_context(context);
        }
    }

    fn on_map_response_start(&self, contexts: &[String]) {
        for callback in &self.callbacks {
            callback.on_map_response_start(contexts);
        }
    }

    fn on_map_response_end(&self, outputs: &[MapSearchResult]) {
        for callback in &self.callbacks {
            callback.on_map_response_end(outputs);
        }
    }

    fn on_reduce_response_start(&self, context: &str) {
        for callback in &self.callbacks {
            callback.on_reduce_response_start(context);
        }
    }

    fn on_reduce_response_end(&self, output: &str) {
        for callback in &self.callbacks {
            callback.on_reduce_response_end(output);
        }
    }

    fn on_llm_new_token(&self, token: &str) {
        for callback in &self.callbacks {
            callback.on_llm_new_token(token);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::{QueryCallbackChain, QueryCallbacks};
    use crate::query::{MapSearchResult, QueryUsageCategory};

    #[derive(Debug)]
    struct RecordingCallback {
        name: &'static str,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl QueryCallbacks for RecordingCallback {
        fn on_map_response_start(&self, _contexts: &[String]) {
            self.events
                .lock()
                .expect("events")
                .push(format!("{}:map_start", self.name));
        }

        fn on_map_response_end(&self, _outputs: &[MapSearchResult]) {
            self.events
                .lock()
                .expect("events")
                .push(format!("{}:map_end", self.name));
        }
    }

    #[test]
    fn test_should_fan_out_global_callbacks_in_chain_order() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let chain = QueryCallbackChain::new(vec![
            Arc::new(RecordingCallback {
                name: "first",
                events: Arc::clone(&events),
            }),
            Arc::new(RecordingCallback {
                name: "second",
                events: Arc::clone(&events),
            }),
        ]);
        let output = MapSearchResult {
            batch_index: 0,
            raw_response: String::new(),
            points: Vec::new(),
            context: String::new(),
            usage: QueryUsageCategory::default(),
        };
        chain.on_map_response_start(&["context".to_owned()]);
        chain.on_map_response_end(&[output]);
        assert_eq!(
            *events.lock().expect("events"),
            [
                "first:map_start",
                "second:map_start",
                "first:map_end",
                "second:map_end",
            ]
        );
    }
}
