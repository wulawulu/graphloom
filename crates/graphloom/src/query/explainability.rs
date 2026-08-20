//! Request-scoped Query Explainability lifecycle and method-specific span topology.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::Utc;

use super::{
    QueryError, QueryOptions, SearchMethod,
    observability::{
        duration_millis as convert_duration_millis, graphloom_error_kind, query_error_kind,
        usize_to_u64 as convert_usize_to_u64,
    },
};
use crate::{
    GraphLoomError,
    explainability::{
        ExplainabilityContentMode, ExplainabilityContractError, ExplainabilityEvent,
        ExplainabilityQueryMethod, ExplainabilityRecord, ExplainabilityRunId,
        ExplainabilityRunKind, ExplainabilitySink, ExplainabilitySpanId, QueryStarted,
        RunCompleted, RunFailed, RunStarted,
    },
    observability::{error_kind, event_name},
};

const DELIVERY_ERROR_KIND: &str = "explainability_delivery";
const DELIVERY_ERROR_MESSAGE: &str = "One or more explainability records could not be delivered.";
const LOCAL_QUERY_ERROR_MESSAGE: &str = "Local query execution failed.";
const GLOBAL_QUERY_ERROR_MESSAGE: &str = "Global query execution failed.";

/// Request-scoped configuration for Query Explainability.
///
/// The caller owns the run identity and sink. This allows a host such as Studio to create a run,
/// establish subscriptions, and then execute the Query with the same identity. In the current
/// runtime supports Local Search and static or Dynamic Global Search. Basic and DRIFT queries
/// ignore this option without error until their complete evidence contracts are available.
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
pub(crate) struct LocalExplainabilitySpans {
    root: ExplainabilitySpanId,
    mapping: ExplainabilitySpanId,
    embedding: ExplainabilitySpanId,
    retrieval: ExplainabilitySpanId,
    graph_expansion: ExplainabilitySpanId,
    context: ExplainabilitySpanId,
    llm: ExplainabilitySpanId,
}

impl LocalExplainabilitySpans {
    fn generate(root: ExplainabilitySpanId) -> Self {
        Self {
            root,
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

#[derive(Debug)]
pub(crate) struct GlobalExplainabilitySpans {
    selection: ExplainabilitySpanId,
    context: ExplainabilitySpanId,
    map: ExplainabilitySpanId,
    reduce: ExplainabilitySpanId,
}

impl GlobalExplainabilitySpans {
    fn generate() -> Self {
        Self {
            selection: ExplainabilitySpanId::generate(),
            context: ExplainabilitySpanId::generate(),
            map: ExplainabilitySpanId::generate(),
            reduce: ExplainabilitySpanId::generate(),
        }
    }

    pub(crate) const fn selection(&self) -> &ExplainabilitySpanId {
        &self.selection
    }

    pub(crate) const fn context(&self) -> &ExplainabilitySpanId {
        &self.context
    }

    pub(crate) const fn map(&self) -> &ExplainabilitySpanId {
        &self.map
    }

    pub(crate) const fn reduce(&self) -> &ExplainabilitySpanId {
        &self.reduce
    }
}

/// One Query's method-neutral event identity, delivery state, and terminal lifecycle.
#[derive(Debug)]
pub(crate) struct QueryExplainabilitySession {
    run_id: ExplainabilityRunId,
    content_mode: ExplainabilityContentMode,
    sink: Arc<dyn ExplainabilitySink>,
    method: ExplainabilityQueryMethod,
    root_span: ExplainabilitySpanId,
    started: Instant,
    delivery_failure_count: AtomicUsize,
    terminal_started: AtomicBool,
}

impl QueryExplainabilitySession {
    fn new(options: &QueryExplainabilityOptions, method: SearchMethod) -> Self {
        Self {
            run_id: options.run_id.clone(),
            content_mode: options.content_mode,
            sink: Arc::clone(&options.sink),
            method: method.into(),
            root_span: ExplainabilitySpanId::generate(),
            started: Instant::now(),
            delivery_failure_count: AtomicUsize::new(0),
            terminal_started: AtomicBool::new(false),
        }
    }

    pub(crate) async fn start(options: &QueryOptions) -> Option<Arc<Self>> {
        let supported = matches!(options.method, SearchMethod::Local | SearchMethod::Global);
        if !supported {
            return None;
        }
        let session = options
            .explainability
            .as_ref()
            .map(|explainability| Arc::new(Self::new(explainability, options.method)))?;
        session.emit_start(&options.query).await;
        Some(session)
    }

    pub(crate) const fn root_span(&self) -> &ExplainabilitySpanId {
        &self.root_span
    }

    pub(crate) fn content(&self, value: &str) -> Option<String> {
        self.content_mode
            .includes_content()
            .then(|| value.to_owned())
    }

    pub(crate) const fn includes_content(&self) -> bool {
        self.content_mode.includes_content()
    }

    async fn emit_start(&self, query: &str) {
        self.emit(
            self.root_span(),
            None,
            ExplainabilityEvent::RunStarted(RunStarted::new(
                ExplainabilityRunKind::Query,
                self.content_mode,
            )),
        )
        .await;
        let mut started = QueryStarted::new(self.method);
        started.query = self.content(query);
        self.emit(
            self.root_span(),
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
        if self.sink.emit(record).await.is_err() {
            self.delivery_failure_count.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                name: event_name::QUERY_EXPLAINABILITY_DELIVERY_FAILED,
                {
                    "graphloom.run.id" = %self.run_id,
                    "graphloom.query.method" = %self.method_name(),
                    "graphloom.error.kind" = error_kind::EXPLAINABILITY_DELIVERY,
                },
                "Explainability sink rejected a Query record"
            );
        }
    }

    pub(crate) async fn emit_contract(
        &self,
        span_id: &ExplainabilitySpanId,
        parent_span_id: Option<&ExplainabilitySpanId>,
        event: Result<ExplainabilityEvent, ExplainabilityContractError>,
    ) {
        if let Ok(event) = event {
            self.emit(span_id, parent_span_id, event).await;
        } else {
            self.mark_sidecar_failure("event_contract");
            tracing::warn!(
                name: event_name::QUERY_EXPLAINABILITY_CONTRACT_FAILED,
                {
                    "graphloom.run.id" = %self.run_id,
                    "graphloom.query.method" = %self.method_name(),
                    "graphloom.error.kind" = error_kind::EVENT_CONTRACT,
                },
                "Explainability event failed Query contract validation"
            );
        }
    }

    pub(crate) fn mark_sidecar_failure(&self, failure_kind: &'static str) {
        self.delivery_failure_count.fetch_add(1, Ordering::Relaxed);
        tracing::warn!(
            name: event_name::QUERY_EXPLAINABILITY_SIDECAR_INCOMPLETE,
            {
                "graphloom.run.id" = %self.run_id,
                "graphloom.query.method" = %self.method_name(),
                "graphloom.error.kind" = failure_kind,
            },
            "Query Explainability sidecar data is incomplete"
        );
    }

    pub(crate) fn usize_to_u64(&self, value: usize) -> Option<u64> {
        let converted = convert_usize_to_u64(value);
        if converted.is_none() {
            self.mark_sidecar_failure("numeric_conversion");
        }
        converted
    }

    pub(crate) fn usize_to_u32(&self, value: usize) -> Option<u32> {
        let converted = u32::try_from(value).ok();
        if converted.is_none() {
            self.mark_sidecar_failure("numeric_conversion");
        }
        converted
    }

    pub(crate) fn duration_millis(&self, duration: Duration) -> Option<u64> {
        let converted = convert_duration_millis(duration);
        if converted.is_none() {
            self.mark_sidecar_failure("elapsed_conversion");
        }
        converted
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
            self.emit(self.root_span(), None, event).await;
        } else {
            self.emit(self.root_span(), None, delivery_failed_event())
                .await;
        }
        self.finish_run().await;
    }

    pub(crate) async fn finish_query_error(&self, error: &QueryError) {
        self.finish_failure(query_error_kind(error), self.query_error_message())
            .await;
    }

    pub(crate) async fn finish_graphloom_error(&self, error: &GraphLoomError) {
        self.finish_failure(graphloom_error_kind(error), self.query_error_message())
            .await;
    }

    pub(crate) async fn finish_stream_ended(&self) {
        self.finish_failure("query_completion", self.query_error_message())
            .await;
    }

    async fn finish_failure(&self, error_kind: &'static str, message: &'static str) {
        if !self.begin_terminal() {
            return;
        }
        self.emit(
            self.root_span(),
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
        if self.sink.finish_run(&self.run_id).await.is_err() {
            tracing::warn!(
                name: event_name::QUERY_EXPLAINABILITY_FINISH_FAILED,
                {
                    "graphloom.run.id" = %self.run_id,
                    "graphloom.query.method" = %self.method_name(),
                    "graphloom.error.kind" = error_kind::EXPLAINABILITY_FINISH,
                },
                "Explainability sink failed to finalize a Query run"
            );
        }
    }

    const fn method_name(&self) -> &'static str {
        match self.method {
            ExplainabilityQueryMethod::Basic => "basic",
            ExplainabilityQueryMethod::Local => "local",
            ExplainabilityQueryMethod::Global => "global",
            ExplainabilityQueryMethod::Drift => "drift",
        }
    }

    const fn query_error_message(&self) -> &'static str {
        match self.method {
            ExplainabilityQueryMethod::Global => GLOBAL_QUERY_ERROR_MESSAGE,
            ExplainabilityQueryMethod::Basic
            | ExplainabilityQueryMethod::Local
            | ExplainabilityQueryMethod::Drift => LOCAL_QUERY_ERROR_MESSAGE,
        }
    }
}

/// Local Search stage spans paired with the shared Query lifecycle.
#[derive(Debug, Clone)]
pub(crate) struct LocalQueryExplainability {
    session: Arc<QueryExplainabilitySession>,
    spans: Arc<LocalExplainabilitySpans>,
}

impl LocalQueryExplainability {
    #[cfg(test)]
    pub(crate) fn new(options: &QueryExplainabilityOptions) -> Self {
        Self::from_session(Arc::new(QueryExplainabilitySession::new(
            options,
            SearchMethod::Local,
        )))
    }

    pub(crate) fn from_session(session: Arc<QueryExplainabilitySession>) -> Self {
        let root = session.root_span().clone();
        Self {
            session,
            spans: Arc::new(LocalExplainabilitySpans::generate(root)),
        }
    }

    pub(crate) fn spans(&self) -> &LocalExplainabilitySpans {
        self.spans.as_ref()
    }

    pub(crate) fn session(&self) -> &QueryExplainabilitySession {
        self.session.as_ref()
    }
}

impl std::ops::Deref for LocalQueryExplainability {
    type Target = QueryExplainabilitySession;

    fn deref(&self) -> &Self::Target {
        self.session()
    }
}

/// Global Search stage spans paired with the shared Query lifecycle.
#[derive(Debug, Clone)]
pub(crate) struct GlobalQueryExplainability {
    session: Arc<QueryExplainabilitySession>,
    spans: Arc<GlobalExplainabilitySpans>,
}

impl GlobalQueryExplainability {
    #[cfg(test)]
    pub(crate) fn new(options: &QueryExplainabilityOptions) -> Self {
        Self::from_session(Arc::new(QueryExplainabilitySession::new(
            options,
            SearchMethod::Global,
        )))
    }

    pub(crate) fn from_session(session: Arc<QueryExplainabilitySession>) -> Self {
        Self {
            session,
            spans: Arc::new(GlobalExplainabilitySpans::generate()),
        }
    }

    pub(crate) fn spans(&self) -> &GlobalExplainabilitySpans {
        self.spans.as_ref()
    }

    pub(crate) fn session(&self) -> &QueryExplainabilitySession {
        self.session.as_ref()
    }

    pub(crate) fn batch_span(&self) -> ExplainabilitySpanId {
        ExplainabilitySpanId::generate()
    }

    pub(crate) fn rating_attempt_span(&self) -> ExplainabilitySpanId {
        ExplainabilitySpanId::generate()
    }
}

impl std::ops::Deref for GlobalQueryExplainability {
    type Target = QueryExplainabilitySession;

    fn deref(&self) -> &Self::Target {
        self.session()
    }
}

fn delivery_failed_event() -> ExplainabilityEvent {
    ExplainabilityEvent::RunFailed(RunFailed::new(
        DELIVERY_ERROR_KIND.to_owned(),
        DELIVERY_ERROR_MESSAGE.to_owned(),
    ))
}
