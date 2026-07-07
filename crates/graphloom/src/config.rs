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
            sections: BTreeMap::new(),
        }
    }
}

impl GraphRagConfig {
    /// Return the configured workflow order or the standard Step-5 prefix.
    #[must_use]
    pub fn workflow_order(&self) -> Vec<String> {
        if self.workflows.is_empty() {
            crate::workflows::STEP5_WORKFLOWS
                .iter()
                .map(|workflow| (*workflow).to_owned())
                .collect()
        } else {
            self.workflows.clone()
        }
    }
}
