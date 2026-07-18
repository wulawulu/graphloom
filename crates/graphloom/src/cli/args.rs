//! Command line argument definitions.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::query::SearchMethod;

/// `GraphLoom` command line.
#[derive(Debug, Parser)]
#[command(
    name = "graphloom",
    version,
    about = "GraphLoom: A graph-based retrieval-augmented generation (RAG) system.",
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
    /// Query a knowledge graph index.
    #[command(about = "Query a knowledge graph index.")]
    Query(QueryArgs),
}

/// `graphloom query` arguments.
#[derive(Debug, Clone, Parser)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "clap option structs intentionally mirror boolean command-line flags"
)]
pub struct QueryArgs {
    /// The project root directory.
    #[arg(
        short = 'r',
        long = "root",
        default_value_os_t = default_root(),
        help = "The project root directory."
    )]
    pub root: PathBuf,
    /// The query algorithm to use.
    #[arg(
        short = 'm',
        long = "method",
        default_value = "global",
        help = "The query algorithm to use."
    )]
    pub method: SearchMethod,
    /// Run the query with verbose logging.
    #[arg(
        short = 'v',
        long = "verbose",
        help = "Run the query with verbose logging."
    )]
    pub verbose: bool,
    /// Index output directory (contains the parquet files).
    #[arg(
        short = 'd',
        long = "data",
        help = "Index output directory (contains the parquet files)."
    )]
    pub data: Option<PathBuf>,
    /// Leiden hierarchy level from which to load community reports. Higher values represent
    /// smaller communities.
    #[arg(
        long = "community-level",
        default_value_t = 2,
        help = "Leiden hierarchy level from which to load community reports. Higher values \
                represent smaller communities."
    )]
    pub community_level: i64,
    /// Use global search with dynamic community selection.
    #[arg(
        long = "dynamic-community-selection",
        action = clap::ArgAction::SetTrue,
        overrides_with = "no_dynamic_selection",
        help = "Use global search with dynamic community selection."
    )]
    pub dynamic_community_selection: bool,
    /// Use global search without dynamic community selection.
    #[arg(
        long = "no-dynamic-selection",
        action = clap::ArgAction::SetTrue,
        help = "Use global search with dynamic community selection."
    )]
    pub no_dynamic_selection: bool,
    /// Free-form description of the desired response format (e.g. 'Single Sentence', 'List of
    /// 3-7 Points', etc.).
    #[arg(
        long = "response-type",
        default_value = "Multiple Paragraphs",
        help = "Free-form description of the desired response format (e.g. 'Single Sentence', \
                'List of 3-7 Points', etc.)."
    )]
    pub response_type: String,
    /// Print the response in a streaming manner.
    #[arg(
        long = "streaming",
        action = clap::ArgAction::SetTrue,
        overrides_with = "no_streaming",
        help = "Print the response in a streaming manner."
    )]
    pub streaming: bool,
    /// Print the response without streaming.
    #[arg(
        long = "no-streaming",
        action = clap::ArgAction::SetTrue,
        help = "Print the response in a streaming manner."
    )]
    pub no_streaming: bool,
    /// The query to execute.
    #[arg(help = "The query to execute.")]
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
    use serde_json::Value;

    use super::{Cli, Command};

    const QUERY_CLI_CONTRACT: &str =
        include_str!("../../../../tests/compat/fixtures/query/query_cli_contract.json");

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
        assert!(!args.verbose);
        assert!(args.data.is_none());
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
        for (flags, expected) in [
            (&[][..], false),
            (&["--streaming"][..], true),
            (&["--no-streaming"][..], false),
            (&["--streaming", "--no-streaming"][..], false),
            (&["--no-streaming", "--streaming"][..], true),
        ] {
            let mut command = vec!["graphloom", "query"];
            command.extend_from_slice(flags);
            command.push("query");
            let cli = Cli::try_parse_from(command).expect("streaming arguments");
            let Command::Query(args) = cli.command else {
                panic!("expected query command");
            };
            assert_eq!(args.streaming_enabled(), expected, "{flags:?}");
        }
        for (flags, expected) in [
            (&[][..], false),
            (&["--dynamic-community-selection"][..], true),
            (&["--no-dynamic-selection"][..], false),
            (
                &["--dynamic-community-selection", "--no-dynamic-selection"][..],
                false,
            ),
            (
                &["--no-dynamic-selection", "--dynamic-community-selection"][..],
                true,
            ),
        ] {
            let mut command = vec!["graphloom", "query"];
            command.extend_from_slice(flags);
            command.push("query");
            let cli = Cli::try_parse_from(command).expect("dynamic-selection arguments");
            let Command::Query(args) = cli.command else {
                panic!("expected query command");
            };
            assert_eq!(args.dynamic_selection_enabled(), expected, "{flags:?}");
        }
    }

    #[test]
    fn test_should_match_shared_graphrag_query_cli_contract() {
        let contract: Value = serde_json::from_str(QUERY_CLI_CONTRACT).expect("CLI contract JSON");
        let defaults =
            Cli::try_parse_from(["graphloom", "query", "question"]).expect("Query defaults");
        let Command::Query(defaults) = defaults.command else {
            panic!("expected query command");
        };
        let mut command = Cli::command();
        let query = command
            .find_subcommand_mut("query")
            .expect("query subcommand");
        assert_eq!(
            query.get_name(),
            contract["command"].as_str().expect("command name")
        );
        assert_eq!(
            query.get_about().map(ToString::to_string).as_deref(),
            contract["about"].as_str()
        );

        let positional = query
            .get_arguments()
            .find(|argument| argument.get_id() == "query")
            .expect("query positional");
        assert!(positional.is_required_set());
        assert_eq!(
            positional.get_help().map(ToString::to_string).as_deref(),
            contract["argument"]["help"].as_str()
        );

        let expected_options = contract["options"].as_array().expect("contract options");
        let actual_order = query
            .get_arguments()
            .filter_map(|argument| match argument.get_id().as_str() {
                "query" | "no_dynamic_selection" | "no_streaming" => None,
                name => Some(name),
            })
            .collect::<Vec<_>>();
        let expected_order = expected_options
            .iter()
            .map(|option| option["name"].as_str().expect("option name"))
            .collect::<Vec<_>>();
        assert_eq!(actual_order, expected_order);

        for option in expected_options {
            let name = option["name"].as_str().expect("option name");
            let argument = query
                .get_arguments()
                .find(|argument| argument.get_id() == name)
                .expect("contract argument");
            let mut flags = Vec::new();
            if let Some(flag) = argument.get_short() {
                flags.push(format!("-{flag}"));
            }
            if let Some(flag) = argument.get_long() {
                flags.push(format!("--{flag}"));
            }
            if name == "dynamic_community_selection" {
                flags.push("--no-dynamic-selection".to_owned());
            } else if name == "streaming" {
                flags.push("--no-streaming".to_owned());
            }
            let expected_flags = option["flags"]
                .as_array()
                .expect("option flags")
                .iter()
                .map(|flag| flag.as_str().expect("flag").to_owned())
                .collect::<Vec<_>>();
            assert_eq!(flags, expected_flags);
            assert_eq!(
                argument.get_help().map(ToString::to_string).as_deref(),
                option["help"].as_str()
            );
            assert_eq!(
                argument.is_required_set(),
                option["required"].as_bool().expect("required flag")
            );
            let default = match name {
                "root" => Value::String(defaults.root.to_string_lossy().into_owned()),
                "method" => Value::String(defaults.method.to_string()),
                "verbose" => Value::Bool(defaults.verbose),
                "data" => Value::Null,
                "community_level" => Value::from(defaults.community_level),
                "dynamic_community_selection" => Value::Bool(defaults.dynamic_selection_enabled()),
                "response_type" => Value::String(defaults.response_type.clone()),
                "streaming" => Value::Bool(defaults.streaming_enabled()),
                other => panic!("unexpected Query option {other}"),
            };
            assert_eq!(default, option["default"]);
            let choices = if name == "method" {
                argument
                    .get_value_parser()
                    .possible_values()
                    .map(|values| {
                        values
                            .map(|value| Value::String(value.get_name().to_owned()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            assert_eq!(
                choices,
                option["choices"].as_array().expect("choices").clone()
            );
        }
    }
}
