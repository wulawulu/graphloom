//! Versioned explainability event vocabulary and named payloads.

use serde::{Deserialize, Serialize};

use super::{
    ContextSectionKind, ExplainabilityCandidate, ExplainabilityContentMode,
    ExplainabilityContextSection, ExplainabilityQueryMethod, ExplainabilityRecordType,
    ExplainabilityRunKind,
};

/// A run entered execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunStarted {
    /// Operation category.
    pub kind: ExplainabilityRunKind,
    /// Content-disclosure mode selected for the run.
    pub content_mode: ExplainabilityContentMode,
}

impl RunStarted {
    /// Create a run-start payload.
    #[must_use]
    pub const fn new(kind: ExplainabilityRunKind, content_mode: ExplainabilityContentMode) -> Self {
        Self { kind, content_mode }
    }
}

/// A run completed successfully.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunCompleted {
    /// Total run latency in milliseconds.
    pub elapsed_ms: u64,
}

impl RunCompleted {
    /// Create a successful run-completion payload.
    #[must_use]
    pub const fn new(elapsed_ms: u64) -> Self {
        Self { elapsed_ms }
    }
}

/// A run terminated with a safe, displayable error summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunFailed {
    /// Stable low-cardinality error category.
    #[serde(with = "super::validation::metadata_string")]
    pub error_kind: String,
    /// Redacted human-readable message without request debug data or credentials.
    #[serde(with = "super::validation::message_string")]
    pub message: String,
}

impl RunFailed {
    /// Create a failed-run payload from already-redacted values.
    #[must_use]
    pub fn new(error_kind: String, message: String) -> Self {
        Self {
            error_kind,
            message,
        }
    }
}

/// Query execution started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct QueryStarted {
    /// Stable query method.
    pub method: ExplainabilityQueryMethod,
    /// Full user query when the content mode permits it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub query: Option<String>,
}

impl QueryStarted {
    /// Create metadata-only query-start payload.
    #[must_use]
    pub const fn new(method: ExplainabilityQueryMethod) -> Self {
        Self {
            method,
            query: None,
        }
    }
}

/// Local Search constructed the query used for entity mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MappingQueryBuilt {
    /// Number of conversation turns considered when building the mapping query.
    pub conversation_turn_count: u32,
    /// Full mapping query when the content mode permits it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub mapping_query: Option<String>,
}

impl MappingQueryBuilt {
    /// Create metadata-only mapping-query payload.
    #[must_use]
    pub const fn new(conversation_turn_count: u32) -> Self {
        Self {
            conversation_turn_count,
            mapping_query: None,
        }
    }
}

/// Query embedding request started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EmbeddingStarted {
    /// Configured embedding-model identifier.
    #[serde(with = "super::validation::metadata_string")]
    pub model_id: String,
    /// Full embedding input when the content mode permits it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub input: Option<String>,
}

impl EmbeddingStarted {
    /// Create metadata-only embedding-start payload.
    #[must_use]
    pub fn new(model_id: String) -> Self {
        Self {
            model_id,
            input: None,
        }
    }
}

/// Query embedding request completed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EmbeddingCompleted {
    /// Configured embedding-model identifier.
    #[serde(with = "super::validation::metadata_string")]
    pub model_id: String,
    /// Input token count resolved from provider usage or the configured tokenizer.
    pub prompt_tokens: u64,
    /// Returned embedding dimensions.
    pub dimensions: u32,
}

impl EmbeddingCompleted {
    /// Create an embedding-completion payload.
    #[must_use]
    pub fn new(model_id: String, prompt_tokens: u64, dimensions: u32) -> Self {
        Self {
            model_id,
            prompt_tokens,
            dimensions,
        }
    }
}

/// ANN or other retrieval returned candidate records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CandidatesRetrieved {
    /// Category shared by the returned candidates.
    pub record_type: ExplainabilityRecordType,
    /// Candidates in provider result order.
    #[serde(default, with = "super::validation::candidates")]
    pub candidates: Vec<ExplainabilityCandidate>,
}

impl CandidatesRetrieved {
    /// Create a retrieval payload.
    #[must_use]
    pub fn new(
        record_type: ExplainabilityRecordType,
        candidates: Vec<ExplainabilityCandidate>,
    ) -> Self {
        Self {
            record_type,
            candidates,
        }
    }
}

/// Candidate filters produced their accepted and rejected result metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CandidatesFiltered {
    /// Category shared by the filtered candidates.
    pub record_type: ExplainabilityRecordType,
    /// Candidates annotated with actual selection state and reason where known.
    #[serde(default, with = "super::validation::candidates")]
    pub candidates: Vec<ExplainabilityCandidate>,
}

impl CandidatesFiltered {
    /// Create a candidate-filter payload.
    #[must_use]
    pub fn new(
        record_type: ExplainabilityRecordType,
        candidates: Vec<ExplainabilityCandidate>,
    ) -> Self {
        Self {
            record_type,
            candidates,
        }
    }
}

macro_rules! candidate_selection_payload {
    ($name:ident, $field:ident, $docs:literal, $field_docs:literal) => {
        #[doc = $docs]
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[non_exhaustive]
        pub struct $name {
            #[doc = $field_docs]
            #[serde(default, with = "super::validation::candidates")]
            pub $field: Vec<ExplainabilityCandidate>,
        }

        impl $name {
            #[doc = concat!("Create a `", stringify!($name), "` payload.")]
            #[must_use]
            pub fn new($field: Vec<ExplainabilityCandidate>) -> Self {
                Self { $field }
            }
        }
    };
}

candidate_selection_payload!(
    EntitiesSelected,
    entities,
    "Entity candidates selected for Local Search context construction.",
    "Selected entities in effective order."
);

/// Graph-neighborhood expansion started from selected entity records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct GraphExpansionStarted {
    /// Seed entity identifiers in effective order.
    #[serde(default, with = "super::validation::record_ids")]
    pub seed_entity_ids: Vec<String>,
}

impl GraphExpansionStarted {
    /// Create a graph-expansion payload.
    #[must_use]
    pub fn new(seed_entity_ids: Vec<String>) -> Self {
        Self { seed_entity_ids }
    }
}

candidate_selection_payload!(
    RelationshipsSelected,
    relationships,
    "Relationships selected during graph expansion.",
    "Selected relationship metadata in context order."
);
candidate_selection_payload!(
    CommunityReportsSelected,
    community_reports,
    "Community reports selected through entity membership.",
    "Selected community-report metadata in context order."
);
candidate_selection_payload!(
    CovariatesSelected,
    covariates,
    "Covariates selected through entity references.",
    "Selected covariate metadata in context order."
);
candidate_selection_payload!(
    TextUnitsSelected,
    text_units,
    "Source text units selected through entity references.",
    "Selected text-unit metadata in context order."
);

/// Token budget assigned to one context section before construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ContextSectionBudget {
    /// Logical context section.
    pub section: ContextSectionKind,
    /// Assigned maximum token count.
    pub token_budget: u64,
}

impl ContextSectionBudget {
    /// Create a section-budget entry.
    #[must_use]
    pub const fn new(section: ContextSectionKind, token_budget: u64) -> Self {
        Self {
            section,
            token_budget,
        }
    }
}

/// Context builder allocated its overall and per-section budgets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ContextBudgetAllocated {
    /// Overall configured context budget.
    pub total_token_budget: u64,
    /// Per-section budgets in construction order.
    #[serde(default, with = "super::validation::context_sections")]
    pub sections: Vec<ContextSectionBudget>,
}

impl ContextBudgetAllocated {
    /// Create a context-budget payload.
    #[must_use]
    pub fn new(total_token_budget: u64, sections: Vec<ContextSectionBudget>) -> Self {
        Self {
            total_token_budget,
            sections,
        }
    }
}

/// One context section completed construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ContextSectionBuilt {
    /// Stable section summary.
    pub section: ExplainabilityContextSection,
}

impl ContextSectionBuilt {
    /// Create a context-section event payload.
    #[must_use]
    pub const fn new(section: ExplainabilityContextSection) -> Self {
        Self { section }
    }
}

/// All query-context sections completed construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ContextCompleted {
    /// Tokens consumed by the completed context.
    pub tokens_used: u64,
    /// Full context when the content mode permits it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub context: Option<String>,
}

impl ContextCompleted {
    /// Create metadata-only completed-context payload.
    #[must_use]
    pub const fn new(tokens_used: u64) -> Self {
        Self {
            tokens_used,
            context: None,
        }
    }
}

/// Completion-model request started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LlmRequestStarted {
    /// Configured completion-model identifier.
    #[serde(with = "super::validation::metadata_string")]
    pub model_id: String,
    /// Counted request input tokens.
    pub prompt_tokens: u64,
    /// Full rendered prompt when the content mode permits it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub prompt: Option<String>,
}

impl LlmRequestStarted {
    /// Create metadata-only LLM-request payload.
    #[must_use]
    pub fn new(model_id: String, prompt_tokens: u64) -> Self {
        Self {
            model_id,
            prompt_tokens,
            prompt: None,
        }
    }
}

/// Completion-model request completed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LlmRequestCompleted {
    /// Configured completion-model identifier.
    #[serde(with = "super::validation::metadata_string")]
    pub model_id: String,
    /// Counted request input tokens.
    pub input_tokens: u64,
    /// Counted generated tokens.
    pub output_tokens: u64,
    /// Model-call latency in milliseconds.
    pub elapsed_ms: u64,
    /// Full model response when the content mode permits it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub response: Option<String>,
}

impl LlmRequestCompleted {
    /// Create metadata-only LLM-completion payload.
    #[must_use]
    pub fn new(model_id: String, input_tokens: u64, output_tokens: u64, elapsed_ms: u64) -> Self {
        Self {
            model_id,
            input_tokens,
            output_tokens,
            elapsed_ms,
            response: None,
        }
    }
}

/// Safe, user-displayable non-fatal diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExplainabilityWarning {
    /// Stable low-cardinality warning code.
    #[serde(with = "super::validation::metadata_string")]
    pub code: String,
    /// Redacted human-readable warning message.
    #[serde(with = "super::validation::message_string")]
    pub message: String,
    /// Related record identifier when known.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_metadata_string"
    )]
    pub record_id: Option<String>,
}

impl ExplainabilityWarning {
    /// Create a warning without a related record.
    #[must_use]
    pub fn new(code: String, message: String) -> Self {
        Self {
            code,
            message,
            record_id: None,
        }
    }
}

/// Explainability event emitted by core orchestration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExplainabilityEvent {
    /// A run entered execution.
    RunStarted(RunStarted),
    /// A run completed successfully.
    RunCompleted(RunCompleted),
    /// A run failed.
    RunFailed(RunFailed),
    /// Query execution started.
    QueryStarted(QueryStarted),
    /// Local Search constructed its mapping query.
    MappingQueryBuilt(MappingQueryBuilt),
    /// Query embedding started.
    EmbeddingStarted(EmbeddingStarted),
    /// Query embedding completed.
    EmbeddingCompleted(EmbeddingCompleted),
    /// Retrieval returned candidates.
    CandidatesRetrieved(CandidatesRetrieved),
    /// Candidate filtering completed.
    CandidatesFiltered(CandidatesFiltered),
    /// Entity selection completed.
    EntitiesSelected(EntitiesSelected),
    /// Graph expansion started.
    GraphExpansionStarted(GraphExpansionStarted),
    /// Relationship selection completed.
    RelationshipsSelected(RelationshipsSelected),
    /// Community-report selection completed.
    CommunityReportsSelected(CommunityReportsSelected),
    /// Covariate selection completed.
    CovariatesSelected(CovariatesSelected),
    /// Text-unit selection completed.
    TextUnitsSelected(TextUnitsSelected),
    /// Context budgets were assigned.
    ContextBudgetAllocated(ContextBudgetAllocated),
    /// One context section completed.
    ContextSectionBuilt(ContextSectionBuilt),
    /// The complete context was built.
    ContextCompleted(ContextCompleted),
    /// Completion-model request started.
    LlmRequestStarted(LlmRequestStarted),
    /// Completion-model request completed.
    LlmRequestCompleted(LlmRequestCompleted),
    /// Non-fatal diagnostic occurred.
    Warning(ExplainabilityWarning),
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        CandidatesFiltered, CandidatesRetrieved, CommunityReportsSelected, ContextBudgetAllocated,
        ContextCompleted, ContextSectionBuilt, CovariatesSelected, EmbeddingCompleted,
        EmbeddingStarted, EntitiesSelected, ExplainabilityEvent, ExplainabilityWarning,
        GraphExpansionStarted, LlmRequestCompleted, LlmRequestStarted, MappingQueryBuilt,
        QueryStarted, RelationshipsSelected, RunCompleted, RunFailed, RunStarted,
        TextUnitsSelected,
    };
    use crate::explainability::{
        ContextSectionKind, ExplainabilityContentMode, ExplainabilityContextSection,
        ExplainabilityQueryMethod, ExplainabilityRecordType, ExplainabilityRunKind,
    };

    #[test]
    fn test_should_use_internal_snake_case_event_discriminator() -> serde_json::Result<()> {
        let event =
            ExplainabilityEvent::QueryStarted(QueryStarted::new(ExplainabilityQueryMethod::Local));
        let value = serde_json::to_value(&event)?;
        assert_eq!(
            value,
            json!({
                "type": "query_started",
                "method": "local"
            })
        );
        assert_eq!(serde_json::from_value::<ExplainabilityEvent>(value)?, event);
        Ok(())
    }

    #[test]
    fn test_should_report_unknown_event_variant_to_strongly_typed_reader() {
        let unknown = json!({"type": "future_event", "optional_field": true});
        let error = serde_json::from_value::<ExplainabilityEvent>(unknown);
        assert!(error.is_err());
    }

    #[test]
    fn test_should_keep_every_event_discriminator_stable() -> serde_json::Result<()> {
        let events = [
            (
                ExplainabilityEvent::RunStarted(RunStarted::new(
                    ExplainabilityRunKind::Query,
                    ExplainabilityContentMode::Metadata,
                )),
                "run_started",
            ),
            (
                ExplainabilityEvent::RunCompleted(RunCompleted::new(12)),
                "run_completed",
            ),
            (
                ExplainabilityEvent::RunFailed(RunFailed::new(
                    "query_error".to_owned(),
                    "safe message".to_owned(),
                )),
                "run_failed",
            ),
            (
                ExplainabilityEvent::QueryStarted(QueryStarted::new(
                    ExplainabilityQueryMethod::Local,
                )),
                "query_started",
            ),
            (
                ExplainabilityEvent::MappingQueryBuilt(MappingQueryBuilt::new(2)),
                "mapping_query_built",
            ),
            (
                ExplainabilityEvent::EmbeddingStarted(EmbeddingStarted::new(
                    "embedding".to_owned(),
                )),
                "embedding_started",
            ),
            (
                ExplainabilityEvent::EmbeddingCompleted(EmbeddingCompleted::new(
                    "embedding".to_owned(),
                    3,
                    1_536,
                )),
                "embedding_completed",
            ),
            (
                ExplainabilityEvent::CandidatesRetrieved(CandidatesRetrieved::new(
                    ExplainabilityRecordType::Entity,
                    Vec::new(),
                )),
                "candidates_retrieved",
            ),
            (
                ExplainabilityEvent::CandidatesFiltered(CandidatesFiltered::new(
                    ExplainabilityRecordType::Entity,
                    Vec::new(),
                )),
                "candidates_filtered",
            ),
            (
                ExplainabilityEvent::EntitiesSelected(EntitiesSelected::new(Vec::new())),
                "entities_selected",
            ),
            (
                ExplainabilityEvent::GraphExpansionStarted(GraphExpansionStarted::new(Vec::new())),
                "graph_expansion_started",
            ),
            (
                ExplainabilityEvent::RelationshipsSelected(RelationshipsSelected::new(Vec::new())),
                "relationships_selected",
            ),
            (
                ExplainabilityEvent::CommunityReportsSelected(CommunityReportsSelected::new(
                    Vec::new(),
                )),
                "community_reports_selected",
            ),
            (
                ExplainabilityEvent::CovariatesSelected(CovariatesSelected::new(Vec::new())),
                "covariates_selected",
            ),
            (
                ExplainabilityEvent::TextUnitsSelected(TextUnitsSelected::new(Vec::new())),
                "text_units_selected",
            ),
            (
                ExplainabilityEvent::ContextBudgetAllocated(ContextBudgetAllocated::new(
                    1_024,
                    Vec::new(),
                )),
                "context_budget_allocated",
            ),
            (
                ExplainabilityEvent::ContextSectionBuilt(ContextSectionBuilt::new(
                    ExplainabilityContextSection::new(ContextSectionKind::Entities, 512),
                )),
                "context_section_built",
            ),
            (
                ExplainabilityEvent::ContextCompleted(ContextCompleted::new(900)),
                "context_completed",
            ),
            (
                ExplainabilityEvent::LlmRequestStarted(LlmRequestStarted::new(
                    "completion".to_owned(),
                    900,
                )),
                "llm_request_started",
            ),
            (
                ExplainabilityEvent::LlmRequestCompleted(LlmRequestCompleted::new(
                    "completion".to_owned(),
                    900,
                    120,
                    42,
                )),
                "llm_request_completed",
            ),
            (
                ExplainabilityEvent::Warning(ExplainabilityWarning::new(
                    "stale_reference".to_owned(),
                    "safe message".to_owned(),
                )),
                "warning",
            ),
        ];

        for (event, expected) in events {
            let value = serde_json::to_value(&event)?;
            assert_eq!(
                value.get("type").and_then(serde_json::Value::as_str),
                Some(expected)
            );
            assert_eq!(serde_json::from_value::<ExplainabilityEvent>(value)?, event);
        }
        Ok(())
    }
}
