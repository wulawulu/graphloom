//! GraphRAG-compatible configuration models used by Step 5.

use std::collections::BTreeMap;

use graphloom_chunking::ChunkingConfig;
use graphloom_llm::ModelConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;

fn default_concurrent_requests() -> usize {
    25
}

fn default_async_mode() -> String {
    "asyncio".to_owned()
}

fn default_input() -> InputConfig {
    InputConfig::default()
}

fn default_chunking() -> ChunkingConfig {
    ChunkingConfig::default()
}

fn default_extract_graph() -> ExtractGraphConfig {
    ExtractGraphConfig::default()
}

fn default_summarize_descriptions() -> SummarizeDescriptionsConfig {
    SummarizeDescriptionsConfig::default()
}

fn default_extract_claims() -> ExtractClaimsConfig {
    ExtractClaimsConfig::default()
}

fn default_cluster_graph() -> ClusterGraphConfig {
    ClusterGraphConfig::default()
}

fn default_snapshots() -> SnapshotsConfig {
    SnapshotsConfig::default()
}

fn default_completion_model_id() -> String {
    "default_completion_model".to_owned()
}

fn default_extract_graph_model_instance_name() -> String {
    "extract_graph".to_owned()
}

fn default_summarize_model_instance_name() -> String {
    "summarize_descriptions".to_owned()
}

fn default_extract_claims_model_instance_name() -> String {
    "extract_claims".to_owned()
}

fn default_claim_description() -> String {
    "Any claims or facts that could be relevant to information discovery.".to_owned()
}

fn default_entity_types() -> Vec<String> {
    ["organization", "person", "geo", "event"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// Input reader configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct InputConfig {
    /// Input reader type.
    #[serde(rename = "type", default = "InputConfig::default_type")]
    pub input_type: String,
    /// File pattern for file readers.
    #[serde(alias = "file_pattern", default = "InputConfig::default_file_pattern")]
    pub file_pattern: String,
    /// Text column for structured inputs.
    #[serde(alias = "text_column", default = "InputConfig::default_text_column")]
    pub text_column: String,
    /// Title column for structured inputs.
    #[serde(alias = "title_column", default = "InputConfig::default_title_column")]
    pub title_column: String,
    /// Metadata fields to collect from structured inputs.
    #[serde(default)]
    pub metadata: Vec<String>,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            input_type: Self::default_type(),
            file_pattern: Self::default_file_pattern(),
            text_column: Self::default_text_column(),
            title_column: Self::default_title_column(),
            metadata: Vec::new(),
        }
    }
}

impl InputConfig {
    fn default_type() -> String {
        "file".to_owned()
    }

    fn default_file_pattern() -> String {
        r".*\.txt$".to_owned()
    }

    fn default_text_column() -> String {
        "text".to_owned()
    }

    fn default_title_column() -> String {
        "title".to_owned()
    }
}

/// Graph extraction configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct ExtractGraphConfig {
    /// Completion model id.
    #[serde(alias = "completion_model_id", default = "default_completion_model_id")]
    pub completion_model_id: String,
    /// Model instance/cache namespace name.
    #[serde(
        alias = "model_instance_name",
        default = "default_extract_graph_model_instance_name"
    )]
    pub model_instance_name: String,
    /// Optional prompt path or inline prompt override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
            completion_model_id: default_completion_model_id(),
            model_instance_name: default_extract_graph_model_instance_name(),
            prompt: None,
            entity_types: default_entity_types(),
            max_gleanings: 1,
        }
    }
}

/// Description summarization configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct SummarizeDescriptionsConfig {
    /// Completion model id.
    #[serde(alias = "completion_model_id", default = "default_completion_model_id")]
    pub completion_model_id: String,
    /// Model instance/cache namespace name.
    #[serde(
        alias = "model_instance_name",
        default = "default_summarize_model_instance_name"
    )]
    pub model_instance_name: String,
    /// Optional prompt path or inline prompt override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
            completion_model_id: default_completion_model_id(),
            model_instance_name: default_summarize_model_instance_name(),
            prompt: None,
            max_length: 500,
            max_input_tokens: 4_000,
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
    #[serde(alias = "completion_model_id", default = "default_completion_model_id")]
    pub completion_model_id: String,
    /// Model instance/cache namespace name.
    #[serde(
        alias = "model_instance_name",
        default = "default_extract_claims_model_instance_name"
    )]
    pub model_instance_name: String,
    /// Optional prompt path or inline prompt override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Claim description inserted into the extraction prompt.
    #[serde(default = "default_claim_description")]
    pub description: String,
    /// Maximum number of claim gleaning rounds.
    #[serde(alias = "max_gleanings")]
    pub max_gleanings: usize,
}

impl Default for ExtractClaimsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            completion_model_id: default_completion_model_id(),
            model_instance_name: default_extract_claims_model_instance_name(),
            prompt: None,
            description: default_claim_description(),
            max_gleanings: 1,
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
            max_cluster_size: 10,
            use_lcc: true,
            seed: 0xDEADBEEF,
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

/// Phase-1 GraphRAG configuration surface.
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
    #[serde(alias = "concurrent_requests", default = "default_concurrent_requests")]
    pub concurrent_requests: usize,
    /// Async mode compatibility field.
    #[serde(alias = "async_mode", default = "default_async_mode")]
    pub async_mode: String,
    /// Input config.
    #[serde(default = "default_input")]
    pub input: InputConfig,
    /// Chunking config.
    #[serde(default = "default_chunking")]
    pub chunking: ChunkingConfig,
    /// Configured workflow order. Empty means standard order.
    #[serde(default)]
    pub workflows: Vec<String>,
    /// Extract graph config.
    #[serde(alias = "extract_graph", default = "default_extract_graph")]
    pub extract_graph: ExtractGraphConfig,
    /// Description summarization config.
    #[serde(
        alias = "summarize_descriptions",
        default = "default_summarize_descriptions"
    )]
    pub summarize_descriptions: SummarizeDescriptionsConfig,
    /// Claim extraction config.
    #[serde(alias = "extract_claims", default = "default_extract_claims")]
    pub extract_claims: ExtractClaimsConfig,
    /// Graph clustering config.
    #[serde(alias = "cluster_graph", default = "default_cluster_graph")]
    pub cluster_graph: ClusterGraphConfig,
    /// Snapshot config.
    #[serde(default = "default_snapshots")]
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
            concurrent_requests: default_concurrent_requests(),
            async_mode: default_async_mode(),
            input: default_input(),
            chunking: default_chunking(),
            workflows: Vec::new(),
            extract_graph: default_extract_graph(),
            summarize_descriptions: default_summarize_descriptions(),
            extract_claims: default_extract_claims(),
            cluster_graph: default_cluster_graph(),
            snapshots: default_snapshots(),
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
