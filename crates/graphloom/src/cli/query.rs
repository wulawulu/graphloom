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
    write_non_streaming_response(&mut output, &result.response)?;
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
                write_stream_token(&mut output, &token)?;
            }
            QueryEvent::Completed(result) => log_completion(&result, method, true),
            QueryEvent::Context(_) => {}
        }
    }
    write_terminal_newline(&mut output)?;
    Ok(())
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
    use std::io::{Error, ErrorKind, Write};

    use super::{write_non_streaming_response, write_stream_token, write_terminal_newline};
    use crate::GraphLoomError;

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
}
