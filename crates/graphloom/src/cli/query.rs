//! Query CLI adapter.

use std::{io::Write, path::Path};

use futures_util::StreamExt;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::{
    GraphLoomError,
    api::query::{query_loaded, query_loaded_stream},
    cli::{
        QueryArgs,
        error::{CliError, Result},
    },
    config::load::load_project_config,
    query::{QueryEvent, QueryOptions, QueryResult, SearchMethod},
};

/// Execute `graphloom query`.
///
/// # Errors
///
/// Returns a typed Query/config/provider error or stdout I/O error.
pub async fn run(args: &QueryArgs) -> Result<()> {
    let project = load_project_config(&args.root).await?;
    let _log_guard = init_logging(&project.paths.reporting_dir, args.verbose).await?;
    let streaming = args.streaming_enabled();
    let mut options = QueryOptions::new(project.root.clone(), args.query.clone(), args.method);
    options.data_dir = args.data.clone();
    options.community_level = args.community_level;
    options.dynamic_community_selection = args.dynamic_selection_enabled();
    options.response_type.clone_from(&args.response_type);
    tracing::info!(method = %args.method, streaming, "query run started");
    let outcome = if streaming {
        run_streaming(project, options, args.method).await
    } else {
        run_non_streaming(project, options, args.method).await
    };
    if let Err(error) = &outcome {
        tracing::error!(
            method = %args.method,
            streaming,
            error_category = error_category(error),
            "query run failed"
        );
    }
    outcome
}

async fn run_non_streaming(
    project: crate::project::LoadedProject,
    options: QueryOptions,
    method: SearchMethod,
) -> Result<()> {
    let result = query_loaded(project, options).await?;
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    write_stdout(
        &mut output,
        result.response.as_bytes(),
        "write Query response",
    )?;
    write_stdout(&mut output, b"\n", "write Query terminal newline")?;
    output.flush().map_err(|source| CliError::Io {
        operation: "flush Query stdout",
        path: Path::new("<stdout>").to_path_buf(),
        source,
    })?;
    log_completion(&result, method, false);
    Ok(())
}

async fn run_streaming(
    project: crate::project::LoadedProject,
    options: QueryOptions,
    method: SearchMethod,
) -> Result<()> {
    let mut events = query_loaded_stream(project, options).await?;
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    while let Some(event) = events.next().await {
        match event? {
            QueryEvent::Token(token) => {
                write_stdout(
                    &mut output,
                    token.as_bytes(),
                    "write streaming Query response",
                )?;
                output.flush().map_err(|source| CliError::Io {
                    operation: "flush streaming Query response",
                    path: Path::new("<stdout>").to_path_buf(),
                    source,
                })?;
            }
            QueryEvent::Completed(result) => log_completion(&result, method, true),
            QueryEvent::Context(_) => {}
        }
    }
    write_stdout(&mut output, b"\n", "write Query terminal newline")?;
    output.flush().map_err(|source| CliError::Io {
        operation: "flush Query stdout",
        path: Path::new("<stdout>").to_path_buf(),
        source,
    })?;
    Ok(())
}

fn write_stdout(output: &mut impl Write, bytes: &[u8], operation: &'static str) -> Result<()> {
    output.write_all(bytes).map_err(|source| CliError::Io {
        operation,
        path: Path::new("<stdout>").to_path_buf(),
        source,
    })
}

fn log_completion(result: &QueryResult, method: SearchMethod, streaming: bool) {
    tracing::info!(
        method = %method,
        streaming,
        elapsed_ms = result.elapsed.as_millis(),
        llm_calls = result.usage.llm_calls,
        prompt_tokens = result.usage.prompt_tokens,
        output_tokens = result.usage.output_tokens,
        "query run completed"
    );
}

fn error_category(error: &GraphLoomError) -> &'static str {
    match error {
        GraphLoomError::Query(_) => "query",
        GraphLoomError::Io { .. } => "io",
        GraphLoomError::MissingSettings { .. }
        | GraphLoomError::ConfigParse { .. }
        | GraphLoomError::MissingEnvironmentVariable { .. }
        | GraphLoomError::InvalidModel { .. } => "configuration",
        _ => "runtime",
    }
}

async fn init_logging(
    reporting_dir: &Path,
    verbose: bool,
) -> Result<Option<tracing_appender::non_blocking::WorkerGuard>> {
    tokio::fs::create_dir_all(reporting_dir)
        .await
        .map_err(|source| CliError::Io {
            operation: "create Query log directory",
            path: reporting_dir.to_path_buf(),
            source,
        })?;
    let file_filter = if verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };
    let console_filter = if verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("off")
    };
    let appender = tracing_appender::rolling::never(reporting_dir, "query.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let console_layer = fmt::layer()
        .with_target(false)
        .with_writer(std::io::stderr)
        .with_filter(console_filter);
    let file_layer = fmt::layer()
        .with_target(true)
        .with_ansi(false)
        .with_writer(writer)
        .with_filter(file_filter);
    let subscriber = tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer);
    match tracing::subscriber::set_global_default(subscriber) {
        Ok(()) => Ok(Some(guard)),
        Err(source) => {
            drop(guard);
            Err(CliError::RuntimeBuild {
                source: Box::new(source),
            })
        }
    }
}
