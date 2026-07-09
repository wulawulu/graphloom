//! Library API for the `graphloom` command line interface.

#![forbid(unsafe_code)]
#![allow(
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    reason = "CLI orchestration code validates external project configuration and keeps command \
              boundaries explicit"
)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

mod args;
mod callbacks;
mod config;
mod error;
mod index;
mod init;
mod project;
mod runtime;

pub use args::{Cli, Command, IndexArgs, IndexMethod, InitArgs};
pub use config::load_project_config;
pub use error::{CliError, Result};
pub use index::{IndexRunResult, run_index};
pub use init::init_project;
pub use project::{LoadedProject, ProjectPaths};

/// Run a parsed CLI command.
///
/// # Errors
///
/// Returns a command or configuration error when the selected command fails.
pub async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Init(args) => init_project(&args).await,
        Command::Index(args) => {
            run_index(&args).await?;
            Ok(())
        }
    }
}
