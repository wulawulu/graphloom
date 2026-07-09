//! GraphRAG-compatible configuration models used by indexing workflows.

use std::collections::BTreeMap;

use graphloom_chunking::ChunkingConfig;
use graphloom_llm::ModelConfig;
use graphloom_vectors::VectorStoreConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod load;

const DEFAULT_CONCURRENT_REQUESTS: usize = 25;
const DEFAULT_COMPLETION_MODEL_ID: &str = "default_completion_model";
const DEFAULT_EXTRACT_GRAPH_MODEL_INSTANCE_NAME: &str = "extract_graph";
const DEFAULT_SUMMARIZE_MODEL_INSTANCE_NAME: &str = "summarize_descriptions";
const DEFAULT_EXTRACT_CLAIMS_MODEL_INSTANCE_NAME: &str = "extract_claims";
const DEFAULT_COMMUNITY_REPORTS_MODEL_INSTANCE_NAME: &str = "community_reporting";
const DEFAULT_CLAIM_DESCRIPTION: &str =
    "Any claims or facts that could be relevant to information discovery.";
const DEFAULT_INPUT_TYPE: &str = "text";
const DEFAULT_FILE_PATTERN: &str = r".*\.txt$";
const DEFAULT_TEXT_COLUMN: &str = "text";
const DEFAULT_TITLE_COLUMN: &str = "title";
const DEFAULT_FILE_STORAGE_TYPE: &str = "file";
const DEFAULT_JSON_CACHE_TYPE: &str = "json";
const DEFAULT_INPUT_BASE_DIR: &str = "input";
const DEFAULT_OUTPUT_BASE_DIR: &str = "output";
const DEFAULT_CACHE_BASE_DIR: &str = "cache";
const DEFAULT_REPORTING_BASE_DIR: &str = "logs";
const DEFAULT_MAX_GLEANINGS: usize = 1;
const DEFAULT_MAX_SUMMARY_LENGTH: usize = 500;
const DEFAULT_MAX_INPUT_TOKENS: usize = 4_000;
const DEFAULT_MAX_CLUSTER_SIZE: u32 = 10;
const DEFAULT_CLUSTER_SEED: u64 = 0xDEAD_BEEF;
const DEFAULT_COMMUNITY_REPORT_MAX_LENGTH: usize = 2_000;
const DEFAULT_COMMUNITY_REPORT_MAX_INPUT_LENGTH: usize = 8_000;
const DEFAULT_EMBEDDING_MODEL_ID: &str = "default_embedding_model";
const DEFAULT_EMBED_TEXT_MODEL_INSTANCE_NAME: &str = "text_embedding";
const DEFAULT_EMBED_TEXT_BATCH_SIZE: usize = 16;
const DEFAULT_EMBED_TEXT_BATCH_MAX_TOKENS: usize = 8_191;

/// Entity title and description embedding name.
pub const ENTITY_DESCRIPTION_EMBEDDING: &str = "entity_description";
/// Community full content embedding name.
pub const COMMUNITY_FULL_CONTENT_EMBEDDING: &str = "community_full_content";
/// Text unit text embedding name.
pub const TEXT_UNIT_TEXT_EMBEDDING: &str = "text_unit_text";
/// All supported embedding names.
pub const ALL_EMBEDDINGS: &[&str] = &[
    ENTITY_DESCRIPTION_EMBEDDING,
    COMMUNITY_FULL_CONTENT_EMBEDDING,
    TEXT_UNIT_TEXT_EMBEDDING,
];
/// Default embedding names, matching Microsoft `GraphRAG`.
pub const DEFAULT_EMBEDDINGS: &[&str] = &[
    ENTITY_DESCRIPTION_EMBEDDING,
    COMMUNITY_FULL_CONTENT_EMBEDDING,
    TEXT_UNIT_TEXT_EMBEDDING,
];

fn default_entity_types() -> Vec<String> {
    ["organization", "person", "geo", "event"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// Input reader configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct InputConfig {
    /// Input reader type.
    #[serde(rename = "type")]
    pub input_type: String,
    /// File pattern for file readers.
    #[serde(alias = "file_pattern")]
    pub file_pattern: String,
    /// Text column for structured inputs.
    #[serde(alias = "text_column")]
    pub text_column: String,
    /// Title column for structured inputs.
    #[serde(alias = "title_column")]
    pub title_column: String,
    /// Metadata fields to collect from structured inputs.
    pub metadata: Vec<String>,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            input_type: DEFAULT_INPUT_TYPE.to_owned(),
            file_pattern: DEFAULT_FILE_PATTERN.to_owned(),
            text_column: DEFAULT_TEXT_COLUMN.to_owned(),
            title_column: DEFAULT_TITLE_COLUMN.to_owned(),
            metadata: Vec::new(),
        }
    }
}

fn default_file_storage_type() -> String {
    DEFAULT_FILE_STORAGE_TYPE.to_owned()
}

fn default_json_cache_type() -> String {
    DEFAULT_JSON_CACHE_TYPE.to_owned()
}

fn default_input_base_dir() -> String {
    DEFAULT_INPUT_BASE_DIR.to_owned()
}

fn default_output_base_dir() -> String {
    DEFAULT_OUTPUT_BASE_DIR.to_owned()
}

fn default_cache_base_dir() -> String {
    DEFAULT_CACHE_BASE_DIR.to_owned()
}

fn default_reporting_base_dir() -> String {
    DEFAULT_REPORTING_BASE_DIR.to_owned()
}

/// File/blob/cosmos storage configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct StorageConfig {
    /// Storage provider type.
    #[serde(
        rename = "type",
        alias = "storage_type",
        default = "default_file_storage_type"
    )]
    pub storage_type: String,
    /// Base directory or provider-specific root.
    #[serde(alias = "base_dir")]
    pub base_dir: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            storage_type: default_file_storage_type(),
            base_dir: default_input_base_dir(),
        }
    }
}

impl StorageConfig {
    fn output_default() -> Self {
        Self {
            storage_type: default_file_storage_type(),
            base_dir: default_output_base_dir(),
        }
    }
}

/// Reporting sink configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct ReportingConfig {
    /// Reporting provider type.
    #[serde(
        rename = "type",
        alias = "reporting_type",
        default = "default_file_storage_type"
    )]
    pub reporting_type: String,
    /// Reporting base directory.
    #[serde(alias = "base_dir", default = "default_reporting_base_dir")]
    pub base_dir: String,
}

impl Default for ReportingConfig {
    fn default() -> Self {
        Self {
            reporting_type: default_file_storage_type(),
            base_dir: default_reporting_base_dir(),
        }
    }
}

/// Cache storage configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct CacheStorageConfig {
    /// Cache storage provider type.
    #[serde(
        rename = "type",
        alias = "storage_type",
        default = "default_file_storage_type"
    )]
    pub storage_type: String,
    /// Cache storage base directory.
    #[serde(alias = "base_dir", default = "default_cache_base_dir")]
    pub base_dir: String,
}

impl Default for CacheStorageConfig {
    fn default() -> Self {
        Self {
            storage_type: default_file_storage_type(),
            base_dir: default_cache_base_dir(),
        }
    }
}

/// LLM cache configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct CacheConfig {
    /// Cache provider type.
    #[serde(
        rename = "type",
        alias = "cache_type",
        default = "default_json_cache_type"
    )]
    pub cache_type: String,
    /// Cache storage.
    pub storage: CacheStorageConfig,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            cache_type: default_json_cache_type(),
            storage: CacheStorageConfig::default(),
        }
    }
}

/// Graph extraction configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct ExtractGraphConfig {
    /// Completion model id.
    #[serde(alias = "completion_model_id")]
    pub completion_model_id: String,
    /// Model instance/cache namespace name.
    #[serde(alias = "model_instance_name")]
    pub model_instance_name: String,
    /// Optional prompt file path override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Entity types to ask the extractor to identify.
    #[serde(alias = "entity_types", default = "default_entity_types")]
    pub entity_types: Vec<String>,
    /// Maximum number of gleaning rounds.
    #[serde(alias = "max_gleanings")]
    pub max_gleanings: usize,
}

impl Default for ExtractGraphConfig {
    fn default() -> Self {
        Self {
            completion_model_id: DEFAULT_COMPLETION_MODEL_ID.to_owned(),
            model_instance_name: DEFAULT_EXTRACT_GRAPH_MODEL_INSTANCE_NAME.to_owned(),
            prompt: None,
            entity_types: default_entity_types(),
            max_gleanings: DEFAULT_MAX_GLEANINGS,
        }
    }
}

/// Description summarization configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct SummarizeDescriptionsConfig {
    /// Completion model id.
    #[serde(alias = "completion_model_id")]
    pub completion_model_id: String,
    /// Model instance/cache namespace name.
    #[serde(alias = "model_instance_name")]
    pub model_instance_name: String,
    /// Optional prompt file path override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Maximum summary length.
    #[serde(alias = "max_length")]
    pub max_length: usize,
    /// Maximum input tokens.
    #[serde(alias = "max_input_tokens")]
    pub max_input_tokens: usize,
}

impl Default for SummarizeDescriptionsConfig {
    fn default() -> Self {
        Self {
            completion_model_id: DEFAULT_COMPLETION_MODEL_ID.to_owned(),
            model_instance_name: DEFAULT_SUMMARIZE_MODEL_INSTANCE_NAME.to_owned(),
            prompt: None,
            max_length: DEFAULT_MAX_SUMMARY_LENGTH,
            max_input_tokens: DEFAULT_MAX_INPUT_TOKENS,
        }
    }
}

/// Claim extraction configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct ExtractClaimsConfig {
    /// Whether claim extraction is enabled.
    pub enabled: bool,
    /// Completion model id.
    #[serde(alias = "completion_model_id")]
    pub completion_model_id: String,
    /// Model instance/cache namespace name.
    #[serde(alias = "model_instance_name")]
    pub model_instance_name: String,
    /// Optional prompt file path override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Claim description inserted into the extraction prompt.
    pub description: String,
    /// Maximum number of claim gleaning rounds.
    #[serde(alias = "max_gleanings")]
    pub max_gleanings: usize,
}

impl Default for ExtractClaimsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            completion_model_id: DEFAULT_COMPLETION_MODEL_ID.to_owned(),
            model_instance_name: DEFAULT_EXTRACT_CLAIMS_MODEL_INSTANCE_NAME.to_owned(),
            prompt: None,
            description: DEFAULT_CLAIM_DESCRIPTION.to_owned(),
            max_gleanings: DEFAULT_MAX_GLEANINGS,
        }
    }
}

/// Graph clustering configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct ClusterGraphConfig {
    /// Maximum cluster size before hierarchical refinement.
    #[serde(alias = "max_cluster_size")]
    pub max_cluster_size: u32,
    /// Whether to restrict clustering to the stable largest connected component.
    #[serde(alias = "use_lcc")]
    pub use_lcc: bool,
    /// Deterministic Leiden seed.
    pub seed: u64,
}

impl Default for ClusterGraphConfig {
    fn default() -> Self {
        Self {
            max_cluster_size: DEFAULT_MAX_CLUSTER_SIZE,
            use_lcc: true,
            seed: DEFAULT_CLUSTER_SEED,
        }
    }
}

/// Community report generation configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct CommunityReportsConfig {
    /// Completion model id.
    #[serde(alias = "completion_model_id")]
    pub completion_model_id: String,
    /// Model instance/cache namespace name.
    #[serde(alias = "model_instance_name")]
    pub model_instance_name: String,
    /// Optional graph-context prompt file path override.
    #[serde(alias = "graph_prompt", skip_serializing_if = "Option::is_none")]
    pub graph_prompt: Option<String>,
    /// Optional text-context prompt file path retained for `GraphRAG` config compatibility.
    #[serde(alias = "text_prompt", skip_serializing_if = "Option::is_none")]
    pub text_prompt: Option<String>,
    /// Maximum report length passed into the prompt.
    #[serde(alias = "max_length")]
    pub max_length: usize,
    /// Maximum input context tokens.
    #[serde(alias = "max_input_length")]
    pub max_input_length: usize,
}

impl Default for CommunityReportsConfig {
    fn default() -> Self {
        Self {
            completion_model_id: DEFAULT_COMPLETION_MODEL_ID.to_owned(),
            model_instance_name: DEFAULT_COMMUNITY_REPORTS_MODEL_INSTANCE_NAME.to_owned(),
            graph_prompt: None,
            text_prompt: None,
            max_length: DEFAULT_COMMUNITY_REPORT_MAX_LENGTH,
            max_input_length: DEFAULT_COMMUNITY_REPORT_MAX_INPUT_LENGTH,
        }
    }
}

/// Text embedding generation configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct EmbedTextConfig {
    /// Embedding model id.
    #[serde(alias = "embedding_model_id")]
    pub embedding_model_id: String,
    /// Model instance/cache namespace name.
    #[serde(alias = "model_instance_name")]
    pub model_instance_name: String,
    /// Maximum input snippets per provider request.
    #[serde(alias = "batch_size")]
    pub batch_size: usize,
    /// Maximum total tokens per provider request and split chunk.
    #[serde(alias = "batch_max_tokens")]
    pub batch_max_tokens: usize,
    /// Embedding names to generate.
    pub names: Vec<String>,
}

impl Default for EmbedTextConfig {
    fn default() -> Self {
        Self {
            embedding_model_id: DEFAULT_EMBEDDING_MODEL_ID.to_owned(),
            model_instance_name: DEFAULT_EMBED_TEXT_MODEL_INSTANCE_NAME.to_owned(),
            batch_size: DEFAULT_EMBED_TEXT_BATCH_SIZE,
            batch_max_tokens: DEFAULT_EMBED_TEXT_BATCH_MAX_TOKENS,
            names: DEFAULT_EMBEDDINGS
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
        }
    }
}

impl EmbedTextConfig {
    /// Validate embedding generation configuration.
    ///
    /// # Errors
    ///
    /// Returns an error message for invalid batching or embedding names.
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.embedding_model_id.trim().is_empty() {
            return Err("embedding_model_id must not be empty".to_owned());
        }
        if self.model_instance_name.trim().is_empty() {
            return Err("model_instance_name must not be empty".to_owned());
        }
        if self.batch_size == 0 {
            return Err("batch_size must be greater than zero".to_owned());
        }
        if self.batch_max_tokens <= 100 {
            return Err("batch_max_tokens must be greater than 100".to_owned());
        }
        if self.names.is_empty() {
            return Err("names must not be empty".to_owned());
        }
        let mut seen = std::collections::BTreeSet::new();
        for name in &self.names {
            if !ALL_EMBEDDINGS.contains(&name.as_str()) {
                return Err(format!("unsupported embedding name {name}"));
            }
            if !seen.insert(name.as_str()) {
                return Err(format!("duplicate embedding name {name}"));
            }
        }
        Ok(())
    }
}

/// Snapshot configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct SnapshotsConfig {
    /// Whether to snapshot embeddings.
    pub embeddings: bool,
    /// Whether to write `graph.graphml`.
    pub graphml: bool,
    /// Whether to write raw extracted graph tables.
    #[serde(alias = "raw_graph")]
    pub raw_graph: bool,
}

/// Phase-1 `GraphRAG` configuration surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct GraphRagConfig {
    /// Completion model registry.
    #[serde(alias = "completion_models")]
    pub completion_models: BTreeMap<String, ModelConfig>,
    /// Embedding model registry.
    #[serde(alias = "embedding_models")]
    pub embedding_models: BTreeMap<String, ModelConfig>,
    /// Maximum concurrent LLM requests.
    #[serde(alias = "concurrent_requests")]
    pub concurrent_requests: usize,
    /// Input config.
    pub input: InputConfig,
    /// Input storage config.
    #[serde(alias = "input_storage")]
    pub input_storage: StorageConfig,
    /// Output storage config.
    #[serde(alias = "output_storage", default = "StorageConfig::output_default")]
    pub output_storage: StorageConfig,
    /// Reporting config.
    pub reporting: ReportingConfig,
    /// Cache config.
    pub cache: CacheConfig,
    /// Chunking config.
    pub chunking: ChunkingConfig,
    /// Configured workflow order. Empty means standard order.
    pub workflows: Vec<String>,
    /// Extract graph config.
    #[serde(alias = "extract_graph")]
    pub extract_graph: ExtractGraphConfig,
    /// Description summarization config.
    #[serde(alias = "summarize_descriptions")]
    pub summarize_descriptions: SummarizeDescriptionsConfig,
    /// Claim extraction config.
    #[serde(alias = "extract_claims")]
    pub extract_claims: ExtractClaimsConfig,
    /// Graph clustering config.
    #[serde(alias = "cluster_graph")]
    pub cluster_graph: ClusterGraphConfig,
    /// Community report generation config.
    #[serde(alias = "community_reports")]
    pub community_reports: CommunityReportsConfig,
    /// Text embedding generation config.
    #[serde(alias = "embed_text")]
    pub embed_text: EmbedTextConfig,
    /// Vector store config.
    #[serde(alias = "vector_store")]
    pub vector_store: VectorStoreConfig,
    /// Snapshot config.
    pub snapshots: SnapshotsConfig,
    /// Future-compatible sections retained as dynamic values.
    #[serde(flatten)]
    pub sections: BTreeMap<String, Value>,
}

impl Default for GraphRagConfig {
    fn default() -> Self {
        Self {
            completion_models: BTreeMap::new(),
            embedding_models: BTreeMap::new(),
            concurrent_requests: DEFAULT_CONCURRENT_REQUESTS,
            input: InputConfig::default(),
            input_storage: StorageConfig::default(),
            output_storage: StorageConfig::output_default(),
            reporting: ReportingConfig::default(),
            cache: CacheConfig::default(),
            chunking: ChunkingConfig::default(),
            workflows: Vec::new(),
            extract_graph: ExtractGraphConfig::default(),
            summarize_descriptions: SummarizeDescriptionsConfig::default(),
            extract_claims: ExtractClaimsConfig::default(),
            cluster_graph: ClusterGraphConfig::default(),
            community_reports: CommunityReportsConfig::default(),
            embed_text: EmbedTextConfig::default(),
            vector_store: VectorStoreConfig::default(),
            snapshots: SnapshotsConfig::default(),
            sections: BTreeMap::new(),
        }
    }
}

impl GraphRagConfig {
    /// Return the configured workflow order or the standard indexing workflow order.
    #[must_use]
    pub fn workflow_order(&self) -> Vec<String> {
        if self.workflows.is_empty() {
            crate::workflows::STEP9_WORKFLOWS
                .iter()
                .map(|workflow| (*workflow).to_owned())
                .collect()
        } else {
            self.workflows.clone()
        }
    }

    /// Validate Step 9 configuration.
    ///
    /// # Errors
    ///
    /// Returns an error message for invalid embedding or vector-store settings.
    pub fn validate_embed_text(&self) -> std::result::Result<(), String> {
        self.embed_text.validate()?;
        self.vector_store
            .validate()
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}
