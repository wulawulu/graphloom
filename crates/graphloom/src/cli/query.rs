//! Query CLI adapter.

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use futures_util::StreamExt;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::{
    GraphLoomError,
    api::query::{query_loaded, query_loaded_stream},
    cli::{
        ExplainabilityContentArg, QueryArgs,
        error::{CliError, Result},
        telemetry::{
            OtlpTraceGuard, OtlpTraceOptions, OtlpTraceRuntime, otel_layer, telemetry_error,
        },
    },
    config::load::load_project_config,
    explainability::{
        JsonlExplainabilityError, JsonlExplainabilityOptions, JsonlExplainabilityRecorder,
    },
    observability::{OBSERVABILITY_CONTRACT_VERSION, error_kind, event_name},
    query::{
        QueryEvent, QueryEventStream, QueryExplainabilityOptions, QueryOptions, QueryResult,
        SearchMethod,
        observability::{duration_millis, graphloom_error_kind, usize_to_u64},
    },
};

const QUERY_FILE_FILTER: &str = "off,graphloom::cli::query=info,graphloom::query=info";
const QUERY_VERBOSE_FILE_FILTER: &str = "off,graphloom::cli::query=debug,graphloom::query=debug";
const QUERY_VERBOSE_CONSOLE_FILTER: &str = "off,graphloom::cli::query=debug,graphloom::query=debug";

/// Execute `graphloom query`.
///
/// # Errors
///
/// Returns a typed Query/config/provider error or stdout I/O error.
pub async fn run(args: &QueryArgs) -> Result<()> {
    let project = load_project_config(&args.root).await?;
    run_query_with_observability(
        args,
        project,
        prepare_query_log_directory,
        OtlpTraceRuntime::build,
        |subscriber| {
            tracing::subscriber::set_global_default(subscriber)
                .map_err(|source| Box::new(source) as Box<dyn std::error::Error + Send + Sync>)
        },
        |args, project| async move { Box::pin(run_query_work(&args, project)).await },
    )
    .await
}

/// Run a Query with injectable observability orchestration.
///
/// Production [`run`] wires the real directory preparer, OTLP runtime builder,
/// global subscriber installer, and Query work; tests inject failing or
/// counting replacements to prove every initialization failure closes any
/// already-created OTLP runtime before returning.
///
/// The Query log directory is prepared *before* any OTLP runtime is built, so
/// a directory failure cannot leak a provider or batch worker.
async fn run_query_with_observability<Prepare, PrepareFut, BuildRuntime, Install, Work, WorkFut>(
    args: &QueryArgs,
    project: crate::project::LoadedProject,
    prepare_directory: Prepare,
    build_runtime: BuildRuntime,
    install_subscriber: Install,
    run_work: Work,
) -> Result<()>
where
    Prepare: FnOnce(PathBuf) -> PrepareFut,
    PrepareFut: std::future::Future<Output = Result<()>>,
    BuildRuntime: FnOnce(&OtlpTraceOptions) -> Result<OtlpTraceRuntime>,
    Install: FnOnce(
        Box<dyn tracing::Subscriber + Send + Sync>,
    ) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>>,
    Work: FnOnce(QueryArgs, crate::project::LoadedProject) -> WorkFut,
    WorkFut: std::future::Future<Output = Result<()>>,
{
    prepare_directory(project.paths.reporting_dir.clone()).await?;
    let otlp_runtime = OtlpTraceOptions::from_args(args)
        .map(|options| build_runtime(&options))
        .transpose()?;
    let mut observability = init_query_observability_prepared_with(
        &project.paths.reporting_dir,
        args.verbose,
        otlp_runtime,
        install_subscriber,
    )
    .await?;
    let work_outcome = Box::pin(run_work(args.clone(), project)).await;
    let telemetry_outcome = observability.shutdown_telemetry().await;
    let outcome = combine_work_and_telemetry_outcomes(work_outcome, telemetry_outcome);
    drop(observability);
    outcome
}

async fn run_query_work(args: &QueryArgs, project: crate::project::LoadedProject) -> Result<()> {
    let streaming = args.streaming_enabled();
    let mut options = QueryOptions::new(project.root.clone(), args.query.clone(), args.method);
    options.data_dir = args.data.clone();
    options.community_level = args.community_level;
    options.dynamic_community_selection = args.dynamic_selection_enabled();
    options.response_type.clone_from(&args.response_type);
    let recorder = create_explainability_recorder(args).await?;
    if let Some(recorder) = recorder.as_ref() {
        let content_mode = args
            .explain_content
            .unwrap_or(ExplainabilityContentArg::Metadata)
            .into();
        let explainability = QueryExplainabilityOptions::generated(content_mode, recorder.sink());
        tracing::info!(
            name: event_name::CLI_EXPLAINABILITY_ENABLED,
            {
                "graphloom.run.id" = %explainability.run_id(),
                "graphloom.explainability.enabled" = true,
            },
            "Local Query Explainability JSONL enabled"
        );
        options.explainability = Some(explainability);
    }
    match options
        .explainability
        .as_ref()
        .map(QueryExplainabilityOptions::run_id)
    {
        Some(run_id) => {
            tracing::info!(
                name: event_name::CLI_QUERY_STARTED,
                {
                    "graphloom.observability.version" = OBSERVABILITY_CONTRACT_VERSION,
                    "graphloom.query.method" = %args.method,
                    "graphloom.query.streaming" = streaming,
                    "graphloom.explainability.enabled" = options.explainability.is_some(),
                    "graphloom.run.id" = %run_id,
                },
                "query run started"
            );
        }
        None => {
            tracing::info!(
                name: event_name::CLI_QUERY_STARTED,
                {
                    "graphloom.observability.version" = OBSERVABILITY_CONTRACT_VERSION,
                    "graphloom.query.method" = %args.method,
                    "graphloom.query.streaming" = streaming,
                    "graphloom.explainability.enabled" = options.explainability.is_some(),
                    "graphloom.run.id" = tracing::field::Empty,
                },
                "query run started"
            );
        }
    }
    let query_outcome = if streaming {
        run_streaming(project, options, args.method).await
    } else {
        run_non_streaming(project, options, args.method).await
    };
    if let Err(error) = &query_outcome {
        emit_query_failed(args.method, streaming, error);
    }
    let recorder_outcome = shutdown_explainability_recorder(recorder).await;
    combine_query_and_recorder_outcomes(query_outcome, recorder_outcome)
}

async fn create_explainability_recorder(
    args: &QueryArgs,
) -> Result<Option<JsonlExplainabilityRecorder>> {
    let Some(path) = args.explain_output.as_ref() else {
        return Ok(None);
    };
    JsonlExplainabilityRecorder::create(JsonlExplainabilityOptions::new(path.clone()))
        .await
        .map(Some)
        .map_err(|source| {
            explainability_output_error("create Explainability JSONL output", path, source)
        })
}

async fn shutdown_explainability_recorder(
    recorder: Option<JsonlExplainabilityRecorder>,
) -> Result<()> {
    let Some(recorder) = recorder else {
        return Ok(());
    };
    let path = recorder.path().to_path_buf();
    recorder.shutdown().await.map_err(|source| {
        explainability_output_error("shutdown Explainability JSONL output", &path, source)
    })
}

fn combine_query_and_recorder_outcomes(query: Result<()>, recorder: Result<()>) -> Result<()> {
    if recorder.is_err() {
        tracing::error!(
            name: event_name::CLI_EXPLAINABILITY_SHUTDOWN_FAILED,
            {
                "graphloom.observability.version" = OBSERVABILITY_CONTRACT_VERSION,
                "graphloom.error.kind" = "explainability_output",
            },
            "Explainability Recorder shutdown failed"
        );
    }
    match (query, recorder) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(recorder_error)) => Err(recorder_error),
        (Err(query_error), _) => Err(query_error),
    }
}

fn combine_work_and_telemetry_outcomes(work: Result<()>, telemetry: Result<()>) -> Result<()> {
    if telemetry.is_err() {
        tracing::error!(
            name: event_name::CLI_TELEMETRY_SHUTDOWN_FAILED,
            {
                "graphloom.observability.version" = OBSERVABILITY_CONTRACT_VERSION,
                "graphloom.error.kind" = error_kind::TELEMETRY_OUTPUT,
            },
            "OpenTelemetry trace export shutdown failed"
        );
    }
    match (work, telemetry) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(telemetry_error)) => Err(telemetry_error),
        (Err(work_error), _) => Err(work_error),
    }
}

fn emit_query_failed(method: SearchMethod, streaming: bool, error: &GraphLoomError) {
    tracing::error!(
        name: event_name::CLI_QUERY_FAILED,
        {
            "graphloom.query.method" = %method,
            "graphloom.query.streaming" = streaming,
            "graphloom.error.kind" = graphloom_error_kind(error),
        },
        "query run failed"
    );
}

fn explainability_output_error(
    operation: &'static str,
    path: &Path,
    source: JsonlExplainabilityError,
) -> GraphLoomError {
    GraphLoomError::ExplainabilityOutput {
        operation,
        path: path.to_path_buf(),
        source: Box::new(source),
    }
}

async fn run_non_streaming(
    project: crate::project::LoadedProject,
    options: QueryOptions,
    method: SearchMethod,
) -> Result<()> {
    let result = query_loaded(project, options).await?;
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    write_non_streaming_response(&mut output, &result.response)?;
    log_completion(&QueryCompletionMetrics::from_result(&result), method, false);
    Ok(())
}

async fn run_streaming(
    project: crate::project::LoadedProject,
    options: QueryOptions,
    method: SearchMethod,
) -> Result<()> {
    let events = query_loaded_stream(project, options).await?;
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    consume_stream_to_output(events, &mut output, method).await
}

async fn consume_stream_to_output(
    events: QueryEventStream,
    output: &mut impl Write,
    method: SearchMethod,
) -> Result<()> {
    let mut events = events;
    let mut completion_metrics = None;
    while let Some(event) = events.next().await {
        match event? {
            QueryEvent::Token(token) => write_stream_token(output, &token)?,
            QueryEvent::Completed(result) => {
                completion_metrics = Some(QueryCompletionMetrics::from_result(&result));
            }
            QueryEvent::Context(_) => {}
        }
    }
    write_terminal_newline(output)?;
    if let Some(metrics) = completion_metrics {
        log_completion(&metrics, method, true);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct QueryCompletionMetrics {
    elapsed_ms: Option<u64>,
    llm_calls: Option<u64>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

impl QueryCompletionMetrics {
    fn from_result(result: &QueryResult) -> Self {
        Self {
            elapsed_ms: duration_millis(result.elapsed),
            llm_calls: usize_to_u64(result.usage.llm_calls),
            input_tokens: usize_to_u64(result.usage.prompt_tokens),
            output_tokens: usize_to_u64(result.usage.output_tokens),
        }
    }
}

fn write_non_streaming_response(output: &mut impl Write, response: &str) -> Result<()> {
    write_stdout(output, response.as_bytes(), "write Query response")?;
    write_terminal_newline(output)
}

fn write_stream_token(output: &mut impl Write, token: &str) -> Result<()> {
    write_stdout(output, token.as_bytes(), "write streaming Query response")?;
    flush_stdout(output, "flush streaming Query response")
}

fn write_terminal_newline(output: &mut impl Write) -> Result<()> {
    write_stdout(output, b"\n", "write Query terminal newline")?;
    flush_stdout(output, "flush Query stdout")
}

fn write_stdout(output: &mut impl Write, bytes: &[u8], operation: &'static str) -> Result<()> {
    output.write_all(bytes).map_err(|source| CliError::Io {
        operation,
        path: Path::new("<stdout>").to_path_buf(),
        source,
    })
}

fn flush_stdout(output: &mut impl Write, operation: &'static str) -> Result<()> {
    output.flush().map_err(|source| CliError::Io {
        operation,
        path: Path::new("<stdout>").to_path_buf(),
        source,
    })
}

fn log_completion(metrics: &QueryCompletionMetrics, method: SearchMethod, streaming: bool) {
    match (
        metrics.elapsed_ms,
        metrics.llm_calls,
        metrics.input_tokens,
        metrics.output_tokens,
    ) {
        (Some(elapsed_ms), Some(llm_calls), Some(input_tokens), Some(output_tokens)) => {
            tracing::info!(
                name: event_name::CLI_QUERY_COMPLETED,
                {
                    "graphloom.query.method" = %method,
                    "graphloom.query.streaming" = streaming,
                    "graphloom.elapsed_ms" = elapsed_ms,
                    "graphloom.llm.calls" = llm_calls,
                    "graphloom.input.tokens" = input_tokens,
                    "graphloom.output.tokens" = output_tokens,
                },
                "query run completed"
            );
        }
        _ => {
            tracing::info!(
                name: event_name::CLI_QUERY_COMPLETED,
                {
                    "graphloom.query.method" = %method,
                    "graphloom.query.streaming" = streaming,
                },
                "query run completed"
            );
        }
    }
}

/// Combined Query observability guard: log writer plus optional OTLP shutdown.
#[derive(Debug)]
#[must_use]
pub(crate) struct QueryObservabilityGuard {
    // Held only for RAII: keeps the non-blocking query.log writer alive until
    // after the telemetry shutdown event has been emitted.
    _log_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
    otlp_guard: Option<OtlpTraceGuard>,
}

impl QueryObservabilityGuard {
    /// Explicitly flush and shut down the OTLP provider, if any.
    ///
    /// The log writer guard stays alive until the caller drops this struct so
    /// telemetry failure events are still written to `query.log`.
    async fn shutdown_telemetry(&mut self) -> Result<()> {
        let Some(guard) = self.otlp_guard.take() else {
            return Ok(());
        };
        guard.shutdown().await
    }
}

/// Create the Query log directory before any OTLP runtime is built.
///
/// Takes an owned path so the returned future never borrows the caller's
/// reference, which keeps the function injectable into the generic
/// observability orchestrator.
///
/// # Errors
///
/// Returns the stable `create Query log directory` I/O error when the
/// reporting directory cannot be created.
async fn prepare_query_log_directory(reporting_dir: PathBuf) -> Result<()> {
    prepare_query_log_directory_with(&reporting_dir, |path| {
        let path = path.to_path_buf();
        async move { tokio::fs::create_dir_all(&path).await }
    })
    .await
}

/// Create the Query log directory through an injected creator.
///
/// Keeps the stable error mapping testable without platform permission
/// differences.
///
/// # Errors
///
/// Returns the stable `create Query log directory` I/O error when the
/// injected creator fails.
async fn prepare_query_log_directory_with<F, Fut>(reporting_dir: &Path, create_dir: F) -> Result<()>
where
    F: FnOnce(&Path) -> Fut,
    Fut: std::future::Future<Output = std::io::Result<()>>,
{
    create_dir(reporting_dir)
        .await
        .map_err(|source| CliError::Io {
            operation: "create Query log directory",
            path: reporting_dir.to_path_buf(),
            source,
        })
}

/// Build the Query subscriber stack for an already prepared log directory.
///
/// The caller must have already created `reporting_dir` successfully (see
/// [`prepare_query_log_directory`]) before building any OTLP runtime. This
/// function never creates directories. The only fallible step after an OTLP
/// runtime is built is the injected subscriber install; on failure the
/// provider is explicitly shut down on a blocking thread before the install
/// error is returned.
///
/// # Errors
///
/// Returns a safe `install Query tracing subscriber` telemetry error when the
/// injected subscriber installer fails.
async fn init_query_observability_prepared_with<F>(
    reporting_dir: &Path,
    verbose: bool,
    otlp_runtime: Option<OtlpTraceRuntime>,
    install_subscriber: F,
) -> Result<QueryObservabilityGuard>
where
    F: FnOnce(
        Box<dyn tracing::Subscriber + Send + Sync>,
    ) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>>,
{
    let file_filter = query_file_filter(verbose);
    let console_filter = query_console_filter(verbose);
    let appender = tracing_appender::rolling::never(reporting_dir, "query.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let console_layer = fmt::layer()
        .with_target(false)
        .with_writer(std::io::stderr)
        .with_filter(console_filter);
    let file_layer = fmt::layer()
        .with_target(true)
        .with_ansi(false)
        .with_span_events(fmt::format::FmtSpan::NEW | fmt::format::FmtSpan::CLOSE)
        .with_writer(writer)
        .with_filter(file_filter);
    let otel_layer = otlp_runtime
        .as_ref()
        .map(|runtime| otel_layer(runtime.tracer().clone()));
    let subscriber: Box<dyn tracing::Subscriber + Send + Sync> = Box::new(
        tracing_subscriber::registry()
            .with(console_layer)
            .with(file_layer)
            .with(otel_layer),
    );
    let otlp_enabled = otlp_runtime.is_some();
    match install_subscriber(subscriber) {
        Ok(()) => {
            if otlp_enabled {
                tracing::info!(
                    name: event_name::CLI_TELEMETRY_ENABLED,
                    {
                        "graphloom.observability.version" = OBSERVABILITY_CONTRACT_VERSION,
                        "graphloom.telemetry.enabled" = true,
                    },
                    "OpenTelemetry trace export enabled"
                );
            }
            Ok(QueryObservabilityGuard {
                _log_guard: Some(guard),
                otlp_guard: otlp_runtime.map(OtlpTraceRuntime::into_guard),
            })
        }
        Err(source) => {
            drop(guard);
            if let Some(runtime) = otlp_runtime {
                runtime.shutdown_after_init_failure().await;
            }
            Err(telemetry_error("install Query tracing subscriber", source))
        }
    }
}

fn query_file_filter(verbose: bool) -> EnvFilter {
    EnvFilter::new(if verbose {
        QUERY_VERBOSE_FILE_FILTER
    } else {
        QUERY_FILE_FILTER
    })
}

fn query_console_filter(verbose: bool) -> EnvFilter {
    EnvFilter::new(if verbose {
        QUERY_VERBOSE_CONSOLE_FILTER
    } else {
        "off"
    })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        io::{Error, ErrorKind, Write},
        path::{Path, PathBuf},
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use opentelemetry_sdk::{
        error::OTelSdkError,
        trace::{SdkTracerProvider, SpanData, SpanProcessor},
    };

    use super::{
        combine_query_and_recorder_outcomes, combine_work_and_telemetry_outcomes,
        consume_stream_to_output, emit_query_failed, prepare_query_log_directory,
        prepare_query_log_directory_with, run_query_with_observability,
        write_non_streaming_response, write_stream_token, write_terminal_newline,
    };
    use crate::{
        GraphLoomError, GraphRagConfig,
        cli::{
            Cli, Command, QueryArgs,
            telemetry::{OtlpTraceRuntime, telemetry_error},
        },
        observability::event_name as contract_event_name,
        project::LoadedProject,
        query::{
            QueryContext, QueryEvent, QueryEventStream, QueryResult, QueryUsage, SearchMethod,
        },
        test_support::tracing_capture,
    };

    fn query_result() -> QueryResult {
        QueryResult {
            response: "answer".to_owned(),
            context: QueryContext::default(),
            elapsed: Duration::from_millis(1),
            usage: QueryUsage {
                llm_calls: 1,
                prompt_tokens: 2,
                output_tokens: 3,
                categories: BTreeMap::new(),
            },
        }
    }

    fn event_count(state: &tracing_capture::CaptureState, name: &str) -> usize {
        state
            .events
            .iter()
            .filter(|event| event.name == name)
            .count()
    }

    fn io_error(operation: &'static str) -> GraphLoomError {
        GraphLoomError::Io {
            operation,
            path: std::path::PathBuf::from("<stdout>"),
            source: Error::new(ErrorKind::BrokenPipe, "forced"),
        }
    }

    fn telemetry_error_for_test() -> GraphLoomError {
        telemetry_error(
            "flush OTLP traces",
            Error::other("forced telemetry failure"),
        )
    }

    #[derive(Debug, Default)]
    struct AlwaysFailWriter;

    impl Write for AlwaysFailWriter {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(Error::new(ErrorKind::BrokenPipe, "forced write failure"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(Error::new(ErrorKind::BrokenPipe, "forced flush failure"))
        }
    }

    #[derive(Debug, Default)]
    struct FlushFailWriter {
        bytes: Vec<u8>,
    }

    impl Write for FlushFailWriter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.bytes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(Error::new(ErrorKind::BrokenPipe, "forced flush failure"))
        }
    }

    #[derive(Debug, Default)]
    struct SecondWriteFailWriter {
        writes: usize,
    }

    impl Write for SecondWriteFailWriter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.writes = self.writes.saturating_add(1);
            if self.writes == 2 {
                return Err(Error::new(
                    ErrorKind::BrokenPipe,
                    "forced terminal newline failure",
                ));
            }
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn assert_io_operation(error: GraphLoomError, expected: &'static str) {
        let GraphLoomError::Io {
            operation, path, ..
        } = error
        else {
            panic!("expected stdout I/O error");
        };
        assert_eq!(operation, expected);
        assert_eq!(path, std::path::Path::new("<stdout>"));
    }

    #[test]
    fn test_should_map_non_stream_writer_failure() {
        let error = write_non_streaming_response(&mut AlwaysFailWriter, "answer")
            .expect_err("non-stream write must fail");
        assert_io_operation(error, "write Query response");
    }

    #[test]
    fn test_should_map_stream_writer_failure() {
        let error =
            write_stream_token(&mut AlwaysFailWriter, "chunk").expect_err("stream write must fail");
        assert_io_operation(error, "write streaming Query response");
    }

    #[test]
    fn test_should_map_stdout_flush_and_terminal_newline_failures() {
        let error =
            write_stream_token(&mut FlushFailWriter::default(), "chunk").expect_err("flush");
        assert_io_operation(error, "flush streaming Query response");

        let error = write_non_streaming_response(&mut SecondWriteFailWriter::default(), "answer")
            .expect_err("terminal newline");
        assert_io_operation(error, "write Query terminal newline");

        let error =
            write_terminal_newline(&mut FlushFailWriter::default()).expect_err("terminal flush");
        assert_io_operation(error, "flush Query stdout");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_should_emit_completed_after_stream_output_succeeds() {
        let state = Arc::new(Mutex::new(tracing_capture::CaptureState::default()));
        let subscriber = tracing_capture::capture_subscriber(state.clone());
        let _guard = tracing::subscriber::set_default(subscriber);

        let events: Vec<crate::query::Result<QueryEvent>> = vec![
            Ok(QueryEvent::Token("chunk".to_owned())),
            Ok(QueryEvent::Completed(query_result())),
        ];
        let stream: QueryEventStream = Box::pin(futures_util::stream::iter(events));
        let mut writer = Vec::new();

        consume_stream_to_output(stream, &mut writer, SearchMethod::Local)
            .await
            .expect("stream output");

        assert_eq!(writer, b"chunk\n");
        let state = state.lock().expect("capture state");
        assert_eq!(
            event_count(&state, contract_event_name::CLI_QUERY_COMPLETED),
            1
        );
        assert_eq!(
            event_count(&state, contract_event_name::CLI_QUERY_FAILED),
            0
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_should_not_emit_completed_when_stream_stdout_fails() {
        let state = Arc::new(Mutex::new(tracing_capture::CaptureState::default()));
        let subscriber = tracing_capture::capture_subscriber(state.clone());
        let _guard = tracing::subscriber::set_default(subscriber);

        let events: Vec<crate::query::Result<QueryEvent>> = vec![
            Ok(QueryEvent::Token("chunk".to_owned())),
            Ok(QueryEvent::Completed(query_result())),
        ];
        let stream: QueryEventStream = Box::pin(futures_util::stream::iter(events));
        let mut writer = SecondWriteFailWriter::default();

        let error = consume_stream_to_output(stream, &mut writer, SearchMethod::Local)
            .await
            .expect_err("terminal newline write must fail");
        let GraphLoomError::Io {
            operation, path, ..
        } = &error
        else {
            panic!("expected stdout I/O error");
        };
        assert_eq!(*operation, "write Query terminal newline");
        assert_eq!(*path, std::path::Path::new("<stdout>"));
        emit_query_failed(SearchMethod::Local, true, &error);

        let state = state.lock().expect("capture state");
        assert_eq!(
            event_count(&state, contract_event_name::CLI_QUERY_COMPLETED),
            0
        );
        assert_eq!(
            event_count(&state, contract_event_name::CLI_QUERY_FAILED),
            1
        );
        let failed = state
            .events
            .iter()
            .find(|event| event.name == contract_event_name::CLI_QUERY_FAILED)
            .expect("failed event");
        assert_eq!(failed.field("graphloom.query.method"), Some("local"));
        assert_eq!(failed.field("graphloom.query.streaming"), Some("true"));
    }

    #[test]
    fn test_should_emit_shutdown_failed_for_every_recorder_failure() {
        let state = Arc::new(Mutex::new(tracing_capture::CaptureState::default()));
        let subscriber = tracing_capture::capture_subscriber(state.clone());
        let _guard = tracing::subscriber::set_default(subscriber);

        let query_error_a = io_error("write Query response");
        let query_error_b = io_error("write Query response");
        let recorder_error_a = io_error("shutdown Explainability JSONL output");
        let recorder_error_b = io_error("shutdown Explainability JSONL output");

        let success_failure = combine_query_and_recorder_outcomes(Ok(()), Err(recorder_error_a));
        assert!(matches!(
            success_failure,
            Err(GraphLoomError::Io { operation, .. })
                if operation == "shutdown Explainability JSONL output"
        ));

        let failure_failure =
            combine_query_and_recorder_outcomes(Err(query_error_a), Err(recorder_error_b));
        assert!(matches!(
            failure_failure,
            Err(GraphLoomError::Io { operation, .. }) if operation == "write Query response"
        ));

        let failure_success = combine_query_and_recorder_outcomes(Err(query_error_b), Ok(()));
        assert!(matches!(
            failure_success,
            Err(GraphLoomError::Io { operation, .. })
                if operation == "write Query response"
        ));
        let both_ok = combine_query_and_recorder_outcomes(Ok(()), Ok(()));
        assert!(both_ok.is_ok());

        let state = state.lock().expect("capture state");
        let shutdown_events = state
            .events
            .iter()
            .filter(|event| event.name == contract_event_name::CLI_EXPLAINABILITY_SHUTDOWN_FAILED)
            .collect::<Vec<_>>();
        assert_eq!(shutdown_events.len(), 2);
        for event in &shutdown_events {
            assert_eq!(event.field("graphloom.observability.version"), Some("1"));
            assert_eq!(
                event.field("graphloom.error.kind"),
                Some("\"explainability_output\"")
            );
            assert!(
                event
                    .fields
                    .iter()
                    .all(|(field, value)| !field.contains("path")
                        && !value.contains("stdout")
                        && !value.contains("shutdown Explainability JSONL output"))
            );
        }
        assert_eq!(
            event_count(&state, contract_event_name::CLI_QUERY_FAILED),
            0
        );
    }

    #[test]
    fn test_should_combine_query_recorder_telemetry_outcomes_by_priority() {
        for (query_operation, recorder_operation, telemetry_failed) in [
            (None, None, false),
            (None, None, true),
            (None, Some("shutdown Explainability JSONL output"), false),
            (None, Some("shutdown Explainability JSONL output"), true),
            (Some("write Query response"), None, false),
            (Some("write Query response"), None, true),
            (
                Some("write Query response"),
                Some("shutdown Explainability JSONL output"),
                false,
            ),
            (
                Some("write Query response"),
                Some("shutdown Explainability JSONL output"),
                true,
            ),
        ] {
            let query = match query_operation {
                Some(operation) => Err(io_error(operation)),
                None => Ok(()),
            };
            let recorder = match recorder_operation {
                Some(operation) => Err(io_error(operation)),
                None => Ok(()),
            };
            let telemetry = if telemetry_failed {
                Err(telemetry_error_for_test())
            } else {
                Ok(())
            };
            let state = Arc::new(Mutex::new(tracing_capture::CaptureState::default()));
            let subscriber = tracing_capture::capture_subscriber(state.clone());
            let _guard = tracing::subscriber::set_default(subscriber);

            let work = combine_query_and_recorder_outcomes(query, recorder);
            let outcome = combine_work_and_telemetry_outcomes(work, telemetry);
            let expected_operation =
                match (query_operation.or(recorder_operation), telemetry_failed) {
                    (Some(operation), _) => Some(operation),
                    (None, true) => Some("flush OTLP traces"),
                    (None, false) => None,
                };
            match expected_operation {
                Some(operation) => {
                    let Err(error) = outcome else {
                        panic!("expected error for {operation:?}");
                    };
                    let actual_operation = match error {
                        GraphLoomError::Io {
                            operation: actual, ..
                        }
                        | GraphLoomError::Telemetry {
                            operation: actual, ..
                        } => actual,
                        other => panic!("unexpected error variant: {other:?}"),
                    };
                    assert_eq!(actual_operation, operation);
                }
                None => {
                    assert!(outcome.is_ok());
                }
            }

            let state = state.lock().expect("capture state");
            let recorder_events = state
                .events
                .iter()
                .filter(|event| {
                    event.name == contract_event_name::CLI_EXPLAINABILITY_SHUTDOWN_FAILED
                })
                .count();
            let telemetry_events = state
                .events
                .iter()
                .filter(|event| event.name == contract_event_name::CLI_TELEMETRY_SHUTDOWN_FAILED)
                .count();
            assert_eq!(
                recorder_events,
                usize::from(recorder_operation.is_some()),
                "recorder failure event must be emitted exactly once per failure"
            );
            assert_eq!(
                telemetry_events,
                usize::from(telemetry_failed),
                "telemetry failure event must be emitted exactly once per failure"
            );
        }
    }

    #[derive(Debug)]
    struct CountingShutdownProcessor {
        shutdowns: Arc<AtomicUsize>,
    }

    impl SpanProcessor for CountingShutdownProcessor {
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
            self.shutdowns.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_should_shutdown_provider_after_subscriber_install_failure_cleanup() {
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let provider = SdkTracerProvider::builder()
            .with_span_processor(CountingShutdownProcessor {
                shutdowns: Arc::clone(&shutdowns),
            })
            .build();
        let runtime = OtlpTraceRuntime::from_provider_for_test(provider, Duration::from_secs(1));

        runtime.shutdown_after_init_failure().await;

        assert_eq!(
            shutdowns.load(Ordering::SeqCst),
            1,
            "provider must be explicitly shut down after subscriber install failure"
        );
    }

    fn local_otel_args() -> QueryArgs {
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
        args
    }

    fn loaded_project(root: &Path) -> LoadedProject {
        LoadedProject::from_config(root, GraphRagConfig::default()).expect("loaded project")
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_should_map_log_directory_preparation_failure_to_io_error() {
        let error = prepare_query_log_directory_with(Path::new("reports"), |_path| async move {
            Err(Error::new(
                ErrorKind::PermissionDenied,
                "forced directory failure",
            ))
        })
        .await
        .expect_err("directory preparation must fail");
        assert!(matches!(
            &error,
            GraphLoomError::Io {
                operation: "create Query log directory",
                path,
                ..
            } if path == Path::new("reports")
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_should_not_build_runtime_or_run_query_when_directory_preparation_fails() {
        let directory = tempfile::tempdir().expect("project directory");
        let project = loaded_project(directory.path());
        let args = local_otel_args();
        let runtime_builds = Arc::new(AtomicUsize::new(0));
        let installs = Arc::new(AtomicUsize::new(0));
        let runtime_builds_for_closure = Arc::clone(&runtime_builds);
        let installs_for_closure = Arc::clone(&installs);

        let error = run_query_with_observability(
            &args,
            project,
            |_path| async move {
                Err(GraphLoomError::Io {
                    operation: "create Query log directory",
                    path: PathBuf::from("reports"),
                    source: Error::other("forced directory failure"),
                })
            },
            |_options| {
                runtime_builds_for_closure.fetch_add(1, Ordering::SeqCst);
                panic!("OTLP runtime must not be built when directory preparation fails");
            },
            |_subscriber| {
                installs_for_closure.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            |_args, _project| async move {
                panic!("Query work must not run when directory preparation fails");
            },
        )
        .await
        .expect_err("directory preparation must fail before OTLP runtime creation");

        assert!(matches!(
            &error,
            GraphLoomError::Io {
                operation: "create Query log directory",
                ..
            }
        ));
        assert_eq!(runtime_builds.load(Ordering::SeqCst), 0);
        assert_eq!(installs.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_should_shutdown_provider_when_subscriber_install_fails_in_orchestration() {
        let directory = tempfile::tempdir().expect("project directory");
        let project = loaded_project(directory.path());
        let args = local_otel_args();
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let installs = Arc::new(AtomicUsize::new(0));
        let provider = SdkTracerProvider::builder()
            .with_span_processor(CountingShutdownProcessor {
                shutdowns: Arc::clone(&shutdowns),
            })
            .build();
        let runtime = OtlpTraceRuntime::from_provider_for_test(provider, Duration::from_secs(1));
        let installs_for_closure = Arc::clone(&installs);
        let state = Arc::new(Mutex::new(tracing_capture::CaptureState::default()));
        let capture = tracing_capture::capture_subscriber(state.clone());
        let _guard = tracing::subscriber::set_default(capture);

        let error = run_query_with_observability(
            &args,
            project,
            prepare_query_log_directory,
            move |_options| Ok(runtime),
            move |_subscriber| {
                installs_for_closure.fetch_add(1, Ordering::SeqCst);
                Err(Box::new(Error::other("forced subscriber install failure")))
            },
            |_args, _project| async move {
                panic!("Query work must not run when subscriber install fails");
            },
        )
        .await
        .expect_err("subscriber install must fail");

        assert!(matches!(
            &error,
            GraphLoomError::Telemetry {
                operation: "install Query tracing subscriber",
                ..
            }
        ));
        assert_eq!(
            error.to_string(),
            "failed to install Query tracing subscriber for OpenTelemetry trace export"
        );
        assert_eq!(installs.load(Ordering::SeqCst), 1);
        assert_eq!(
            shutdowns.load(Ordering::SeqCst),
            1,
            "provider must be explicitly shut down after subscriber install failure"
        );
        let state = state.lock().expect("capture state");
        assert_eq!(
            state
                .events
                .iter()
                .filter(|event| {
                    event.name == contract_event_name::CLI_TELEMETRY_SHUTDOWN_FAILED
                })
                .count(),
            0,
            "install failure must not emit the telemetry shutdown named event"
        );
    }
}
