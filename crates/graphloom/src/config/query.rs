//! Strongly typed `GraphRAG` 3.1 query configuration.

use serde::{Deserialize, Serialize};

const DEFAULT_COMPLETION_MODEL_ID: &str = "default_completion_model";
const DEFAULT_EMBEDDING_MODEL_ID: &str = "default_embedding_model";

fn completion_model_id() -> String {
    DEFAULT_COMPLETION_MODEL_ID.to_owned()
}

fn embedding_model_id() -> String {
    DEFAULT_EMBEDDING_MODEL_ID.to_owned()
}

fn validate_model_id(value: &str, field: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{field} must not be empty"))
    } else {
        Ok(())
    }
}

fn validate_positive(value: usize, field: &str) -> Result<(), String> {
    if value == 0 {
        Err(format!("{field} must be greater than zero"))
    } else {
        Ok(())
    }
}

fn validate_optional_positive(value: Option<u32>, field: &str) -> Result<(), String> {
    if value == Some(0) {
        Err(format!("{field} must be greater than zero when configured"))
    } else {
        Ok(())
    }
}

fn validate_ratio(value: f64, field: &str) -> Result<(), String> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        Err(format!("{field} must be finite and between 0 and 1"))
    } else {
        Ok(())
    }
}

fn validate_temperature(value: f64, field: &str) -> Result<(), String> {
    if !value.is_finite() || !(0.0..=2.0).contains(&value) {
        Err(format!("{field} must be finite and between 0 and 2"))
    } else {
        Ok(())
    }
}

/// Local Search configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
#[non_exhaustive]
pub struct LocalSearchConfig {
    /// Optional local-search system prompt path or inline prompt.
    pub prompt: Option<String>,
    /// Completion model identifier.
    #[serde(alias = "completionModelId")]
    pub completion_model_id: String,
    /// Embedding model identifier.
    #[serde(alias = "embeddingModelId")]
    pub embedding_model_id: String,
    /// Fraction of context allocated to text units.
    #[serde(alias = "textUnitProp")]
    pub text_unit_prop: f64,
    /// Fraction of context allocated to community reports.
    #[serde(alias = "communityProp")]
    pub community_prop: f64,
    /// Maximum prior conversation turns.
    #[serde(alias = "conversationHistoryMaxTurns")]
    pub conversation_history_max_turns: usize,
    /// Number of mapped entities.
    #[serde(alias = "topKEntities")]
    pub top_k_entities: usize,
    /// Number of related relationships.
    #[serde(alias = "topKRelationships")]
    pub top_k_relationships: usize,
    /// Maximum context tokens.
    #[serde(alias = "maxContextTokens")]
    pub max_context_tokens: usize,
}

impl Default for LocalSearchConfig {
    fn default() -> Self {
        Self {
            prompt: None,
            completion_model_id: completion_model_id(),
            embedding_model_id: embedding_model_id(),
            text_unit_prop: 0.5,
            community_prop: 0.15,
            conversation_history_max_turns: 5,
            top_k_entities: 10,
            top_k_relationships: 10,
            max_context_tokens: 12_000,
        }
    }
}

impl LocalSearchConfig {
    /// Validate local-search values without resolving model registries.
    ///
    /// # Errors
    ///
    /// Returns a message identifying the first invalid value.
    pub fn validate(&self) -> Result<(), String> {
        validate_model_id(&self.completion_model_id, "completion_model_id")?;
        validate_model_id(&self.embedding_model_id, "embedding_model_id")?;
        validate_ratio(self.text_unit_prop, "text_unit_prop")?;
        validate_ratio(self.community_prop, "community_prop")?;
        if self.text_unit_prop + self.community_prop > 1.0 {
            return Err("text_unit_prop + community_prop must not exceed 1".to_owned());
        }
        validate_positive(self.top_k_entities, "top_k_entities")?;
        validate_positive(self.top_k_relationships, "top_k_relationships")?;
        validate_positive(self.max_context_tokens, "max_context_tokens")
    }
}

/// Global Search configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
#[non_exhaustive]
pub struct GlobalSearchConfig {
    /// Map prompt path or inline prompt.
    #[serde(alias = "mapPrompt")]
    pub map_prompt: Option<String>,
    /// Reduce prompt path or inline prompt.
    #[serde(alias = "reducePrompt")]
    pub reduce_prompt: Option<String>,
    /// General-knowledge prompt path or inline prompt.
    #[serde(alias = "knowledgePrompt")]
    pub knowledge_prompt: Option<String>,
    /// Completion model identifier.
    #[serde(alias = "completionModelId")]
    pub completion_model_id: String,
    /// Maximum tokens per map context.
    #[serde(alias = "maxContextTokens")]
    pub max_context_tokens: usize,
    /// Maximum tokens supplied to reduce.
    #[serde(alias = "dataMaxTokens")]
    pub data_max_tokens: usize,
    /// Maximum map response length in words.
    #[serde(alias = "mapMaxLength")]
    pub map_max_length: usize,
    /// Maximum reduce response length in words.
    #[serde(alias = "reduceMaxLength")]
    pub reduce_max_length: usize,
    /// Dynamic-selection relevance threshold.
    #[serde(alias = "dynamicSearchThreshold")]
    pub dynamic_search_threshold: i64,
    /// Whether dynamic selection keeps relevant parents.
    #[serde(alias = "dynamicSearchKeepParent")]
    pub dynamic_search_keep_parent: bool,
    /// Repeated ratings per report.
    #[serde(alias = "dynamicSearchNumRepeats")]
    pub dynamic_search_num_repeats: usize,
    /// Whether dynamic selection rates summaries.
    #[serde(alias = "dynamicSearchUseSummary")]
    pub dynamic_search_use_summary: bool,
    /// Deepest fallback hierarchy level.
    #[serde(alias = "dynamicSearchMaxLevel")]
    pub dynamic_search_max_level: i64,
}

impl Default for GlobalSearchConfig {
    fn default() -> Self {
        Self {
            map_prompt: None,
            reduce_prompt: None,
            knowledge_prompt: None,
            completion_model_id: completion_model_id(),
            max_context_tokens: 12_000,
            data_max_tokens: 12_000,
            map_max_length: 1_000,
            reduce_max_length: 2_000,
            dynamic_search_threshold: 1,
            dynamic_search_keep_parent: false,
            dynamic_search_num_repeats: 1,
            dynamic_search_use_summary: false,
            dynamic_search_max_level: 2,
        }
    }
}

impl GlobalSearchConfig {
    /// Validate global-search values without resolving model registries.
    ///
    /// # Errors
    ///
    /// Returns a message identifying the first invalid value.
    pub fn validate(&self) -> Result<(), String> {
        validate_model_id(&self.completion_model_id, "completion_model_id")?;
        validate_positive(self.max_context_tokens, "max_context_tokens")?;
        validate_positive(self.data_max_tokens, "data_max_tokens")?;
        validate_positive(self.map_max_length, "map_max_length")?;
        validate_positive(self.reduce_max_length, "reduce_max_length")?;
        validate_positive(
            self.dynamic_search_num_repeats,
            "dynamic_search_num_repeats",
        )?;
        if self.dynamic_search_max_level < 0 {
            return Err("dynamic_search_max_level must be non-negative".to_owned());
        }
        Ok(())
    }
}

/// Basic Search configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
#[non_exhaustive]
pub struct BasicSearchConfig {
    /// Basic system prompt path or inline prompt.
    pub prompt: Option<String>,
    /// Completion model identifier.
    #[serde(alias = "completionModelId")]
    pub completion_model_id: String,
    /// Embedding model identifier.
    #[serde(alias = "embeddingModelId")]
    pub embedding_model_id: String,
    /// Number of nearest text units.
    pub k: usize,
    /// Maximum context tokens.
    #[serde(alias = "maxContextTokens")]
    pub max_context_tokens: usize,
}

impl Default for BasicSearchConfig {
    fn default() -> Self {
        Self {
            prompt: None,
            completion_model_id: completion_model_id(),
            embedding_model_id: embedding_model_id(),
            k: 10,
            max_context_tokens: 12_000,
        }
    }
}

impl BasicSearchConfig {
    /// Validate basic-search values without resolving model registries.
    ///
    /// # Errors
    ///
    /// Returns a message identifying the first invalid value.
    pub fn validate(&self) -> Result<(), String> {
        validate_model_id(&self.completion_model_id, "completion_model_id")?;
        validate_model_id(&self.embedding_model_id, "embedding_model_id")?;
        validate_positive(self.k, "k")?;
        validate_positive(self.max_context_tokens, "max_context_tokens")
    }
}

/// DRIFT Search configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
#[non_exhaustive]
pub struct DriftSearchConfig {
    /// DRIFT local prompt path or inline prompt.
    pub prompt: Option<String>,
    /// DRIFT reduce prompt path or inline prompt.
    #[serde(alias = "reducePrompt")]
    pub reduce_prompt: Option<String>,
    /// Completion model identifier.
    #[serde(alias = "completionModelId")]
    pub completion_model_id: String,
    /// Embedding model identifier.
    #[serde(alias = "embeddingModelId")]
    pub embedding_model_id: String,
    /// Primer data token limit.
    #[serde(alias = "dataMaxTokens")]
    pub data_max_tokens: usize,
    /// Legacy reduce output token limit.
    #[serde(alias = "reduceMaxTokens")]
    pub reduce_max_tokens: Option<u32>,
    /// Reduce temperature.
    #[serde(alias = "reduceTemperature")]
    pub reduce_temperature: f64,
    /// Reduce completion token limit.
    #[serde(alias = "reduceMaxCompletionTokens")]
    pub reduce_max_completion_tokens: Option<u32>,
    /// DRIFT orchestration concurrency.
    pub concurrency: usize,
    /// Number of follow-up actions selected per depth.
    #[serde(alias = "driftKFollowups")]
    pub drift_k_followups: usize,
    /// Number of primer folds; zero retains upstream's effective one-fold behavior.
    #[serde(alias = "primerFolds")]
    pub primer_folds: usize,
    /// Primer input token limit.
    #[serde(alias = "primerLlmMaxTokens")]
    pub primer_llm_max_tokens: usize,
    /// Maximum action depth.
    #[serde(alias = "nDepth")]
    pub n_depth: usize,
    /// DRIFT-local text-unit proportion.
    #[serde(alias = "localSearchTextUnitProp")]
    pub local_search_text_unit_prop: f64,
    /// DRIFT-local community proportion.
    #[serde(alias = "localSearchCommunityProp")]
    pub local_search_community_prop: f64,
    /// DRIFT-local mapped entity count.
    #[serde(alias = "localSearchTopKMappedEntities")]
    pub local_search_top_k_mapped_entities: usize,
    /// DRIFT-local relationship count.
    #[serde(alias = "localSearchTopKRelationships")]
    pub local_search_top_k_relationships: usize,
    /// DRIFT-local data token limit.
    #[serde(alias = "localSearchMaxDataTokens")]
    pub local_search_max_data_tokens: usize,
    /// DRIFT-local temperature.
    #[serde(alias = "localSearchTemperature")]
    pub local_search_temperature: f64,
    /// DRIFT-local nucleus sampling value.
    #[serde(alias = "localSearchTopP")]
    pub local_search_top_p: f64,
    /// DRIFT-local completion count.
    #[serde(alias = "localSearchN")]
    pub local_search_n: usize,
    /// DRIFT-local legacy output token limit.
    #[serde(alias = "localSearchLlmMaxGenTokens")]
    pub local_search_llm_max_gen_tokens: Option<u32>,
    /// DRIFT-local completion token limit.
    #[serde(alias = "localSearchLlmMaxGenCompletionTokens")]
    pub local_search_llm_max_gen_completion_tokens: Option<u32>,
}

impl Default for DriftSearchConfig {
    fn default() -> Self {
        Self {
            prompt: None,
            reduce_prompt: None,
            completion_model_id: completion_model_id(),
            embedding_model_id: embedding_model_id(),
            data_max_tokens: 12_000,
            reduce_max_tokens: None,
            reduce_temperature: 0.0,
            reduce_max_completion_tokens: None,
            concurrency: 32,
            drift_k_followups: 20,
            primer_folds: 5,
            primer_llm_max_tokens: 12_000,
            n_depth: 3,
            local_search_text_unit_prop: 0.9,
            local_search_community_prop: 0.1,
            local_search_top_k_mapped_entities: 10,
            local_search_top_k_relationships: 10,
            local_search_max_data_tokens: 12_000,
            local_search_temperature: 0.0,
            local_search_top_p: 1.0,
            local_search_n: 1,
            local_search_llm_max_gen_tokens: None,
            local_search_llm_max_gen_completion_tokens: None,
        }
    }
}

impl DriftSearchConfig {
    /// Return the effective primer fold count used by `GraphRAG`.
    #[must_use]
    pub fn effective_primer_folds(&self) -> usize {
        self.primer_folds.max(1)
    }

    /// Validate DRIFT values without resolving model registries.
    ///
    /// # Errors
    ///
    /// Returns a message identifying the first invalid value.
    pub fn validate(&self) -> Result<(), String> {
        validate_model_id(&self.completion_model_id, "completion_model_id")?;
        validate_model_id(&self.embedding_model_id, "embedding_model_id")?;
        validate_positive(self.data_max_tokens, "data_max_tokens")?;
        validate_optional_positive(self.reduce_max_tokens, "reduce_max_tokens")?;
        validate_temperature(self.reduce_temperature, "reduce_temperature")?;
        validate_optional_positive(
            self.reduce_max_completion_tokens,
            "reduce_max_completion_tokens",
        )?;
        validate_positive(self.concurrency, "concurrency")?;
        validate_positive(self.drift_k_followups, "drift_k_followups")?;
        validate_positive(self.primer_llm_max_tokens, "primer_llm_max_tokens")?;
        validate_positive(self.n_depth, "n_depth")?;
        validate_ratio(
            self.local_search_text_unit_prop,
            "local_search_text_unit_prop",
        )?;
        validate_ratio(
            self.local_search_community_prop,
            "local_search_community_prop",
        )?;
        if self.local_search_text_unit_prop + self.local_search_community_prop > 1.0 {
            return Err(
                "local_search_text_unit_prop + local_search_community_prop must not exceed 1"
                    .to_owned(),
            );
        }
        validate_positive(
            self.local_search_top_k_mapped_entities,
            "local_search_top_k_mapped_entities",
        )?;
        validate_positive(
            self.local_search_top_k_relationships,
            "local_search_top_k_relationships",
        )?;
        validate_positive(
            self.local_search_max_data_tokens,
            "local_search_max_data_tokens",
        )?;
        validate_temperature(self.local_search_temperature, "local_search_temperature")?;
        validate_ratio(self.local_search_top_p, "local_search_top_p")?;
        validate_positive(self.local_search_n, "local_search_n")?;
        validate_optional_positive(
            self.local_search_llm_max_gen_tokens,
            "local_search_llm_max_gen_tokens",
        )?;
        validate_optional_positive(
            self.local_search_llm_max_gen_completion_tokens,
            "local_search_llm_max_gen_completion_tokens",
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn assert_f64_eq(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < f64::EPSILON);
    }

    #[test]
    fn test_should_match_graphrag_query_defaults() {
        let local = LocalSearchConfig::default();
        assert_f64_eq(local.text_unit_prop, 0.5);
        assert_f64_eq(local.community_prop, 0.15);
        assert_eq!(local.top_k_entities, 10);
        assert_eq!(local.max_context_tokens, 12_000);
        let global = GlobalSearchConfig::default();
        assert_eq!(global.data_max_tokens, 12_000);
        assert_eq!(global.map_max_length, 1_000);
        assert_eq!(global.reduce_max_length, 2_000);
        assert_eq!(global.dynamic_search_max_level, 2);
        let basic = BasicSearchConfig::default();
        assert_eq!(basic.k, 10);
        assert_eq!(basic.max_context_tokens, 12_000);
        let drift = DriftSearchConfig::default();
        assert_eq!(drift.effective_primer_folds(), 5);
        assert_eq!(drift.concurrency, 32);
        assert_eq!(drift.n_depth, 3);
        assert_f64_eq(drift.local_search_top_p, 1.0);
    }

    #[test]
    fn test_should_accept_snake_case_and_legacy_camel_case() {
        let snake: BasicSearchConfig = serde_json::from_value(json!({
            "completion_model_id": "chat",
            "embedding_model_id": "embed",
            "max_context_tokens": 42,
            "k": 3
        }))
        .expect("snake case");
        let camel: BasicSearchConfig = serde_json::from_value(json!({
            "completionModelId": "chat",
            "embeddingModelId": "embed",
            "maxContextTokens": 42,
            "k": 3
        }))
        .expect("camel case");
        assert_eq!(snake, camel);

        let local_snake: LocalSearchConfig = serde_json::from_value(json!({
            "completion_model_id": "chat",
            "embedding_model_id": "embed",
            "text_unit_prop": 0.4,
            "top_k_relationships": 7
        }))
        .expect("local snake case");
        let local_camel: LocalSearchConfig = serde_json::from_value(json!({
            "completionModelId": "chat",
            "embeddingModelId": "embed",
            "textUnitProp": 0.4,
            "topKRelationships": 7
        }))
        .expect("local camel case");
        assert_eq!(local_snake, local_camel);

        let global_camel: GlobalSearchConfig = serde_json::from_value(json!({
            "completionModelId": "chat",
            "mapPrompt": "map.txt",
            "dataMaxTokens": 99
        }))
        .expect("global camel case");
        assert_eq!(global_camel.completion_model_id, "chat");
        assert_eq!(global_camel.map_prompt.as_deref(), Some("map.txt"));
        assert_eq!(global_camel.data_max_tokens, 99);

        let drift_camel: DriftSearchConfig = serde_json::from_value(json!({
            "completionModelId": "chat",
            "embeddingModelId": "embed",
            "reduceTemperature": 0.5,
            "localSearchTopP": 0.7
        }))
        .expect("DRIFT camel case");
        assert_f64_eq(drift_camel.reduce_temperature, 0.5);
        assert_f64_eq(drift_camel.local_search_top_p, 0.7);
    }

    #[test]
    fn test_should_round_trip_query_configs_through_yaml_and_json() {
        let original = DriftSearchConfig::default();
        let yaml = serde_yaml::to_string(&original).expect("yaml serialize");
        assert!(yaml.contains("completion_model_id:"));
        assert_eq!(
            serde_yaml::from_str::<DriftSearchConfig>(&yaml).expect("yaml deserialize"),
            original
        );
        let json = serde_json::to_string(&original).expect("json serialize");
        assert_eq!(
            serde_json::from_str::<DriftSearchConfig>(&json).expect("json deserialize"),
            original
        );
    }

    #[test]
    fn test_should_reject_invalid_query_config_values() {
        let local = LocalSearchConfig {
            text_unit_prop: f64::NAN,
            ..LocalSearchConfig::default()
        };
        assert!(local.validate().is_err());
        let local = LocalSearchConfig {
            top_k_entities: 0,
            ..LocalSearchConfig::default()
        };
        assert!(local.validate().is_err());

        let basic = BasicSearchConfig {
            max_context_tokens: 0,
            ..BasicSearchConfig::default()
        };
        assert!(basic.validate().is_err());
        let basic = BasicSearchConfig {
            completion_model_id: String::new(),
            ..BasicSearchConfig::default()
        };
        assert!(basic.validate().is_err());

        let drift = DriftSearchConfig {
            local_search_top_p: f64::INFINITY,
            ..DriftSearchConfig::default()
        };
        assert!(drift.validate().is_err());
        let drift = DriftSearchConfig {
            concurrency: 0,
            ..DriftSearchConfig::default()
        };
        assert!(drift.validate().is_err());
        let drift = DriftSearchConfig {
            reduce_max_completion_tokens: Some(0),
            ..DriftSearchConfig::default()
        };
        assert!(drift.validate().is_err());

        let global = GlobalSearchConfig {
            data_max_tokens: 0,
            ..GlobalSearchConfig::default()
        };
        assert!(global.validate().is_err());
        let global = GlobalSearchConfig {
            completion_model_id: "  ".to_owned(),
            ..GlobalSearchConfig::default()
        };
        assert!(global.validate().is_err());
    }

    #[test]
    fn test_should_treat_zero_primer_folds_as_one() {
        let config = DriftSearchConfig {
            primer_folds: 0,
            ..DriftSearchConfig::default()
        };
        assert_eq!(config.effective_primer_folds(), 1);
        assert!(config.validate().is_ok());
    }
}
