//! CLI-hosted OpenTelemetry trace export adapter.
//!
//! This module owns the optional OTLP/HTTP Trace pipeline used by
//! `graphloom query`:
//!
//! * [`OtlpTraceOptions`] holds the safe, secret-free configuration;
//! * [`OtlpTraceRuntime`] builds the batch exporter, provider, and tracer;
//! * [`OtlpTraceGuard`] explicitly flushes and shuts the provider down.
//!
//! The adapter is a CLI host concern and is intentionally not part of the
//! public `GraphLoom` library contract. It never installs a global OpenTelemetry
//! provider and never records endpoint, header, or token values.

use std::{error::Error, fmt, time::Duration};

use opentelemetry::{InstrumentationScope, KeyValue, trace::TracerProvider};
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    Resource,
    error::OTelSdkResult,
    trace::{SdkTracer, SdkTracerProvider, SpanExporter as SdkSpanExporter},
};
use tracing::Subscriber;
use tracing_subscriber::{EnvFilter, Layer, registry::LookupSpan};

use crate::{
    GraphLoomError,
    cli::{QueryArgs, error::Result},
    observability::OBSERVABILITY_CONTRACT_VERSION,
};

/// Timeout for one OTLP HTTP export request.
const OTLP_EXPORT_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for the explicit provider flush and shutdown.
const OTLP_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
/// Effective `service.name` when `--otel-service-name` is omitted.
const DEFAULT_OTLP_SERVICE_NAME: &str = "graphloom";
/// Instrumentation scope name.
const OTLP_SCOPE_NAME: &str = "graphloom";
/// Only export spans whose target lives under `graphloom::query`.
const OTLP_LAYER_FILTER: &str = "off,graphloom::query=info";
/// OTLP/HTTP signal path for traces.
const OTLP_TRACES_PATH: &str = "/v1/traces";

/// Safe OTLP trace configuration derived from the Query CLI arguments.
///
/// The type deliberately owns no secrets: standard `OTLP` header environment
/// variables are read by the official exporter, never by `GraphLoom`.
#[derive(Clone)]
pub(crate) struct OtlpTraceOptions {
    endpoint: String,
    service_name: String,
    export_timeout: Duration,
    shutdown_timeout: Duration,
}

impl OtlpTraceOptions {
    /// Create options with the fixed production timeouts.
    pub(crate) fn new(endpoint: String, service_name: String) -> Self {
        Self {
            endpoint,
            service_name,
            export_timeout: OTLP_EXPORT_TIMEOUT,
            shutdown_timeout: OTLP_SHUTDOWN_TIMEOUT,
        }
    }

    /// Derive options from Query arguments; `None` when OTLP is disabled.
    pub(crate) fn from_args(args: &QueryArgs) -> Option<Self> {
        let endpoint = args.otel_endpoint.clone()?;
        let service_name = match &args.otel_service_name {
            Some(name) => name.clone(),
            None => DEFAULT_OTLP_SERVICE_NAME.to_owned(),
        };
        Some(Self::new(endpoint, service_name))
    }

    /// Collector base endpoint (never logged or exported).
    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Effective `service.name` resource attribute.
    pub(crate) fn service_name(&self) -> &str {
        &self.service_name
    }

    /// Per-export HTTP timeout.
    pub(crate) fn export_timeout(&self) -> Duration {
        self.export_timeout
    }

    /// Explicit provider shutdown timeout.
    pub(crate) fn shutdown_timeout(&self) -> Duration {
        self.shutdown_timeout
    }
}

impl fmt::Debug for OtlpTraceOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OtlpTraceOptions")
            .field("endpoint", &"<redacted>")
            .field("service_name", &self.service_name)
            .field("export_timeout", &self.export_timeout)
            .field("shutdown_timeout", &self.shutdown_timeout)
            .finish()
    }
}

/// Built OTLP trace pipeline: provider, tracer, and shutdown timeout.
#[derive(Debug)]
pub(crate) struct OtlpTraceRuntime {
    provider: SdkTracerProvider,
    tracer: SdkTracer,
    shutdown_timeout: Duration,
}

impl OtlpTraceRuntime {
    /// Build the OTLP/HTTP binary exporter, batch provider, and tracer.
    ///
    /// # Errors
    ///
    /// Returns a safe telemetry error when the exporter cannot be built. The
    /// error never contains the collector endpoint.
    pub(crate) fn build(options: &OtlpTraceOptions) -> Result<Self> {
        let exporter = SpanExporter::builder()
            .with_http()
            .with_endpoint(traces_endpoint(options.endpoint()))
            .with_protocol(Protocol::HttpBinary)
            .with_timeout(options.export_timeout())
            .build()
            .map_err(|source| telemetry_error("build OTLP trace exporter", source))?;
        Ok(Self::from_exporter(options, exporter))
    }

    /// Build a provider around an already constructed span exporter.
    ///
    /// Production uses the OTLP exporter through [`Self::build`]; tests inject
    /// recording exporters through this crate-private constructor.
    pub(crate) fn from_exporter(
        options: &OtlpTraceOptions,
        exporter: impl SdkSpanExporter + 'static,
    ) -> Self {
        let resource = build_resource(options.service_name());
        let scope = instrumentation_scope();
        let provider = SdkTracerProvider::builder()
            .with_resource(resource)
            .with_batch_exporter(exporter)
            .build();
        let tracer = provider.tracer_with_scope(scope);
        Self {
            provider,
            tracer,
            shutdown_timeout: options.shutdown_timeout(),
        }
    }

    /// Build a runtime around a pre-constructed provider for unit tests.
    #[cfg(test)]
    pub(crate) fn from_provider_for_test(
        provider: SdkTracerProvider,
        shutdown_timeout: Duration,
    ) -> Self {
        let scope = instrumentation_scope();
        let tracer = provider.tracer_with_scope(scope);
        Self {
            provider,
            tracer,
            shutdown_timeout,
        }
    }

    /// Borrow the tracer used by the `tracing-opentelemetry` layer.
    pub(crate) fn tracer(&self) -> &SdkTracer {
        &self.tracer
    }

    /// Convert the runtime into the explicit shutdown guard.
    pub(crate) fn into_guard(self) -> OtlpTraceGuard {
        OtlpTraceGuard {
            provider: Some(self.provider),
            shutdown_timeout: self.shutdown_timeout,
        }
    }

    /// Explicitly shut the provider down after a subscriber install failure.
    ///
    /// This runs on a blocking thread so no batch worker thread is leaked and
    /// the async worker is never blocked. Cleanup failures are intentionally
    /// ignored: this path best-effort closes the provider, never overrides the
    /// primary install error, and never emits `CLI_TELEMETRY_SHUTDOWN_FAILED`
    /// because no subscriber was successfully installed.
    pub(crate) async fn shutdown_after_init_failure(self) {
        let timeout = self.shutdown_timeout;
        let provider = self.provider;
        let _ = tokio::task::spawn_blocking(move || {
            let _ = provider.force_flush();
            let _ = provider.shutdown_with_timeout(timeout);
        })
        .await;
    }
}

/// Append the OTLP/HTTP traces signal path to a collector base endpoint.
fn traces_endpoint(base_endpoint: &str) -> String {
    if base_endpoint.is_empty() {
        return String::new();
    }
    if base_endpoint.ends_with('/') {
        format!(
            "{base_endpoint}{}",
            OTLP_TRACES_PATH.trim_start_matches('/')
        )
    } else {
        format!("{base_endpoint}{OTLP_TRACES_PATH}")
    }
}

/// Consumable guard that explicitly flushes and shuts down the provider.
#[derive(Debug)]
#[must_use]
pub(crate) struct OtlpTraceGuard {
    provider: Option<SdkTracerProvider>,
    shutdown_timeout: Duration,
}

impl OtlpTraceGuard {
    /// Force-flush all completed spans and shut the provider down.
    ///
    /// The provider is moved onto a blocking thread so the Tokio worker never
    /// performs synchronous export or shutdown work. Consuming the guard makes
    /// a second shutdown impossible.
    ///
    /// # Errors
    ///
    /// Returns a safe telemetry error when flushing, shutdown, or the blocking
    /// task join fails. The error never contains the endpoint or response body.
    pub(crate) async fn shutdown(self) -> Result<()> {
        let Some(provider) = self.provider else {
            return Ok(());
        };
        let shutdown_timeout = self.shutdown_timeout;
        let outcome = run_telemetry_shutdown_blocking(move || {
            let flush_result = provider.force_flush();
            let shutdown_result = provider.shutdown_with_timeout(shutdown_timeout);
            TelemetryShutdownOutcome {
                flush_result,
                shutdown_result,
            }
        })
        .await;
        match outcome {
            Ok(outcome) => {
                if let Err(source) = outcome.flush_result {
                    return Err(telemetry_error("flush OTLP traces", source));
                }
                if let Err(source) = outcome.shutdown_result {
                    return Err(telemetry_error("shutdown OTLP tracer provider", source));
                }
                Ok(())
            }
            Err(source) => Err(join_telemetry_error(source)),
        }
    }
}

/// Result of the explicit flush-then-shutdown sequence.
#[derive(Debug)]
struct TelemetryShutdownOutcome {
    flush_result: OTelSdkResult,
    shutdown_result: OTelSdkResult,
}

/// Run the synchronous flush/shutdown sequence on a Tokio blocking thread.
async fn run_telemetry_shutdown_blocking(
    shutdown: impl FnOnce() -> TelemetryShutdownOutcome + Send + 'static,
) -> std::result::Result<TelemetryShutdownOutcome, tokio::task::JoinError> {
    tokio::task::spawn_blocking(shutdown).await
}

/// Map a blocking-task join failure to the stable telemetry error.
fn join_telemetry_error(source: tokio::task::JoinError) -> GraphLoomError {
    telemetry_error("join OTLP shutdown task", source)
}

/// Build the stable trace Resource, preserving SDK-provided defaults.
fn build_resource(service_name: &str) -> Resource {
    Resource::builder()
        .with_service_name(service_name.to_owned())
        .with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")))
        .with_attribute(KeyValue::new(
            "graphloom.observability.version",
            i64::from(OBSERVABILITY_CONTRACT_VERSION),
        ))
        .build()
}

/// Build the stable `graphloom` instrumentation scope.
fn instrumentation_scope() -> InstrumentationScope {
    InstrumentationScope::builder(OTLP_SCOPE_NAME)
        .with_version(env!("CARGO_PKG_VERSION"))
        .build()
}

/// Build the filtered `tracing-opentelemetry` layer for a Query subscriber.
pub(crate) fn otel_layer<S>(
    tracer: SdkTracer,
) -> tracing_subscriber::filter::Filtered<
    tracing_opentelemetry::OpenTelemetryLayer<S, SdkTracer>,
    EnvFilter,
    S,
>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    tracing_opentelemetry::layer()
        .with_tracer(tracer)
        .with_filter(EnvFilter::new(OTLP_LAYER_FILTER))
}

/// Map a source failure to the stable, endpoint-free telemetry error.
pub(crate) fn telemetry_error(
    operation: &'static str,
    source: impl Into<Box<dyn Error + Send + Sync>>,
) -> GraphLoomError {
    GraphLoomError::Telemetry {
        operation,
        source: source.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
        time::Duration,
    };

    use opentelemetry::{Key, Value};
    use opentelemetry_sdk::{
        Resource,
        error::OTelSdkError,
        trace::{SdkTracerProvider, SpanData, SpanExporter as SdkSpanExporter, SpanProcessor},
    };
    use tracing_subscriber::prelude::*;

    use super::{
        DEFAULT_OTLP_SERVICE_NAME, OTLP_LAYER_FILTER, OTLP_TRACES_PATH, OtlpTraceOptions,
        OtlpTraceRuntime, TelemetryShutdownOutcome, build_resource, instrumentation_scope,
        join_telemetry_error, otel_layer, run_telemetry_shutdown_blocking, traces_endpoint,
    };
    use crate::{
        GraphLoomError,
        cli::{Cli, Command},
        observability::{OBSERVABILITY_CONTRACT_VERSION, field_name, span_name},
    };

    #[derive(Clone, Debug)]
    struct RecordingSpanExporter {
        spans: Arc<Mutex<Vec<SpanData>>>,
        resource: Arc<Mutex<Option<Resource>>>,
        shutdown_calls: Arc<AtomicUsize>,
        export_error: Arc<Mutex<Option<String>>>,
    }

    impl RecordingSpanExporter {
        fn new() -> Self {
            Self {
                spans: Arc::new(Mutex::new(Vec::new())),
                resource: Arc::new(Mutex::new(None)),
                shutdown_calls: Arc::new(AtomicUsize::new(0)),
                export_error: Arc::new(Mutex::new(None)),
            }
        }

        fn set_export_error(&self, message: &str) {
            *self.export_error.lock().expect("export error") = Some(message.to_owned());
        }

        fn spans(&self) -> Vec<SpanData> {
            self.spans.lock().expect("spans").clone()
        }

        fn resource(&self) -> Option<Resource> {
            self.resource.lock().expect("resource").clone()
        }

        fn shutdown_calls(&self) -> usize {
            self.shutdown_calls.load(Ordering::SeqCst)
        }
    }

    impl SdkSpanExporter for RecordingSpanExporter {
        async fn export(&self, batch: Vec<SpanData>) -> Result<(), OTelSdkError> {
            if let Some(message) = self.export_error.lock().expect("export error").as_deref() {
                return Err(OTelSdkError::InternalFailure(message.to_owned()));
            }
            self.spans.lock().expect("spans").extend(batch);
            Ok(())
        }

        fn shutdown_with_timeout(&self, _timeout: Duration) -> Result<(), OTelSdkError> {
            self.shutdown_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn set_resource(&mut self, resource: &Resource) {
            *self.resource.lock().expect("resource") = Some(resource.clone());
        }
    }

    #[derive(Debug)]
    struct FailingShutdownProcessor;

    impl SpanProcessor for FailingShutdownProcessor {
        fn on_start(
            &self,
            _span: &mut opentelemetry_sdk::trace::Span,
            _cx: &opentelemetry::Context,
        ) {
        }

        fn on_end(&self, _span: SpanData) {}

        fn force_flush(&self) -> Result<(), OTelSdkError> {
            Ok(())
        }

        fn shutdown_with_timeout(&self, _timeout: Duration) -> Result<(), OTelSdkError> {
            Err(OTelSdkError::InternalFailure(
                "forced processor shutdown failure".to_owned(),
            ))
        }
    }

    fn resource_attribute(resource: &Resource, key: &str) -> Option<Value> {
        resource.get(&Key::from(key.to_owned()))
    }

    fn span_attribute(span: &SpanData, key: &str) -> Option<Value> {
        span.attributes
            .iter()
            .find(|attribute| attribute.key.as_str() == key)
            .map(|attribute| attribute.value.clone())
    }

    fn test_options(service_name: &str) -> OtlpTraceOptions {
        OtlpTraceOptions::new(
            "http://collector.invalid:4318".to_owned(),
            service_name.to_owned(),
        )
    }

    #[test]
    fn test_should_pin_default_service_name() {
        assert_eq!(DEFAULT_OTLP_SERVICE_NAME, "graphloom");
    }

    #[test]
    fn test_should_derive_options_from_query_args() {
        let cli = Cli::try_parse_from([
            "graphloom",
            "query",
            "--method",
            "local",
            "--otel-endpoint",
            "http://collector.invalid:4318",
            "question",
        ])
        .expect("Query arguments");
        let Command::Query(args) = cli.command else {
            panic!("expected Query command");
        };
        let options = OtlpTraceOptions::from_args(&args).expect("OTLP options");
        assert_eq!(options.endpoint(), "http://collector.invalid:4318");
        assert_eq!(options.service_name(), "graphloom");
        assert_eq!(options.export_timeout(), Duration::from_secs(10));
        assert_eq!(options.shutdown_timeout(), Duration::from_secs(10));
    }

    #[test]
    fn test_should_keep_options_unset_without_endpoint() {
        let options = OtlpTraceOptions::new(
            "http://collector.invalid:4318".to_owned(),
            "explicit".to_owned(),
        );
        assert_eq!(options.service_name(), "explicit");
        let cli = Cli::try_parse_from(["graphloom", "query", "question"]).expect("Query arguments");
        let Command::Query(args) = cli.command else {
            panic!("expected Query command");
        };
        assert!(OtlpTraceOptions::from_args(&args).is_none());
    }

    #[test]
    fn test_should_redact_endpoint_in_debug() {
        let options = test_options("graphloom-test");
        let debug = format!("{options:?}");
        assert!(!debug.contains("collector.invalid"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn test_should_build_expected_resource_and_scope() {
        let resource = build_resource("graphloom-test");
        assert_eq!(
            resource_attribute(&resource, "service.name")
                .as_ref()
                .map(Value::as_str)
                .as_deref(),
            Some("graphloom-test")
        );
        assert_eq!(
            resource_attribute(&resource, "service.version")
                .as_ref()
                .map(Value::as_str)
                .as_deref(),
            Some(env!("CARGO_PKG_VERSION"))
        );
        let expected_version = i64::from(OBSERVABILITY_CONTRACT_VERSION).to_string();
        assert_eq!(
            resource_attribute(&resource, "graphloom.observability.version")
                .as_ref()
                .map(Value::as_str)
                .as_deref(),
            Some(expected_version.as_str())
        );
        assert!(
            resource_attribute(&resource, "telemetry.sdk.name").is_some(),
            "SDK default resource attributes must be preserved"
        );
        let scope = instrumentation_scope();
        assert_eq!(scope.name(), "graphloom");
        assert_eq!(scope.version(), Some(env!("CARGO_PKG_VERSION")));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_should_flush_queued_spans_and_shutdown_on_guard_drop() {
        let options = test_options("graphloom-test");
        let exporter = RecordingSpanExporter::new();
        let runtime = OtlpTraceRuntime::from_exporter(&options, exporter.clone());
        let subscriber = tracing_subscriber::registry().with(otel_layer(runtime.tracer().clone()));
        let guard = tracing::subscriber::set_default(subscriber);

        let root = tracing::info_span!(
            target: "graphloom::query",
            span_name::QUERY_LOCAL,
            "graphloom.status" = tracing::field::Empty,
        );
        let child = tracing::info_span!(
            target: "graphloom::query",
            parent: &root,
            span_name::QUERY_RUNTIME,
        );
        drop(child);
        root.record(field_name::STATUS, "ok");
        drop(root);
        drop(guard);

        assert!(
            exporter.spans().is_empty(),
            "batch processor must not export synchronously on span close"
        );
        let guard = runtime.into_guard();
        guard.shutdown().await.expect("shutdown");

        let spans = exporter.spans();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].name.as_ref(), span_name::QUERY_RUNTIME);
        assert_eq!(spans[1].name.as_ref(), span_name::QUERY_LOCAL);
        assert_eq!(exporter.shutdown_calls(), 1);
        let resource = exporter.resource().expect("resource");
        assert_eq!(
            resource_attribute(&resource, "service.name")
                .as_ref()
                .map(Value::as_str)
                .as_deref(),
            Some("graphloom-test")
        );
        assert_eq!(spans[1].instrumentation_scope.name(), "graphloom");
        assert_eq!(
            spans[1].instrumentation_scope.version(),
            Some(env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(
            span_attribute(&spans[1], field_name::STATUS)
                .as_ref()
                .map(Value::as_str)
                .as_deref(),
            Some("ok")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_should_report_flush_failure_without_leaking_details() {
        let options = test_options("graphloom-test");
        let exporter = RecordingSpanExporter::new();
        exporter.set_export_error("forced export failure");
        let runtime = OtlpTraceRuntime::from_exporter(&options, exporter.clone());
        let subscriber = tracing_subscriber::registry().with(otel_layer(runtime.tracer().clone()));
        let guard = tracing::subscriber::set_default(subscriber);
        let span = tracing::info_span!(target: "graphloom::query", "graphloom.query.local");
        drop(span);
        drop(guard);
        let error = runtime
            .into_guard()
            .shutdown()
            .await
            .expect_err("flush failure must surface");
        assert!(matches!(
            error,
            GraphLoomError::Telemetry {
                operation: "flush OTLP traces",
                ..
            }
        ));
        assert_eq!(
            error.to_string(),
            "failed to flush OTLP traces for OpenTelemetry trace export"
        );
        assert!(!error.to_string().contains("collector.invalid"));
        assert!(!error.to_string().contains("forced export failure"));
        assert_eq!(
            exporter.shutdown_calls(),
            1,
            "shutdown must still run after flush failure"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_should_report_shutdown_failure_without_leaking_details() {
        let options = test_options("graphloom-test");
        let provider = SdkTracerProvider::builder()
            .with_span_processor(FailingShutdownProcessor)
            .build();
        let runtime =
            OtlpTraceRuntime::from_provider_for_test(provider, options.shutdown_timeout());
        let error = runtime
            .into_guard()
            .shutdown()
            .await
            .expect_err("shutdown failure must surface");
        assert!(matches!(
            error,
            GraphLoomError::Telemetry {
                operation: "shutdown OTLP tracer provider",
                ..
            }
        ));
        assert_eq!(
            error.to_string(),
            "failed to shutdown OTLP tracer provider for OpenTelemetry trace export"
        );
        assert!(!error.to_string().contains("forced shutdown failure"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_should_map_blocking_task_join_error_safely() {
        let join_error = run_telemetry_shutdown_blocking(|| {
            panic!("forced shutdown task panic");
        })
        .await
        .expect_err("panic must surface as join error");
        let error = join_telemetry_error(join_error);
        assert!(matches!(
            error,
            GraphLoomError::Telemetry {
                operation: "join OTLP shutdown task",
                ..
            }
        ));
        assert_eq!(
            error.to_string(),
            "failed to join OTLP shutdown task for OpenTelemetry trace export"
        );
        assert!(!error.to_string().contains("panic"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_should_run_shutdown_sequence_on_blocking_thread() {
        let worker_thread = thread::current().id();
        let observed_thread = Arc::new(Mutex::new(None));
        let observed = Arc::clone(&observed_thread);
        let outcome = run_telemetry_shutdown_blocking(move || {
            *observed.lock().expect("observed thread") = Some(thread::current().id());
            TelemetryShutdownOutcome {
                flush_result: Ok(()),
                shutdown_result: Ok(()),
            }
        })
        .await
        .expect("blocking shutdown");
        assert!(outcome.flush_result.is_ok());
        assert!(outcome.shutdown_result.is_ok());
        assert_ne!(
            *observed_thread.lock().expect("observed thread"),
            Some(worker_thread),
            "shutdown must run on a spawn_blocking thread, not the async worker"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_should_export_only_graphloom_query_targets() {
        let options = test_options("graphloom-test");
        let exporter = RecordingSpanExporter::new();
        let runtime = OtlpTraceRuntime::from_exporter(&options, exporter.clone());
        let subscriber = tracing_subscriber::registry().with(otel_layer(runtime.tracer().clone()));
        let guard = tracing::subscriber::set_default(subscriber);

        let query_span = tracing::info_span!(target: "graphloom::query", "graphloom.query.local");
        let reqwest_span = tracing::info_span!(target: "reqwest", "reqwest.internal");
        let otel_span = tracing::info_span!(target: "opentelemetry", "opentelemetry.internal");
        let unrelated_span = tracing::info_span!(target: "unrelated_dependency", "unrelated.span");
        drop(query_span);
        drop(reqwest_span);
        drop(otel_span);
        drop(unrelated_span);
        drop(guard);

        let guard = runtime.into_guard();
        guard.shutdown().await.expect("shutdown");
        let spans = exporter.spans();
        assert_eq!(
            spans.len(),
            1,
            "only graphloom::query spans must be exported"
        );
        assert_eq!(spans[0].name.as_ref(), "graphloom.query.local");
    }

    #[test]
    fn test_should_pin_otel_layer_filter() {
        assert_eq!(OTLP_LAYER_FILTER, "off,graphloom::query=info");
    }

    #[test]
    fn test_should_append_traces_signal_path_to_base_endpoint() {
        assert_eq!(traces_endpoint(""), "");
        assert_eq!(
            traces_endpoint("http://collector.invalid:4318"),
            "http://collector.invalid:4318/v1/traces"
        );
        assert_eq!(
            traces_endpoint("http://collector.invalid:4318/"),
            "http://collector.invalid:4318/v1/traces"
        );
        assert_eq!(
            traces_endpoint("http://collector.invalid:4318/custom"),
            "http://collector.invalid:4318/custom/v1/traces"
        );
        assert_eq!(
            traces_endpoint("http://collector.invalid:4318/collector/"),
            "http://collector.invalid:4318/collector/v1/traces"
        );
        assert_eq!(
            traces_endpoint("http://collector.invalid:4318"),
            format!("http://collector.invalid:4318{OTLP_TRACES_PATH}")
        );
        assert!(
            !traces_endpoint("http://collector.invalid:4318").contains("//v1/traces"),
            "trailing slash must not produce a double slash"
        );
    }
}
