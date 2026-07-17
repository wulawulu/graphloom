//! Query CLI adapter.

use std::{io::Write, path::Path};

use futures_util::StreamExt;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::{
    api::query::{query_loaded, query_loaded_stream},
    cli::{
        QueryArgs,
        error::{CliError, Result},
    },
    config::load::load_project_config,
    query::{QueryEvent, QueryOptions},
};

/// Execute `graphloom query`.
///
/// # Errors
///
/// Returns a typed Query/config/provider error or stdout I/O error.
pub async fn run(args: &QueryArgs) -> Result<()> {
    let project = load_project_config(&args.root).await?;
    let _log_guard = init_logging(&project.paths.reporting_dir, args.verbose).await?;
    let mut options = QueryOptions::new(project.root.clone(), args.query.clone(), args.method);
    options.data_dir = args.data.clone();
    options.community_level = args.community_level;
    options.dynamic_community_selection = args.dynamic_selection_enabled();
    options.response_type.clone_from(&args.response_type);
    tracing::info!(method = %args.method, streaming = args.streaming_enabled(), "query run started");
    if args.streaming_enabled() {
        let mut events = query_loaded_stream(project, options).await?;
        let stdout = std::io::stdout();
        let mut output = stdout.lock();
        while let Some(event) = events.next().await {
            match event? {
                QueryEvent::Token(token) => {
                    output
                        .write_all(token.as_bytes())
                        .and_then(|()| output.flush())
                        .map_err(|source| CliError::Io {
                            operation: "write streaming Query response",
                            path: Path::new("<stdout>").to_path_buf(),
                            source,
                        })?;
                }
                QueryEvent::Completed(result) => log_completion(&result),
                QueryEvent::Context(_) => {}
            }
        }
        output.write_all(b"\n").map_err(|source| CliError::Io {
            operation: "write Query terminal newline",
            path: Path::new("<stdout>").to_path_buf(),
            source,
        })?;
        output.flush().map_err(|source| CliError::Io {
            operation: "flush Query stdout",
            path: Path::new("<stdout>").to_path_buf(),
            source,
        })?;
    } else {
        let result = query_loaded(project, options).await?;
        println!("{}", result.response);
        log_completion(&result);
    }
    Ok(())
}

fn log_completion(result: &crate::query::QueryResult) {
    tracing::info!(
        elapsed_ms = result.elapsed.as_millis(),
        llm_calls = result.usage.llm_calls,
        prompt_tokens = result.usage.prompt_tokens,
        output_tokens = result.usage.output_tokens,
        "query run completed"
    );
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
