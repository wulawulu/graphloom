//! Command line adapter for `GraphLoom`.

pub mod args;
pub mod callbacks;
pub mod error;
pub mod index;
pub mod init;
pub mod prompt_tune;
pub mod query;
mod telemetry;
pub mod update;

pub use args::{
    Cli, Command, ExplainabilityContentArg, IndexArgs, IndexMethodArg, InitArgs, PromptLanguage,
    PromptTuneArgs, PromptTuneSelectionMethod, QueryArgs, UpdateArgs,
};
pub use error::{CliError, Result};
pub use index::run as run_index;
pub use init::init_project;
pub use prompt_tune::run as run_prompt_tune;
pub use query::run as run_query;
pub use update::run as run_update;

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
        Command::Update(args) => {
            run_update(&args).await?;
            Ok(())
        }
        Command::Query(args) => run_query(&args).await,
        Command::PromptTune(args) => run_prompt_tune(&args).await,
    }
}
