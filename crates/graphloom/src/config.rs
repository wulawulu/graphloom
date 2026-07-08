//! GraphRAG-compatible configuration models used by Step 5.

use std::collections::BTreeMap;

use graphloom_chunking::ChunkingConfig;
use graphloom_llm::ModelConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEFAULT_CONCURRENT_REQUESTS: usize = 25;
const DEFAULT_COMPLETION_MODEL_ID: &str = "default_completion_model";
const DEFAULT_EXTRACT_GRAPH_MODEL_INSTANCE_NAME: &str = "extract_graph";
const DEFAULT_SUMMARIZE_MODEL_INSTANCE_NAME: &str = "summarize_descriptions";
const DEFAULT_EXTRACT_CLAIMS_MODEL_INSTANCE_NAME: &str = "extract_claims";
const DEFAULT_CLAIM_DESCRIPTION: &str =
    "Any claims or facts that could be relevant to information discovery.";
const DEFAULT_INPUT_TYPE: &str = "file";
const DEFAULT_FILE_PATTERN: &str = r".*\.txt$";
const DEFAULT_TEXT_COLUMN: &str = "text";
const DEFAULT_TITLE_COLUMN: &str = "title";
const DEFAULT_MAX_GLEANINGS: usize = 1;
const DEFAULT_MAX_SUMMARY_LENGTH: usize = 500;
const DEFAULT_MAX_INPUT_TOKENS: usize = 4_000;
const DEFAULT_MAX_CLUSTER_SIZE: u32 = 10;
const DEFAULT_CLUSTER_SEED: u64 = 0xDEAD_BEEF;

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
    /// Optional prompt path or inline prompt override.
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
    /// Optional prompt path or inline prompt override.
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
    /// Optional prompt path or inline prompt override.
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
            chunking: ChunkingConfig::default(),
            workflows: Vec::new(),
            extract_graph: ExtractGraphConfig::default(),
            summarize_descriptions: SummarizeDescriptionsConfig::default(),
            extract_claims: ExtractClaimsConfig::default(),
            cluster_graph: ClusterGraphConfig::default(),
            snapshots: SnapshotsConfig::default(),
            sections: BTreeMap::new(),
        }
    }
}

impl GraphRagConfig {
    /// Return the configured workflow order or the standard Step-5 prefix.
    #[must_use]
    pub fn workflow_order(&self) -> Vec<String> {
        if self.workflows.is_empty() {
            crate::workflows::STEP7_WORKFLOWS
                .iter()
                .map(|workflow| (*workflow).to_owned())
                .collect()
        } else {
            self.workflows.clone()
        }
    }
}
