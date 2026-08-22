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
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use tracing::{Instrument, Span};

use super::{
    QueryError, QueryOptions, QueryResult, Result, SearchMethod,
    explainability::{
        BasicQueryExplainability, DriftQueryExplainability, GlobalQueryExplainability,
        LocalQueryExplainability, QueryExplainabilitySession,
    },
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
        GraphLoomError::Telemetry { .. } => error_kind::TELEMETRY_OUTPUT,
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
    let Some(root_span) = trace.clone_root_span() else {
        return future.await;
    };
    let span = tracing::info_span!(
        parent: &root_span,
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
    span: Mutex<Option<Span>>,
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
            span: Mutex::new(Some(span)),
            started: Instant::now(),
            terminal: AtomicBool::new(false),
        });
        if let Some(run_id) = options
            .explainability
            .as_ref()
            .map(|explainability| explainability.run_id().as_str())
            && let Some(span) = session.clone_root_span()
        {
            span.record(field_name::RUN_ID, run_id);
        }
        Some(session)
    }

    /// Clone the root span, used as the explicit parent of top-level stages.
    ///
    /// The clone is temporary and is released immediately after child span
    /// creation; it never extends the root span's terminal close.
    pub(crate) fn clone_root_span(&self) -> Option<Span> {
        self.span
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Take the only long-lived root span handle for terminal recording.
    fn take_root_span(&self) -> Option<Span> {
        self.span
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    /// Record a successful terminal state from the real Query result.
    pub(crate) fn finish_ok(&self, result: &QueryResult, context_tokens: Option<u64>) {
        if !self.begin_terminal() {
            return;
        }
        let Some(span) = self.take_root_span() else {
            return;
        };
        span.record(field_name::STATUS, status::OK);
        record_u64(
            &span,
            field_name::INPUT_TOKENS,
            usize_to_u64(result.usage.prompt_tokens),
        );
        record_u64(
            &span,
            field_name::OUTPUT_TOKENS,
            usize_to_u64(result.usage.output_tokens),
        );
        record_u64(&span, field_name::CONTEXT_TOKENS, context_tokens);
        record_u64(
            &span,
            field_name::LLM_CALLS,
            usize_to_u64(result.usage.llm_calls),
        );
        record_u64(
            &span,
            field_name::ELAPSED_MS,
            duration_millis(self.started.elapsed()),
        );
        drop(span);
    }

    /// Record a failed terminal state with a stable error category.
    pub(crate) fn finish_error(&self, error_kind_value: &'static str) {
        if !self.begin_terminal() {
            return;
        }
        let Some(span) = self.take_root_span() else {
            return;
        };
        span.record(field_name::STATUS, status::ERROR);
        span.record(field_name::ERROR_KIND, error_kind_value);
        record_u64(
            &span,
            field_name::ELAPSED_MS,
            duration_millis(self.started.elapsed()),
        );
        drop(span);
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
        if !self.begin_terminal() {
            return;
        }
        let Some(span) = self
            .span
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        else {
            return;
        };
        span.record(field_name::STATUS, status::ABANDONED);
        record_u64(
            &span,
            field_name::ELAPSED_MS,
            duration_millis(self.started.elapsed()),
        );
    }
}

/// Request-scoped lifecycle handles for Query observability channels.
///
/// This combines two independent channels: the `tracing` trace session and the
/// optional Explainability session. It only co-locates their lifetimes; it
/// never merges their data models or derives one channel from the other.
#[derive(Debug, Clone)]
pub(crate) struct QueryInstrumentation {
    trace: Option<Arc<QueryTraceSession>>,
    explainability: Option<QueryMethodExplainability>,
}

#[derive(Debug, Clone)]
enum QueryMethodExplainability {
    Basic(BasicQueryExplainability),
    Local(LocalQueryExplainability),
    Global(GlobalQueryExplainability),
    Drift(DriftQueryExplainability),
}

impl QueryMethodExplainability {
    fn session(&self) -> &QueryExplainabilitySession {
        match self {
            Self::Basic(handle) => handle.session(),
            Self::Local(handle) => handle.session(),
            Self::Global(handle) => handle.session(),
            Self::Drift(handle) => handle.session(),
        }
    }
}

impl QueryInstrumentation {
    /// Start supported observability channels for one Query request.
    ///
    /// Local Search may start tracing and Explainability. Basic plus static and Dynamic Global
    /// Search and DRIFT may start Explainability only.
    /// No request-scoped channel is cached on the runtime.
    pub(crate) async fn start(options: &QueryOptions, streaming: bool) -> Option<Self> {
        let explainability = match options.method {
            SearchMethod::Basic => QueryExplainabilitySession::start(options)
                .await
                .map(BasicQueryExplainability::from_session)
                .map(QueryMethodExplainability::Basic),
            SearchMethod::Local => QueryExplainabilitySession::start(options)
                .await
                .map(LocalQueryExplainability::from_session)
                .map(QueryMethodExplainability::Local),
            SearchMethod::Global => QueryExplainabilitySession::start(options)
                .await
                .map(GlobalQueryExplainability::from_session)
                .map(QueryMethodExplainability::Global),
            SearchMethod::Drift => QueryExplainabilitySession::start(options)
                .await
                .map(DriftQueryExplainability::from_session)
                .map(QueryMethodExplainability::Drift),
        };
        let trace = (options.method == SearchMethod::Local)
            .then(|| QueryTraceSession::start(options, streaming))
            .flatten();
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
    pub(crate) fn local_explainability(&self) -> Option<&LocalQueryExplainability> {
        match self.explainability.as_ref() {
            Some(QueryMethodExplainability::Local(handle)) => Some(handle),
            Some(QueryMethodExplainability::Basic(_))
            | Some(QueryMethodExplainability::Global(_))
            | Some(QueryMethodExplainability::Drift(_))
            | None => None,
        }
    }

    /// Borrow Global Explainability, when enabled.
    pub(crate) fn global_explainability(&self) -> Option<&GlobalQueryExplainability> {
        match self.explainability.as_ref() {
            Some(QueryMethodExplainability::Global(handle)) => Some(handle),
            Some(QueryMethodExplainability::Basic(_))
            | Some(QueryMethodExplainability::Local(_))
            | Some(QueryMethodExplainability::Drift(_))
            | None => None,
        }
    }

    /// Borrow Basic Explainability, when enabled.
    pub(crate) fn basic_explainability(&self) -> Option<&BasicQueryExplainability> {
        match self.explainability.as_ref() {
            Some(QueryMethodExplainability::Basic(handle)) => Some(handle),
            Some(QueryMethodExplainability::Local(_))
            | Some(QueryMethodExplainability::Global(_))
            | Some(QueryMethodExplainability::Drift(_))
            | None => None,
        }
    }

    /// Borrow DRIFT Explainability, when enabled.
    pub(crate) fn drift_explainability(&self) -> Option<&DriftQueryExplainability> {
        match self.explainability.as_ref() {
            Some(QueryMethodExplainability::Drift(handle)) => Some(handle),
            Some(QueryMethodExplainability::Basic(_))
            | Some(QueryMethodExplainability::Local(_))
            | Some(QueryMethodExplainability::Global(_))
            | None => None,
        }
    }

    /// Close the root tracing span with a successful Query result.
    ///
    /// The caller must close the LLM span first and only then await any
    /// Explainability sink work.
    pub(crate) fn finish_trace_success(&self, result: &QueryResult, context_tokens: Option<u64>) {
        if let Some(trace) = &self.trace {
            trace.finish_ok(result, context_tokens);
        }
    }

    /// Finalize the Explainability run after both tracing spans are closed.
    pub(crate) async fn finish_explainability_success(&self) {
        if let Some(explainability) = &self.explainability {
            explainability.session().finish_success().await;
        }
    }

    /// Finalize both channels with a typed Query error.
    pub(crate) async fn finish_query_error(&self, error: &QueryError) {
        if let Some(trace) = &self.trace {
            trace.finish_query_error(error);
        }
        if let Some(explainability) = &self.explainability {
            explainability.session().finish_query_error(error).await;
        }
    }

    /// Finalize both channels with a request-level `GraphLoom` error.
    pub(crate) async fn finish_graphloom_error(&self, error: &GraphLoomError) {
        if let Some(trace) = &self.trace {
            trace.finish_graphloom_error(error);
        }
        if let Some(explainability) = &self.explainability {
            explainability.session().finish_graphloom_error(error).await;
        }
    }

    /// Finalize both channels for a stream that ended without `Completed`.
    pub(crate) async fn finish_stream_ended(&self) {
        if let Some(trace) = &self.trace {
            trace.finish_query_completion();
        }
        if let Some(explainability) = &self.explainability {
            explainability.session().finish_stream_ended().await;
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

    /// Clone the LLM span so stream polls run inside it.
    ///
    /// The clone is temporary and is released after each poll; it never delays
    /// the terminal close.
    pub(crate) fn span(&self) -> Option<Span> {
        self.span.clone()
    }

    /// Record the successful completion state from the real Query result.
    pub(crate) fn finish_ok(&mut self, result: &QueryResult) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        let Some(span) = self.span.take() else {
            return;
        };
        span.record(field_name::STATUS, status::OK);
        let usage = result.usage.categories.get("response");
        record_u64(
            &span,
            field_name::INPUT_TOKENS,
            usage.and_then(|usage| usize_to_u64(usage.prompt_tokens)),
        );
        record_u64(
            &span,
            field_name::OUTPUT_TOKENS,
            usage.and_then(|usage| usize_to_u64(usage.output_tokens)),
        );
        record_u64(
            &span,
            field_name::ELAPSED_MS,
            duration_millis(self.started.elapsed()),
        );
        drop(span);
    }

    /// Record the completion failure terminal state.
    pub(crate) fn finish_completion_error(&mut self) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        let Some(span) = self.span.take() else {
            return;
        };
        span.record(field_name::STATUS, status::ERROR);
        span.record(field_name::ERROR_KIND, error_kind::QUERY_COMPLETION);
        record_u64(
            &span,
            field_name::ELAPSED_MS,
            duration_millis(self.started.elapsed()),
        );
        drop(span);
    }
}

impl Drop for LlmSpanLatch {
    fn drop(&mut self) {
        if self.finalized {
            return;
        }
        let Some(span) = self.span.take() else {
            return;
        };
        span.record(field_name::STATUS, status::ABANDONED);
        record_u64(
            &span,
            field_name::ELAPSED_MS,
            duration_millis(self.started.elapsed()),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex, atomic::AtomicBool},
        time::{Duration, Instant},
    };

    use super::{QueryTraceSession, duration_millis, query_error_kind, usize_to_u64};
    use crate::{
        observability::{field_name, span_name},
        query::{QueryContext, QueryError, QueryResult, QueryUsage, SearchMethod},
        test_support::tracing_capture,
    };

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

    #[test]
    fn test_should_use_session_started_for_root_elapsed() {
        let state = Arc::new(Mutex::new(tracing_capture::CaptureState::default()));
        let subscriber = tracing_capture::capture_subscriber(state.clone());
        let _guard = tracing::subscriber::set_default(subscriber);

        let span = tracing::info_span!(
            span_name::QUERY_LOCAL,
            "graphloom.observability.version" = 1_u64,
            "graphloom.run.id" = tracing::field::Empty,
            "graphloom.operation" = "query",
            "graphloom.query.method" = "local",
            "graphloom.query.streaming" = false,
            "graphloom.explainability.enabled" = false,
            "graphloom.status" = tracing::field::Empty,
            "graphloom.error.kind" = tracing::field::Empty,
            "graphloom.input.tokens" = tracing::field::Empty,
            "graphloom.output.tokens" = tracing::field::Empty,
            "graphloom.context.tokens" = tracing::field::Empty,
            "graphloom.llm.calls" = tracing::field::Empty,
            "graphloom.elapsed_ms" = tracing::field::Empty,
        );
        let started = Instant::now()
            .checked_sub(Duration::from_secs(5))
            .expect("past instant");
        let session = QueryTraceSession {
            span: Mutex::new(Some(span)),
            started,
            terminal: AtomicBool::new(false),
        };
        let result = QueryResult {
            response: "answer".to_owned(),
            context: QueryContext::default(),
            elapsed: Duration::from_millis(1),
            usage: QueryUsage::default(),
        };

        session.finish_ok(&result, None);

        let state = state.lock().expect("capture state");
        let captured = state
            .spans
            .iter()
            .find(|span| span.name == span_name::QUERY_LOCAL)
            .expect("root span");
        let elapsed_ms = captured
            .field(field_name::ELAPSED_MS)
            .expect("root elapsed")
            .parse::<u64>()
            .expect("u64 elapsed");
        assert!(
            elapsed_ms >= 5_000,
            "root elapsed must cover request lifetime"
        );
        assert_ne!(
            elapsed_ms,
            u64::try_from(result.elapsed.as_millis()).expect("result elapsed"),
            "root elapsed must not reuse QueryResult.elapsed"
        );
        assert!(captured.closed);
        assert_eq!(captured.field(field_name::STATUS), Some("\"ok\""));
    }
}
