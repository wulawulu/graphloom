//! Request-scoped Local Query tracing instrumentation.
//!
//! This module owns the internal `tracing` session for a Local Query request:
//! the root span lifecycle, stable error classification shared with
//! Explainability, and the request-scoped handle that keeps the tracing and
//! Explainability channels aligned without merging their data models.
//!
//! The module never initializes a subscriber and never performs business work.
//! When the root span callsite is disabled, no session state is allocated.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use tracing::{Instrument, Span};

use super::{
    QueryError, QueryOptions, QueryResult, Result, SearchMethod,
    explainability::QueryExplainabilitySession,
};
use crate::{
    GraphLoomError,
    observability::{
        OBSERVABILITY_CONTRACT_VERSION, error_kind, field_name, operation, span_name, status,
    },
};

/// Map a typed Query error to its stable, low-cardinality category.
///
/// Explainability `RunFailed` records and the `graphloom.error.kind` tracing
/// field share this single mapping.
pub(crate) const fn query_error_kind(error: &QueryError) -> &'static str {
    match error {
        QueryError::InvalidQueryConfig { .. } => error_kind::INVALID_QUERY_CONFIG,
        QueryError::MissingQueryTable { .. } => error_kind::MISSING_QUERY_TABLE,
        QueryError::InvalidQueryTable { .. } => error_kind::INVALID_QUERY_TABLE,
        QueryError::MissingVectorIndex { .. } => error_kind::MISSING_VECTOR_INDEX,
        QueryError::InvalidVectorIndex { .. } => error_kind::INVALID_VECTOR_INDEX,
        QueryError::QueryPrompt { .. } => error_kind::QUERY_PROMPT,
        QueryError::QueryEmbedding { .. } => error_kind::QUERY_EMBEDDING,
        QueryError::QueryCompletion { .. } => error_kind::QUERY_COMPLETION,
        QueryError::QueryParse { .. } => error_kind::QUERY_PARSE,
        QueryError::QueryContext { .. } => error_kind::QUERY_CONTEXT,
        QueryError::QueryRuntime { .. } => error_kind::QUERY_RUNTIME,
        QueryError::QueryMethod { .. } => error_kind::QUERY_METHOD,
    }
}

/// Map a request-level `GraphLoom` error to its stable category.
pub(crate) const fn graphloom_error_kind(error: &GraphLoomError) -> &'static str {
    match error {
        GraphLoomError::Query(error) => query_error_kind(error),
        GraphLoomError::InvalidRoot { .. } => error_kind::INVALID_QUERY_CONFIG,
        GraphLoomError::ExplainabilityOutput { .. } => error_kind::EXPLAINABILITY_OUTPUT,
        _ => error_kind::QUERY_RUNTIME,
    }
}

/// Convert a platform `usize` to the contract's `u64` wire type.
///
/// Returns `None` when the conversion cannot be represented; callers omit the
/// tracing field in that case instead of fabricating a value.
pub(crate) fn usize_to_u64(value: usize) -> Option<u64> {
    u64::try_from(value).ok()
}

/// Convert a duration to contract milliseconds as `u64`.
///
/// Returns `None` when the value cannot be represented; callers omit the
/// tracing field in that case.
pub(crate) fn duration_millis(duration: Duration) -> Option<u64> {
    u64::try_from(duration.as_millis()).ok()
}

/// Record an optional `u64` field, omitting it when the value is unknown.
pub(crate) fn record_u64(span: &Span, field: &'static str, value: Option<u64>) {
    if let Some(value) = value {
        span.record(field, value);
    }
}

/// Record the stable error terminal state on a stage span.
pub(crate) fn record_stage_error(span: &Span, error_kind_value: &'static str) {
    span.record(field_name::STATUS, status::ERROR);
    span.record(field_name::ERROR_KIND, error_kind_value);
}

/// Run a fallible Local runtime assembly step inside the runtime span.
///
/// The span is a child of the request root span and records `ok` or the stable
/// error category before closing. When the tracing session is disabled, the
/// future runs without any span.
pub(crate) async fn with_runtime_span<T>(
    trace: Option<&QueryTraceSession>,
    future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    let Some(trace) = trace else {
        return future.await;
    };
    let span = tracing::info_span!(
        parent: trace.root_span(),
        span_name::QUERY_RUNTIME,
        "graphloom.observability.version" = OBSERVABILITY_CONTRACT_VERSION,
        "graphloom.operation" = operation::RUNTIME_LOAD,
        "graphloom.query.method" = "local",
        "graphloom.status" = tracing::field::Empty,
        "graphloom.error.kind" = tracing::field::Empty,
    );
    let record_span = span.clone();
    async {
        let outcome = future.await;
        match &outcome {
            Ok(_) => {
                record_span.record(field_name::STATUS, status::OK);
            }
            Err(error) => record_stage_error(&record_span, query_error_kind(error)),
        }
        outcome
    }
    .instrument(span)
    .await
}

/// One Local Query request's root `graphloom.query.local` tracing session.
///
/// The session owns the root span handle, records terminal state exactly once,
/// and finalizes an abandoned state synchronously when the request is dropped
/// early. It never depends on Explainability's asynchronous `finish_run()`.
#[derive(Debug)]
pub(crate) struct QueryTraceSession {
    span: Span,
    started: Instant,
    terminal: AtomicBool,
}

impl QueryTraceSession {
    /// Create the request root span and session.
    ///
    /// The span becomes a child of the host's current span. No session state is
    /// allocated when the span callsite is disabled.
    pub(crate) fn start(options: &QueryOptions, streaming: bool) -> Option<Arc<Self>> {
        let explainability_enabled = options.explainability.is_some();
        let span = tracing::info_span!(
            span_name::QUERY_LOCAL,
            "graphloom.observability.version" = OBSERVABILITY_CONTRACT_VERSION,
            "graphloom.run.id" = tracing::field::Empty,
            "graphloom.operation" = operation::QUERY,
            "graphloom.query.method" = "local",
            "graphloom.query.streaming" = streaming,
            "graphloom.explainability.enabled" = explainability_enabled,
            "graphloom.status" = tracing::field::Empty,
            "graphloom.error.kind" = tracing::field::Empty,
            "graphloom.input.tokens" = tracing::field::Empty,
            "graphloom.output.tokens" = tracing::field::Empty,
            "graphloom.context.tokens" = tracing::field::Empty,
            "graphloom.llm.calls" = tracing::field::Empty,
            "graphloom.elapsed_ms" = tracing::field::Empty,
        );
        if span.is_disabled() {
            return None;
        }
        let session = Arc::new(Self {
            span,
            started: Instant::now(),
            terminal: AtomicBool::new(false),
        });
        if let Some(run_id) = options
            .explainability
            .as_ref()
            .map(|explainability| explainability.run_id().as_str())
        {
            session.span.record(field_name::RUN_ID, run_id);
        }
        Some(session)
    }

    /// Borrow the root span, used as the explicit parent of top-level stages.
    pub(crate) const fn root_span(&self) -> &Span {
        &self.span
    }

    /// Record a successful terminal state from the real Query result.
    pub(crate) fn finish_ok(&self, result: &QueryResult, context_tokens: Option<u64>) {
        if !self.begin_terminal() {
            return;
        }
        self.span.record(field_name::STATUS, status::OK);
        record_u64(
            &self.span,
            field_name::INPUT_TOKENS,
            usize_to_u64(result.usage.prompt_tokens),
        );
        record_u64(
            &self.span,
            field_name::OUTPUT_TOKENS,
            usize_to_u64(result.usage.output_tokens),
        );
        record_u64(&self.span, field_name::CONTEXT_TOKENS, context_tokens);
        record_u64(
            &self.span,
            field_name::LLM_CALLS,
            usize_to_u64(result.usage.llm_calls),
        );
        record_u64(
            &self.span,
            field_name::ELAPSED_MS,
            duration_millis(result.elapsed),
        );
    }

    /// Record a failed terminal state with a stable error category.
    pub(crate) fn finish_error(&self, error_kind_value: &'static str) {
        if !self.begin_terminal() {
            return;
        }
        self.span.record(field_name::STATUS, status::ERROR);
        self.span.record(field_name::ERROR_KIND, error_kind_value);
        record_u64(
            &self.span,
            field_name::ELAPSED_MS,
            duration_millis(self.started.elapsed()),
        );
    }

    /// Record a failed terminal state from a typed Query error.
    pub(crate) fn finish_query_error(&self, error: &QueryError) {
        self.finish_error(query_error_kind(error));
    }

    /// Record a failed terminal state from a request-level `GraphLoom` error.
    pub(crate) fn finish_graphloom_error(&self, error: &GraphLoomError) {
        self.finish_error(graphloom_error_kind(error));
    }

    /// Record the stream-ended-without-Completed business error.
    pub(crate) fn finish_query_completion(&self) {
        self.finish_error(error_kind::QUERY_COMPLETION);
    }

    fn begin_terminal(&self) -> bool {
        self.terminal
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

impl Drop for QueryTraceSession {
    fn drop(&mut self) {
        if self.begin_terminal() {
            self.span.record(field_name::STATUS, status::ABANDONED);
            record_u64(
                &self.span,
                field_name::ELAPSED_MS,
                duration_millis(self.started.elapsed()),
            );
        }
    }
}

/// Request-scoped lifecycle handles for Local Query observability channels.
///
/// This combines two independent channels: the `tracing` trace session and the
/// optional Explainability session. It only co-locates their lifetimes; it
/// never merges their data models or derives one channel from the other.
#[derive(Debug, Clone)]
pub(crate) struct LocalQueryInstrumentation {
    trace: Option<Arc<QueryTraceSession>>,
    explainability: Option<Arc<QueryExplainabilitySession>>,
}

impl LocalQueryInstrumentation {
    /// Start both channels for one Local Query request.
    ///
    /// Returns `None` for non-Local methods or when both channels are disabled.
    /// Neither channel is cached on the runtime.
    pub(crate) async fn start(options: &QueryOptions, streaming: bool) -> Option<Self> {
        if options.method != SearchMethod::Local {
            return None;
        }
        let explainability = QueryExplainabilitySession::start_local(options).await;
        let trace = QueryTraceSession::start(options, streaming);
        if trace.is_none() && explainability.is_none() {
            return None;
        }
        Some(Self {
            trace,
            explainability,
        })
    }

    /// Borrow the tracing session, when enabled.
    pub(crate) fn trace(&self) -> Option<&QueryTraceSession> {
        self.trace.as_deref()
    }

    /// Borrow the Explainability session, when enabled.
    pub(crate) fn explainability(&self) -> Option<&QueryExplainabilitySession> {
        self.explainability.as_deref()
    }

    /// Finalize both channels with a successful Query result.
    pub(crate) async fn finish_success(&self, result: &QueryResult, context_tokens: Option<u64>) {
        if let Some(trace) = &self.trace {
            trace.finish_ok(result, context_tokens);
        }
        if let Some(session) = &self.explainability {
            session.finish_success().await;
        }
    }

    /// Finalize both channels with a typed Query error.
    pub(crate) async fn finish_query_error(&self, error: &QueryError) {
        if let Some(trace) = &self.trace {
            trace.finish_query_error(error);
        }
        if let Some(session) = &self.explainability {
            session.finish_query_error(error).await;
        }
    }

    /// Finalize both channels with a request-level `GraphLoom` error.
    pub(crate) async fn finish_graphloom_error(&self, error: &GraphLoomError) {
        if let Some(trace) = &self.trace {
            trace.finish_graphloom_error(error);
        }
        if let Some(session) = &self.explainability {
            session.finish_graphloom_error(error).await;
        }
    }

    /// Finalize both channels for a stream that ended without `Completed`.
    pub(crate) async fn finish_stream_ended(&self) {
        if let Some(trace) = &self.trace {
            trace.finish_query_completion();
        }
        if let Some(session) = &self.explainability {
            session.finish_stream_ended().await;
        }
    }
}

/// Lifetime guard for the completion `graphloom.llm.request` span.
///
/// The span is opened before the provider handshake and stays open through
/// full stream consumption. Explicit terminal states record `ok`/`error`;
/// dropping without a terminal state records `abandoned` synchronously.
#[derive(Debug)]
pub(crate) struct LlmSpanLatch {
    span: Option<Span>,
    started: Instant,
    finalized: bool,
}

impl LlmSpanLatch {
    /// Create a latch for the LLM span opened before the handshake.
    pub(crate) fn new(span: Option<Span>, started: Instant) -> Self {
        Self {
            span,
            started,
            finalized: false,
        }
    }

    /// Record the successful completion state from the real Query result.
    pub(crate) fn finish_ok(&mut self, result: &QueryResult) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        let Some(span) = &self.span else {
            return;
        };
        span.record(field_name::STATUS, status::OK);
        let usage = result.usage.categories.get("response");
        record_u64(
            span,
            field_name::INPUT_TOKENS,
            usage.and_then(|usage| usize_to_u64(usage.prompt_tokens)),
        );
        record_u64(
            span,
            field_name::OUTPUT_TOKENS,
            usage.and_then(|usage| usize_to_u64(usage.output_tokens)),
        );
        record_u64(
            span,
            field_name::ELAPSED_MS,
            duration_millis(self.started.elapsed()),
        );
    }

    /// Record the completion failure terminal state.
    pub(crate) fn finish_completion_error(&mut self) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        let Some(span) = &self.span else {
            return;
        };
        span.record(field_name::STATUS, status::ERROR);
        span.record(field_name::ERROR_KIND, error_kind::QUERY_COMPLETION);
        record_u64(
            span,
            field_name::ELAPSED_MS,
            duration_millis(self.started.elapsed()),
        );
    }
}

impl Drop for LlmSpanLatch {
    fn drop(&mut self) {
        if self.finalized {
            return;
        }
        let Some(span) = &self.span else {
            return;
        };
        span.record(field_name::STATUS, status::ABANDONED);
        record_u64(
            span,
            field_name::ELAPSED_MS,
            duration_millis(self.started.elapsed()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{duration_millis, query_error_kind, usize_to_u64};
    use crate::query::{QueryError, SearchMethod};

    #[test]
    fn test_should_convert_usize_and_durations_safely() {
        assert_eq!(usize_to_u64(0), Some(0));
        assert_eq!(usize_to_u64(usize::MAX), u64::try_from(usize::MAX).ok());
        assert_eq!(duration_millis(std::time::Duration::ZERO), Some(0));
        assert_eq!(
            duration_millis(std::time::Duration::from_secs(1)),
            Some(1_000)
        );
    }

    #[test]
    fn test_should_classify_every_query_error_variant() {
        let errors = [
            QueryError::InvalidQueryConfig {
                method: SearchMethod::Local,
                operation: "test",
                message: "x".to_owned(),
            },
            QueryError::MissingQueryTable {
                method: SearchMethod::Local,
                operation: "test",
                table: "t",
            },
            QueryError::QueryMethod {
                method: None,
                operation: "test",
                message: "x".to_owned(),
            },
        ];
        for error in errors {
            assert!(!query_error_kind(&error).is_empty());
        }
    }
}
