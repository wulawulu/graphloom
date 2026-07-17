//! Query callback contracts.

use std::sync::Arc;

use super::QueryContext;

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
    fn on_map_response_end(&self, _outputs: &[String]) {}
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

    fn on_map_response_end(&self, outputs: &[String]) {
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
