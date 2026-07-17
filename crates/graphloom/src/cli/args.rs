//! Command line argument definitions.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::query::SearchMethod;

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
    /// Query an existing GraphRAG-compatible index.
    Query(QueryArgs),
}

/// `graphloom query` arguments.
#[derive(Debug, Clone, Parser)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "clap option structs intentionally mirror boolean command-line flags"
)]
pub struct QueryArgs {
    /// Project root directory.
    #[arg(short = 'r', long = "root", default_value_os_t = default_root())]
    pub root: PathBuf,
    /// Query algorithm.
    #[arg(short = 'm', long = "method", default_value = "global")]
    pub method: SearchMethod,
    /// Enable verbose Query logging on stderr.
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,
    /// Override only the Parquet table directory.
    #[arg(short = 'd', long = "data")]
    pub data: Option<PathBuf>,
    /// Maximum Leiden hierarchy level.
    #[arg(long = "community-level", default_value_t = 2)]
    pub community_level: i64,
    /// Enable dynamic Global Search community selection.
    #[arg(long = "dynamic-community-selection", action = clap::ArgAction::SetTrue, overrides_with = "no_dynamic_selection")]
    pub dynamic_community_selection: bool,
    /// Disable dynamic Global Search community selection.
    #[arg(long = "no-dynamic-selection", action = clap::ArgAction::SetTrue)]
    pub no_dynamic_selection: bool,
    /// Desired response format.
    #[arg(long = "response-type", default_value = "Multiple Paragraphs")]
    pub response_type: String,
    /// Stream provider deltas to stdout.
    #[arg(long = "streaming", action = clap::ArgAction::SetTrue, overrides_with = "no_streaming")]
    pub streaming: bool,
    /// Disable streaming output.
    #[arg(long = "no-streaming", action = clap::ArgAction::SetTrue)]
    pub no_streaming: bool,
    /// Query text.
    pub query: String,
}

impl QueryArgs {
    /// Return the effective dynamic-selection flag.
    #[must_use]
    pub fn dynamic_selection_enabled(&self) -> bool {
        self.dynamic_community_selection && !self.no_dynamic_selection
    }

    /// Return the effective streaming flag.
    #[must_use]
    pub fn streaming_enabled(&self) -> bool {
        self.streaming && !self.no_streaming
    }
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
    /// Validate non-destructive indexing prerequisites and exit before runtime preparation.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    /// Use LLM cache for this run.
    #[arg(long = "cache", default_value_t = true, action = clap::ArgAction::SetTrue, overrides_with = "no_cache")]
    pub cache: bool,
    /// Disable LLM cache for this run.
    #[arg(long = "no-cache", action = clap::ArgAction::SetTrue)]
    pub no_cache: bool,
    /// Skip optional external-resource preflight checks.
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
    use clap::{CommandFactory, Parser};

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

    #[test]
    fn test_should_parse_query_defaults_compatible_with_graphrag() {
        let cli =
            Cli::try_parse_from(["graphloom", "query", "What happened?"]).expect("Query arguments");
        let Command::Query(args) = cli.command else {
            panic!("expected query command");
        };
        assert_eq!(args.root, std::path::Path::new("."));
        assert_eq!(args.method, crate::query::SearchMethod::Global);
        assert_eq!(args.community_level, 2);
        assert_eq!(args.response_type, "Multiple Paragraphs");
        assert!(!args.dynamic_selection_enabled());
        assert!(!args.streaming_enabled());
    }

    #[test]
    fn test_should_parse_all_query_methods_and_boolean_pairs() {
        for method in ["global", "local", "drift", "basic"] {
            let cli = Cli::try_parse_from([
                "graphloom",
                "query",
                "--method",
                method,
                "--streaming",
                "query",
            ])
            .expect("Query arguments");
            let Command::Query(args) = cli.command else {
                panic!("expected query command");
            };
            assert!(args.streaming_enabled());
        }
        let cli = Cli::try_parse_from([
            "graphloom",
            "query",
            "--streaming",
            "--no-streaming",
            "query",
        ])
        .expect("Query arguments");
        let Command::Query(args) = cli.command else {
            panic!("expected query command");
        };
        assert!(!args.streaming_enabled());
    }

    #[test]
    fn test_should_render_query_help_with_required_query_and_compatible_default() {
        let mut command = Cli::command();
        let query = command
            .find_subcommand_mut("query")
            .expect("query subcommand");
        let mut help = Vec::new();
        query.write_long_help(&mut help).expect("query help");
        let help = String::from_utf8(help).expect("UTF-8 help");

        assert!(help.contains("[OPTIONS] <QUERY>"));
        assert!(help.contains("--method <METHOD>"));
        assert!(help.contains("[default: global]"));
        for method in ["global", "local", "drift", "basic"] {
            assert!(help.contains(method));
        }
        assert!(help.contains("--streaming"));
        assert!(help.contains("--no-streaming"));
    }
}
