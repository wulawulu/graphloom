//! Command line adapter for `GraphLoom`.

pub mod args;
pub mod callbacks;
pub mod error;
pub mod index;
pub mod init;

pub use args::{Cli, Command, IndexArgs, IndexMethodArg, InitArgs};
pub use error::{CliError, Result};
pub use index::run as run_index;
pub use init::init_project;

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
