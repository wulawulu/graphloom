//! Command line argument definitions.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// `GraphLoom` command line.
#[derive(Debug, Parser)]
#[command(
    name = "graphloom",
    version,
    about = "GraphLoom indexing CLI",
    arg_required_else_help = true
)]
pub struct Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Command,
}

/// Supported top-level commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize a `GraphLoom` project.
    Init(InitArgs),
    /// Run standard indexing.
    Index(IndexArgs),
}

fn default_root() -> PathBuf {
    PathBuf::from(".")
}

/// `graphloom init` arguments.
#[derive(Debug, Clone, Parser)]
pub struct InitArgs {
    /// Project root directory.
    #[arg(short = 'r', long = "root", default_value_os_t = default_root())]
    pub root: PathBuf,
    /// Default completion model name.
    #[arg(short = 'm', long = "model", default_value = "gpt-4.1")]
    pub model: String,
    /// Default embedding model name.
    #[arg(
        short = 'e',
        long = "embedding",
        default_value = "text-embedding-3-large"
    )]
    pub embedding: String,
    /// Overwrite GraphLoom-managed files.
    #[arg(short = 'f', long = "force")]
    pub force: bool,
}

/// CLI indexing method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum IndexMethodArg {
    /// Standard full indexing pipeline.
    Standard,
}

/// `graphloom index` arguments.
#[derive(Debug, Clone, Parser)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "clap option structs intentionally mirror boolean command-line flags"
)]
pub struct IndexArgs {
    /// Project root directory.
    #[arg(short = 'r', long = "root", default_value_os_t = default_root())]
    pub root: PathBuf,
    /// Indexing method.
    #[arg(short = 'm', long = "method", default_value = "standard")]
    pub method: IndexMethodArg,
    /// Print more detailed progress.
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,
    /// Validate configuration and print a run plan without side effects.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    /// Use LLM cache for this run.
    #[arg(long = "cache", default_value_t = true, action = clap::ArgAction::SetTrue, overrides_with = "no_cache")]
    pub cache: bool,
    /// Disable LLM cache for this run.
    #[arg(long = "no-cache", action = clap::ArgAction::SetTrue)]
    pub no_cache: bool,
    /// Skip optional preflight checks.
    #[arg(long = "skip-validation")]
    pub skip_validation: bool,
}

impl IndexArgs {
    /// Return whether cache is enabled for this run.
    #[must_use]
    pub fn cache_enabled(&self) -> bool {
        self.cache && !self.no_cache
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Command};

    #[test]
    fn test_should_enable_configured_cache_by_default() {
        let cli = Cli::try_parse_from(["graphloom", "index"]).expect("CLI arguments");
        let Command::Index(args) = cli.command else {
            panic!("expected index command");
        };
        assert!(args.cache_enabled());
    }

    #[test]
    fn test_should_disable_cache_with_no_cache_flag() {
        let cli = Cli::try_parse_from(["graphloom", "index", "--no-cache"]).expect("CLI arguments");
        let Command::Index(args) = cli.command else {
            panic!("expected index command");
        };
        assert!(!args.cache_enabled());
    }
}
