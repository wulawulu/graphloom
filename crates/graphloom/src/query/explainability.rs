//! Request-scoped Local Search Explainability orchestration.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::Utc;

use super::{QueryError, QueryOptions, SearchMethod};
use crate::{
    GraphLoomError,
    explainability::{
        ExplainabilityContentMode, ExplainabilityContractError, ExplainabilityEvent,
        ExplainabilityQueryMethod, ExplainabilityRecord, ExplainabilityRunId,
        ExplainabilityRunKind, ExplainabilitySink, ExplainabilitySpanId, QueryStarted,
        RunCompleted, RunFailed, RunStarted,
    },
};

const DELIVERY_ERROR_KIND: &str = "explainability_delivery";
const DELIVERY_ERROR_MESSAGE: &str = "One or more explainability records could not be delivered.";
const QUERY_ERROR_MESSAGE: &str = "Local query execution failed.";

/// Request-scoped configuration for Query Explainability.
///
/// The caller owns the run identity and sink. This allows a host such as Studio to create a run,
/// establish subscriptions, and then execute the Query with the same identity. In the current
/// phase only Local Search emits Explainability events; other Query methods ignore this option.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct QueryExplainabilityOptions {
    run_id: ExplainabilityRunId,
    content_mode: ExplainabilityContentMode,
    sink: Arc<dyn ExplainabilitySink>,
}

impl QueryExplainabilityOptions {
    /// Create options using a caller-provided run identity.
    #[must_use]
    pub fn new(
        run_id: ExplainabilityRunId,
        content_mode: ExplainabilityContentMode,
        sink: Arc<dyn ExplainabilitySink>,
    ) -> Self {
        Self {
            run_id,
            content_mode,
            sink,
        }
    }

    /// Create options with a newly generated run identity.
    #[must_use]
    pub fn generated(
        content_mode: ExplainabilityContentMode,
        sink: Arc<dyn ExplainabilitySink>,
    ) -> Self {
        Self::new(ExplainabilityRunId::generate(), content_mode, sink)
    }

    /// Borrow the caller-owned run identity.
    #[must_use]
    pub const fn run_id(&self) -> &ExplainabilityRunId {
        &self.run_id
    }

    /// Return the selected content-disclosure mode.
    #[must_use]
    pub const fn content_mode(&self) -> ExplainabilityContentMode {
        self.content_mode
    }

    /// Borrow the non-null sink used for this request.
    #[must_use]
    pub const fn sink(&self) -> &Arc<dyn ExplainabilitySink> {
        &self.sink
    }
}

#[derive(Debug)]
pub(crate) struct QueryExplainabilitySpans {
    root: ExplainabilitySpanId,
    mapping: ExplainabilitySpanId,
    embedding: ExplainabilitySpanId,
    retrieval: ExplainabilitySpanId,
    graph_expansion: ExplainabilitySpanId,
    context: ExplainabilitySpanId,
    llm: ExplainabilitySpanId,
}

impl QueryExplainabilitySpans {
    fn generate() -> Self {
        Self {
            root: ExplainabilitySpanId::generate(),
            mapping: ExplainabilitySpanId::generate(),
            embedding: ExplainabilitySpanId::generate(),
            retrieval: ExplainabilitySpanId::generate(),
            graph_expansion: ExplainabilitySpanId::generate(),
            context: ExplainabilitySpanId::generate(),
            llm: ExplainabilitySpanId::generate(),
        }
    }

    pub(crate) const fn root(&self) -> &ExplainabilitySpanId {
        &self.root
    }

    pub(crate) const fn mapping(&self) -> &ExplainabilitySpanId {
        &self.mapping
    }

    pub(crate) const fn embedding(&self) -> &ExplainabilitySpanId {
        &self.embedding
    }

    pub(crate) const fn retrieval(&self) -> &ExplainabilitySpanId {
        &self.retrieval
    }

    pub(crate) const fn graph_expansion(&self) -> &ExplainabilitySpanId {
        &self.graph_expansion
    }

    pub(crate) const fn context(&self) -> &ExplainabilitySpanId {
        &self.context
    }

    pub(crate) const fn llm(&self) -> &ExplainabilitySpanId {
        &self.llm
    }
}

/// One Local Query's event identity, delivery state, and terminal lifecycle.
#[derive(Debug)]
pub(crate) struct QueryExplainabilitySession {
    run_id: ExplainabilityRunId,
    content_mode: ExplainabilityContentMode,
    sink: Arc<dyn ExplainabilitySink>,
    spans: QueryExplainabilitySpans,
    started: Instant,
    delivery_failure_count: AtomicUsize,
    terminal_started: AtomicBool,
}

impl QueryExplainabilitySession {
    pub(crate) fn new(options: &QueryExplainabilityOptions) -> Self {
        Self {
            run_id: options.run_id.clone(),
            content_mode: options.content_mode,
            sink: Arc::clone(&options.sink),
            spans: QueryExplainabilitySpans::generate(),
            started: Instant::now(),
            delivery_failure_count: AtomicUsize::new(0),
            terminal_started: AtomicBool::new(false),
        }
    }

    pub(crate) async fn start_local(options: &QueryOptions) -> Option<Arc<Self>> {
        if options.method != SearchMethod::Local {
            return None;
        }
        let session = options
            .explainability
            .as_ref()
            .map(|options| Arc::new(Self::new(options)))?;
        session.start(&options.query).await;
        Some(session)
    }

    pub(crate) const fn spans(&self) -> &QueryExplainabilitySpans {
        &self.spans
    }

    pub(crate) fn content(&self, value: &str) -> Option<String> {
        self.content_mode
            .includes_content()
            .then(|| value.to_owned())
    }

    pub(crate) async fn start(&self, query: &str) {
        self.emit(
            self.spans.root(),
            None,
            ExplainabilityEvent::RunStarted(RunStarted::new(
                ExplainabilityRunKind::Query,
                self.content_mode,
            )),
        )
        .await;
        let mut started = QueryStarted::new(ExplainabilityQueryMethod::Local);
        started.query = self.content(query);
        self.emit(
            self.spans.root(),
            None,
            ExplainabilityEvent::QueryStarted(started),
        )
        .await;
    }

    pub(crate) async fn emit(
        &self,
        span_id: &ExplainabilitySpanId,
        parent_span_id: Option<&ExplainabilitySpanId>,
        event: ExplainabilityEvent,
    ) {
        let record = Arc::new(ExplainabilityRecord::new(
            self.run_id.clone(),
            Utc::now(),
            span_id.clone(),
            parent_span_id.cloned(),
            event,
        ));
        if let Err(error) = self.sink.emit(record).await {
            self.delivery_failure_count.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                error = %error,
                "Explainability sink rejected a Local Query record"
            );
        }
    }

    pub(crate) async fn emit_contract(
        &self,
        span_id: &ExplainabilitySpanId,
        parent_span_id: Option<&ExplainabilitySpanId>,
        event: Result<ExplainabilityEvent, ExplainabilityContractError>,
    ) {
        match event {
            Ok(event) => self.emit(span_id, parent_span_id, event).await,
            Err(error) => {
                self.mark_sidecar_failure("event_contract");
                tracing::warn!(
                    error = %error,
                    "Explainability event failed Local Query contract validation"
                );
            }
        }
    }

    pub(crate) fn mark_sidecar_failure(&self, failure_kind: &'static str) {
        self.delivery_failure_count.fetch_add(1, Ordering::Relaxed);
        tracing::warn!(
            failure_kind,
            "Local Query Explainability sidecar data is incomplete"
        );
    }

    pub(crate) fn usize_to_u64(&self, value: usize) -> Option<u64> {
        if let Ok(value) = u64::try_from(value) {
            Some(value)
        } else {
            self.mark_sidecar_failure("numeric_conversion");
            None
        }
    }

    pub(crate) fn duration_millis(&self, duration: Duration) -> Option<u64> {
        if let Ok(value) = u64::try_from(duration.as_millis()) {
            Some(value)
        } else {
            self.mark_sidecar_failure("elapsed_conversion");
            None
        }
    }

    pub(crate) async fn finish_success(&self) {
        if !self.begin_terminal() {
            return;
        }
        let elapsed_ms = self.duration_millis(self.started.elapsed());
        let event = if self.delivery_failure_count.load(Ordering::Relaxed) == 0 {
            elapsed_ms.map(|value| ExplainabilityEvent::RunCompleted(RunCompleted::new(value)))
        } else {
            Some(delivery_failed_event())
        };
        if let Some(event) = event {
            self.emit(self.spans.root(), None, event).await;
        } else {
            self.emit(self.spans.root(), None, delivery_failed_event())
                .await;
        }
        self.finish_run().await;
    }

    pub(crate) async fn finish_query_error(&self, error: &QueryError) {
        self.finish_failure(query_error_kind(error), QUERY_ERROR_MESSAGE)
            .await;
    }

    pub(crate) async fn finish_graphloom_error(&self, error: &GraphLoomError) {
        let error_kind = match error {
            GraphLoomError::Query(error) => query_error_kind(error),
            GraphLoomError::InvalidRoot { .. } => "invalid_query_config",
            _ => "query_runtime",
        };
        self.finish_failure(error_kind, QUERY_ERROR_MESSAGE).await;
    }

    pub(crate) async fn finish_stream_ended(&self) {
        self.finish_failure("query_completion", QUERY_ERROR_MESSAGE)
            .await;
    }

    async fn finish_failure(&self, error_kind: &'static str, message: &'static str) {
        if !self.begin_terminal() {
            return;
        }
        self.emit(
            self.spans.root(),
            None,
            ExplainabilityEvent::RunFailed(RunFailed::new(
                error_kind.to_owned(),
                message.to_owned(),
            )),
        )
        .await;
        self.finish_run().await;
    }

    fn begin_terminal(&self) -> bool {
        self.terminal_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    async fn finish_run(&self) {
        if let Err(error) = self.sink.finish_run(&self.run_id).await {
            tracing::warn!(error = %error, "Explainability sink failed to finalize a Local Query run");
        }
    }
}

fn delivery_failed_event() -> ExplainabilityEvent {
    ExplainabilityEvent::RunFailed(RunFailed::new(
        DELIVERY_ERROR_KIND.to_owned(),
        DELIVERY_ERROR_MESSAGE.to_owned(),
    ))
}

const fn query_error_kind(error: &QueryError) -> &'static str {
    match error {
        QueryError::InvalidQueryConfig { .. } => "invalid_query_config",
        QueryError::MissingQueryTable { .. } => "missing_query_table",
        QueryError::InvalidQueryTable { .. } => "invalid_query_table",
        QueryError::MissingVectorIndex { .. } => "missing_vector_index",
        QueryError::InvalidVectorIndex { .. } => "invalid_vector_index",
        QueryError::QueryPrompt { .. } => "query_prompt",
        QueryError::QueryEmbedding { .. } => "query_embedding",
        QueryError::QueryCompletion { .. } => "query_completion",
        QueryError::QueryParse { .. } => "query_parse",
        QueryError::QueryContext { .. } => "query_context",
        QueryError::QueryRuntime { .. } => "query_runtime",
        QueryError::QueryMethod { .. } => "query_method",
    }
}
