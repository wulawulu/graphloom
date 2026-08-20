//! Command line argument definitions.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use clap::{CommandFactory, Parser, Subcommand, ValueEnum, error::ErrorKind};

use crate::{explainability::ExplainabilityContentMode, query::SearchMethod};

/// Explainability content policy accepted by the Query CLI.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ExplainabilityContentArg {
    /// Persist metadata and identifiers without user or model content.
    Metadata,
    /// Persist the content fields needed to replay and inspect a Query.
    Content,
    /// Persist the most detailed currently supported diagnostic content.
    Debug,
}

impl From<ExplainabilityContentArg> for ExplainabilityContentMode {
    fn from(value: ExplainabilityContentArg) -> Self {
        match value {
            ExplainabilityContentArg::Metadata => Self::Metadata,
            ExplainabilityContentArg::Content => Self::Content,
            ExplainabilityContentArg::Debug => Self::Debug,
        }
    }
}

/// `GraphLoom` command line.
#[derive(Debug, Parser)]
#[command(
    name = "graphloom",
    bin_name = "graphloom",
    version,
    about = "GraphLoom: A graph-based retrieval-augmented generation (RAG) system.",
    arg_required_else_help = true
)]
pub struct Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// Parse process arguments and validate Query paths after Clap syntax validation succeeds.
    #[must_use]
    pub fn parse() -> Self {
        match Self::try_parse_from(std::env::args_os()) {
            Ok(cli) => cli,
            Err(error) => error.exit(),
        }
    }

    /// Parse supplied arguments and validate Query paths after Clap syntax validation succeeds.
    ///
    /// # Errors
    ///
    /// Returns a Clap syntax or path value-validation error.
    pub fn try_parse_from<I, T>(arguments: I) -> std::result::Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        try_parse_cli_from_with_probe(arguments, probe_directory_writable)
    }
}

/// Supported top-level commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize a `GraphLoom` project.
    Init(InitArgs),
    /// Run standard indexing.
    Index(IndexArgs),
    /// Update an existing knowledge graph index.
    Update(UpdateArgs),
    /// Query a knowledge graph index.
    #[command(about = "Query a knowledge graph index.")]
    Query(QueryArgs),
    /// Generate custom graphrag prompts with your own data (i.e. auto templating).
    #[command(
        name = "prompt-tune",
        about = "Generate custom graphrag prompts with your own data (i.e. auto templating)."
    )]
    PromptTune(PromptTuneArgs),
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
    /// Write supported Query Explainability envelopes as JSONL.
    #[arg(
        long = "explain-output",
        value_name = "EXPLAIN_OUTPUT",
        help = "Write Local, Global, or Basic Query Explainability envelopes as JSONL."
    )]
    pub explain_output: Option<PathBuf>,
    /// Explainability content policy; valid only with `--explain-output` and defaults to metadata.
    #[arg(
        long = "explain-content",
        value_name = "EXPLAIN_CONTENT",
        value_enum,
        requires = "explain_output",
        help = "Explainability content policy for Query JSONL [possible values: metadata, \
                content, debug]."
    )]
    pub explain_content: Option<ExplainabilityContentArg>,
    /// Export Local Query traces to an OTLP/HTTP collector base endpoint.
    #[arg(
        long = "otel-endpoint",
        value_name = "OTEL_ENDPOINT",
        help = "Export Local Query traces to an OTLP/HTTP collector base endpoint."
    )]
    pub otel_endpoint: Option<String>,
    /// OpenTelemetry service.name used when OTLP trace export is enabled.
    #[arg(
        long = "otel-service-name",
        value_name = "OTEL_SERVICE_NAME",
        requires = "otel_endpoint",
        value_parser = validate_otel_service_name,
        help = "OpenTelemetry service.name used when OTLP trace export is enabled."
    )]
    pub otel_service_name: Option<String>,
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

fn try_parse_cli_from_with_probe<I, T, F>(
    arguments: I,
    writable_probe: F,
) -> std::result::Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    F: Fn(&Path) -> Result<(), String>,
{
    let mut cli = <Cli as Parser>::try_parse_from(arguments)?;
    let Command::Query(args) = &mut cli.command else {
        return Ok(cli);
    };
    validate_query_explainability(args)?;
    validate_query_telemetry(args)?;
    args.explain_output = args
        .explain_output
        .as_deref()
        .map(resolve_output_path)
        .transpose()
        .map_err(query_path_error)?;
    args.root = parse_existing_root(&args.root).map_err(query_path_error)?;
    args.data = args
        .data
        .as_deref()
        .map(parse_existing_data_path)
        .transpose()
        .map_err(query_path_error)?;
    writable_probe(&args.root).map_err(query_path_error)?;
    Ok(cli)
}

fn validate_query_explainability(args: &QueryArgs) -> std::result::Result<(), clap::Error> {
    if args.explain_output.is_some()
        && !matches!(args.method, SearchMethod::Local | SearchMethod::Global)
    {
        return Err(query_path_error(
            "--explain-output currently supports --method local and static global".to_owned(),
        ));
    }
    Ok(())
}

fn validate_query_telemetry(args: &QueryArgs) -> std::result::Result<(), clap::Error> {
    if args.otel_endpoint.is_some() && !matches!(args.method, SearchMethod::Local) {
        return Err(query_path_error(
            "--otel-endpoint currently supports only --method local".to_owned(),
        ));
    }
    if let Some(endpoint) = args.otel_endpoint.as_deref()
        && let Err(message) = validate_otel_endpoint(endpoint)
    {
        return Err(query_path_error(message));
    }
    Ok(())
}

fn validate_otel_endpoint(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        return Err(OTEL_ENDPOINT_VALIDATION_ERROR.to_owned());
    }
    let lower = value.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"));
    let Some(rest) = rest else {
        return Err(OTEL_ENDPOINT_VALIDATION_ERROR.to_owned());
    };
    if rest.is_empty() || rest.chars().any(char::is_whitespace) {
        return Err(OTEL_ENDPOINT_VALIDATION_ERROR.to_owned());
    }
    if value.contains('?') || value.contains('#') {
        return Err(OTEL_ENDPOINT_VALIDATION_ERROR.to_owned());
    }
    if lower.trim_end_matches('/').ends_with("/v1/traces") {
        return Err(OTEL_ENDPOINT_VALIDATION_ERROR.to_owned());
    }
    Ok(value.to_owned())
}

const OTEL_ENDPOINT_VALIDATION_ERROR: &str = "--otel-endpoint must be a non-empty http or https \
                                              collector base endpoint without query, fragment, or \
                                              /v1/traces";

fn validate_otel_service_name(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err("OpenTelemetry service name must not be empty".to_owned());
    }
    if value.trim().is_empty() {
        return Err("OpenTelemetry service name must not be whitespace-only".to_owned());
    }
    if value.len() > 128 {
        return Err("OpenTelemetry service name must be at most 128 bytes".to_owned());
    }
    Ok(value.to_owned())
}

fn resolve_output_path(value: &Path) -> Result<PathBuf, String> {
    if value.is_absolute() {
        return Ok(value.to_path_buf());
    }
    std::env::current_dir()
        .map(|current| current.join(value))
        .map_err(|source| format!("cannot resolve Explainability output path: {source}"))
}

fn query_path_error(message: String) -> clap::Error {
    let mut command = Cli::command();
    if let Some(query) = command.find_subcommand_mut("query") {
        return query.clone().error(ErrorKind::ValueValidation, message);
    }
    clap::Error::raw(ErrorKind::ValueValidation, message)
}

#[allow(
    clippy::disallowed_methods,
    reason = "clap path validation is synchronous and must finish before async runtime entry"
)]
fn parse_existing_root(value: &Path) -> Result<PathBuf, String> {
    let path = canonicalize_from_current_dir(value, "project root")?;
    if !path.is_dir() {
        return Err(format!(
            "project root must be a directory: {}",
            path.display()
        ));
    }
    std::fs::read_dir(&path)
        .map_err(|source| format!("project root is not readable {}: {source}", path.display()))?;
    Ok(path)
}

#[allow(
    clippy::disallowed_methods,
    reason = "clap value parsers are synchronous and must validate paths before async runtime \
              entry"
)]
fn parse_existing_data_path(value: &Path) -> Result<PathBuf, String> {
    let path = canonicalize_from_current_dir(value, "data path")?;
    if !path.is_dir() {
        return Err("data path must be a readable index output directory".to_owned());
    }
    std::fs::read_dir(&path)
        .map_err(|source| format!("data directory is not readable: {source}"))?;
    Ok(path)
}

fn canonicalize_from_current_dir(value: &Path, description: &str) -> Result<PathBuf, String> {
    let path = if value.is_absolute() {
        value.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|source| format!("cannot resolve current directory: {source}"))?
            .join(value)
    };
    path.canonicalize()
        .map_err(|source| format!("{description} does not exist or cannot be resolved: {source}"))
}

#[allow(
    clippy::disallowed_methods,
    reason = "the synchronous clap parser requires an atomic create/remove writability probe"
)]
fn probe_directory_writable(path: &Path) -> Result<(), String> {
    static PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| format!("cannot construct project root write probe: {source}"))?
        .as_nanos();
    let sequence = PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let probe = path.join(format!(
        ".graphloom-query-write-probe-{}-{timestamp}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir(&probe)
        .map_err(|source| format!("project root is not writable {}: {source}", path.display()))?;
    std::fs::remove_dir(&probe).map_err(|source| {
        let _cleanup_result = std::fs::remove_dir(&probe);
        format!(
            "cannot remove project root write probe {}: {source}",
            probe.display()
        )
    })
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
    /// Language used for the initialized prompt templates.
    #[arg(short = 'l', long = "language", value_enum, default_value = "english")]
    pub language: PromptLanguage,
    /// Overwrite GraphLoom-managed files.
    #[arg(short = 'f', long = "force")]
    pub force: bool,
}

/// Languages available for project prompt templates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum PromptLanguage {
    /// Original English `GraphRAG` prompts.
    #[default]
    #[value(alias = "en")]
    English,
    /// Chinese `GraphRAG`-compatible prompts.
    #[value(alias = "zh", alias = "zh-cn")]
    Chinese,
}

/// CLI document selection method for prompt tuning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PromptTuneSelectionMethod {
    /// Select the first `limit` chunks in document order.
    Top,
    /// Select `limit` chunks uniformly at random.
    Random,
    /// Select chunks closest to the embedding centroid.
    Auto,
}

/// `graphloom prompt-tune` arguments.
#[derive(Debug, Clone, Parser)]
pub struct PromptTuneArgs {
    /// Project root directory.
    #[arg(short = 'r', long = "root", default_value_os_t = default_root())]
    pub root: PathBuf,
    /// Run the prompt tuning pipeline with verbose logging.
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,
    /// Domain description (auto-detected if omitted).
    #[arg(
        long = "domain",
        help = "The domain your input data is related to. For example 'space science', \
                'microbiology', 'environmental news'. If not defined, a domain will be inferred \
                from the input data."
    )]
    pub domain: Option<String>,
    /// The text chunk selection method.
    #[arg(
        long = "selection-method",
        default_value = "random",
        help = "The text chunk selection method."
    )]
    pub selection_method: PromptTuneSelectionMethod,
    /// The number of text chunks to embed when --selection-method=auto.
    #[arg(
        long = "n-subset-max",
        default_value_t = 300,
        help = "The number of text chunks to embed when --selection-method=auto."
    )]
    pub n_subset_max: usize,
    /// The maximum number of documents to select from each centroid when
    /// --selection-method=auto.
    #[arg(
        long = "k",
        default_value_t = 15,
        help = "The maximum number of documents to select from each centroid when \
                --selection-method=auto."
    )]
    pub k: usize,
    /// The number of documents to load when --selection-method={random,top}.
    #[arg(
        long = "limit",
        default_value_t = 15,
        help = "The number of documents to load when --selection-method={random,top}."
    )]
    pub limit: usize,
    /// The max token count for prompt generation.
    #[arg(
        long = "max-tokens",
        default_value_t = 2000,
        help = "The max token count for prompt generation."
    )]
    pub max_tokens: usize,
    /// The minimum number of examples to generate/include in the entity extraction prompt.
    #[arg(
        long = "min-examples-required",
        default_value_t = 2,
        help = "The minimum number of examples to generate/include in the entity extraction \
                prompt."
    )]
    pub min_examples_required: usize,
    /// The size of each example text chunk. Overrides chunking.size in the configuration file.
    #[arg(
        long = "chunk-size",
        default_value_t = 1200,
        help = "The size of each example text chunk. Overrides chunking.size in the configuration \
                file."
    )]
    pub chunk_size: usize,
    /// The overlap size for chunking documents. Overrides chunking.overlap in the configuration
    /// file.
    #[arg(
        long = "overlap",
        default_value_t = 100,
        help = "The overlap size for chunking documents. Overrides chunking.overlap in the \
                configuration file."
    )]
    pub overlap: usize,
    /// The primary language used for inputs and outputs in graphrag prompts.
    #[arg(
        long = "language",
        help = "The primary language used for inputs and outputs in graphrag prompts."
    )]
    pub language: Option<String>,
    /// Discover and extract unspecified entity types.
    #[arg(
        long = "discover-entity-types",
        default_value_t = true,
        action = clap::ArgAction::SetTrue,
        overrides_with = "no_discover_entity_types",
        help = "Discover and extract unspecified entity types."
    )]
    pub discover_entity_types: bool,
    /// Do not discover entity types; generate untyped extraction templates.
    #[arg(
        long = "no-discover-entity-types",
        action = clap::ArgAction::SetTrue,
        help = "Skip entity-type discovery and generate untyped extraction templates."
    )]
    pub no_discover_entity_types: bool,
    /// The directory to save prompts to, relative to the project root directory.
    #[arg(
        short = 'o',
        long = "output",
        default_value = "prompts",
        help = "The directory to save prompts to, relative to the project root directory."
    )]
    pub output: PathBuf,
}

impl PromptTuneArgs {
    /// Return whether entity type discovery is enabled.
    #[must_use]
    pub fn discover_entity_types_enabled(&self) -> bool {
        self.discover_entity_types && !self.no_discover_entity_types
    }
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

/// `graphloom update` arguments.
#[derive(Debug, Clone, Parser)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "clap option structs intentionally mirror boolean command-line flags"
)]
pub struct UpdateArgs {
    /// Project root directory.
    #[arg(short = 'r', long = "root", default_value_os_t = default_root())]
    pub root: PathBuf,
    /// Indexing method.
    #[arg(short = 'm', long = "method", default_value = "standard")]
    pub method: IndexMethodArg,
    /// Print more detailed progress.
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,
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

impl UpdateArgs {
    /// Return whether cache is enabled for this run.
    #[must_use]
    pub fn cache_enabled(&self) -> bool {
        self.cache && !self.no_cache
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use clap::CommandFactory;
    use serde_json::Value;
    use tempfile::TempDir;

    use super::{
        Cli, Command, PromptLanguage, parse_existing_data_path, parse_existing_root,
        try_parse_cli_from_with_probe,
    };

    const QUERY_CLI_CONTRACT: &str =
        include_str!("../../../../tests/compat/fixtures/query/query_cli_contract.json");

    #[test]
    fn test_should_use_stable_binary_name_when_argv_zero_has_exe_suffix() {
        let error = Cli::try_parse_from(["graphloom.exe", "query", "--help"])
            .expect_err("help should exit through Clap");

        assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);

        let help = error.to_string();
        assert!(help.contains("Usage: graphloom query [OPTIONS] <QUERY>"));
        assert!(!help.contains("graphloom.exe"));
    }

    #[test]
    fn test_should_enable_configured_cache_by_default() {
        let cli = Cli::try_parse_from(["graphloom", "index"]).expect("CLI arguments");
        let Command::Index(args) = cli.command else {
            panic!("expected index command");
        };
        assert!(args.cache_enabled());
    }

    #[test]
    fn test_should_parse_init_prompt_language_with_english_default() {
        let cli = Cli::try_parse_from(["graphloom", "init"]).expect("default init arguments");
        let Command::Init(args) = cli.command else {
            panic!("expected init command");
        };
        assert_eq!(args.language, PromptLanguage::English);

        for language in ["chinese", "zh", "zh-cn"] {
            let cli = Cli::try_parse_from(["graphloom", "init", "--language", language])
                .expect("Chinese init arguments");
            let Command::Init(args) = cli.command else {
                panic!("expected init command");
            };
            assert_eq!(args.language, PromptLanguage::Chinese);
        }
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
        assert_eq!(
            args.root,
            std::env::current_dir()
                .expect("current directory")
                .canonicalize()
                .expect("canonical current directory")
        );
        assert_eq!(args.method, crate::query::SearchMethod::Global);
        assert_eq!(args.community_level, 2);
        assert_eq!(args.response_type, "Multiple Paragraphs");
        assert!(!args.verbose);
        assert!(args.data.is_none());
        assert!(!args.dynamic_selection_enabled());
        assert!(!args.streaming_enabled());
    }

    #[test]
    fn test_should_apply_graphrag_path_contract_to_query_arguments() {
        let fixture = TempDir::new().expect("path fixture");
        let data_file = tempfile::NamedTempFile::new_in(fixture.path()).expect("data file");
        let data_file_path = data_file.path();

        assert_eq!(
            parse_existing_root(fixture.path()).expect("existing root"),
            fixture.path().canonicalize().expect("canonical root")
        );
        assert_eq!(
            parse_existing_data_path(fixture.path()).expect("existing data directory"),
            fixture
                .path()
                .canonicalize()
                .expect("canonical data directory")
        );
        let file_error =
            parse_existing_data_path(data_file_path).expect_err("data file must be rejected");
        assert_eq!(
            file_error,
            "data path must be a readable index output directory"
        );
        assert!(parse_existing_root(data_file_path).is_err());
        assert!(parse_existing_data_path(&fixture.path().join("missing")).is_err());
        assert!(
            !fixture
                .path()
                .read_dir()
                .expect("fixture entries")
                .filter_map(std::result::Result::ok)
                .any(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".graphloom-query-write-probe-"))
        );
    }

    #[test]
    fn test_should_not_probe_query_root_before_clap_syntax_succeeds() {
        for arguments in [
            &["graphloom", "query"][..],
            &["graphloom", "query", "--unknown", "question"][..],
            &["graphloom", "query", "--root"][..],
            &[
                "graphloom",
                "query",
                "--data",
                ".graphloom-definitely-missing-query-data",
                "question",
            ][..],
        ] {
            let probed = AtomicBool::new(false);
            let result = try_parse_cli_from_with_probe(arguments, |_| {
                probed.store(true, Ordering::Relaxed);
                Ok(())
            });
            assert!(result.is_err(), "{arguments:?}");
            assert!(!probed.load(Ordering::Relaxed), "{arguments:?}");
        }
    }

    #[test]
    fn test_should_reject_otlp_arguments_for_non_local_methods() {
        for method_name in ["basic", "global", "drift"] {
            let error = Cli::try_parse_from([
                "graphloom",
                "query",
                "--method",
                method_name,
                "--otel-endpoint",
                "http://collector.invalid:4318",
                "question",
            ])
            .expect_err("non-local OTLP endpoint must be rejected");
            assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
            assert!(
                error
                    .to_string()
                    .contains("--otel-endpoint currently supports only --method local"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    fn test_should_require_endpoint_for_otel_service_name() {
        let error = Cli::try_parse_from([
            "graphloom",
            "query",
            "--method",
            "local",
            "--otel-service-name",
            "graphloom-test",
            "question",
        ])
        .expect_err("service name without endpoint must be rejected");
        assert!(
            error.to_string().contains("--otel-endpoint"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn test_should_reject_invalid_otel_service_names() {
        let long_name = "a".repeat(129);
        for value in ["", "   ", long_name.as_str()] {
            let error = Cli::try_parse_from([
                "graphloom",
                "query",
                "--method",
                "local",
                "--otel-endpoint",
                "http://collector.invalid:4318",
                "--otel-service-name",
                value,
                "question",
            ])
            .expect_err("invalid service name must be rejected");
            assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
        }
    }

    #[test]
    fn test_should_parse_valid_otel_arguments() {
        let cli = Cli::try_parse_from([
            "graphloom",
            "query",
            "--method",
            "local",
            "--otel-endpoint",
            "http://collector.invalid:4318",
            "--otel-service-name",
            "graphloom-test",
            "question",
        ])
        .expect("valid OTLP arguments");
        let Command::Query(args) = cli.command else {
            panic!("expected Query command");
        };
        assert_eq!(
            args.otel_endpoint.as_deref(),
            Some("http://collector.invalid:4318")
        );
        assert_eq!(args.otel_service_name.as_deref(), Some("graphloom-test"));
    }

    #[test]
    fn test_should_accept_valid_otel_base_endpoints() {
        for endpoint in [
            "http://localhost:4318",
            "http://localhost:4318/",
            "https://collector.example.com",
            "https://collector.example.com/custom",
            "HTTPS://collector.example.com/custom",
        ] {
            let cli = Cli::try_parse_from([
                "graphloom",
                "query",
                "--method",
                "local",
                "--otel-endpoint",
                endpoint,
                "question",
            ])
            .expect("valid OTLP endpoint");
            let Command::Query(args) = cli.command else {
                panic!("expected Query command");
            };
            assert_eq!(
                args.otel_endpoint.as_deref(),
                Some(endpoint),
                "original base path must be preserved"
            );
        }
    }

    #[test]
    fn test_should_reject_invalid_otel_endpoints_without_echoing_value() {
        for endpoint in [
            "",
            "   ",
            "ftp://collector:4318",
            "http://collector:4318?token=secret",
            "http://collector:4318/#fragment",
            "http://collector:4318/v1/traces",
            "http://collector:4318/v1/traces/",
            "http://",
            "http://host with space:4318",
        ] {
            let error = Cli::try_parse_from([
                "graphloom",
                "query",
                "--method",
                "local",
                "--otel-endpoint",
                endpoint,
                "question",
            ])
            .expect_err("invalid OTLP endpoint must be rejected");
            assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
            let text = error.to_string();
            assert!(
                text.contains(
                    "--otel-endpoint must be a non-empty http or https collector base endpoint \
                     without query, fragment, or /v1/traces"
                ),
                "unexpected error: {text}"
            );
            if !endpoint.is_empty() && !endpoint.trim().is_empty() {
                assert!(
                    !text.contains(endpoint),
                    "error must not echo the endpoint value: {text}"
                );
            }
        }
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
                "query"
                | "no_dynamic_selection"
                | "no_streaming"
                | "explain_output"
                | "explain_content"
                | "otel_endpoint"
                | "otel_service_name" => None,
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
                "root" => Value::String(".".to_owned()),
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
