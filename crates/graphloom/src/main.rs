//! `graphloom` binary entrypoint.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::{io, process::ExitCode, thread};

use graphloom::cli::{Cli, CliError, run};
use thiserror::Error;

// The Windows executable main thread has a smaller stack than the Unix runners.
// The CLI assembles a large debug indexing runtime, so the root future is executed
// on a deliberately sized thread rather than relying on the platform linker default.
const CLI_THREAD_STACK_SIZE: usize = 8 * 1024 * 1024;

#[derive(Debug, Error)]
enum CliEntryError {
    #[error("failed to build GraphLoom CLI runtime: {source}")]
    RuntimeBuild {
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Cli(#[from] CliError),
}

fn main() -> ExitCode {
    let handle = match thread::Builder::new()
        .name("graphloom-cli".to_owned())
        .stack_size(CLI_THREAD_STACK_SIZE)
        .spawn(run_cli)
    {
        Ok(handle) => handle,
        Err(source) => {
            eprintln!("failed to start GraphLoom CLI thread: {source}");
            return ExitCode::FAILURE;
        }
    };

    match handle.join() {
        Ok(Ok(())) => ExitCode::SUCCESS,
        Ok(Err(error)) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn run_cli() -> Result<(), CliEntryError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(CLI_THREAD_STACK_SIZE)
        .build()
        .map_err(|source| CliEntryError::RuntimeBuild { source })?;

    runtime.block_on(async {
        let cli = Cli::parse();
        run(cli).await.map_err(CliEntryError::from)
    })
}
