//! Versioned explainability event vocabulary and named payloads.

use std::collections::HashSet;

use serde::{
    Deserialize, Serialize, Serializer,
    ser::{Error as _, SerializeStruct},
};

use super::{
    ContextSectionKind, DynamicCommunityRatingEvidence, ExplainabilityCandidate,
    ExplainabilityContentMode, ExplainabilityContextSection, ExplainabilityContractError,
    ExplainabilityQueryMethod, ExplainabilityRecordType, ExplainabilityRunKind,
    GlobalMapPointDecision, GlobalMapPointDecisionReason, GlobalMapPointEvidence,
};

struct ValidatedRecordIds<'a>(&'a [String]);

impl Serialize for ValidatedRecordIds<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        super::validation::record_ids::serialize(self.0, serializer)
    }
}

struct ValidatedGlobalMapPoints<'a>(&'a [GlobalMapPointEvidence]);

impl Serialize for ValidatedGlobalMapPoints<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        super::validation::global_map_points::serialize(self.0, serializer)
    }
}

struct ValidatedGlobalMapPointDecisions<'a>(&'a [GlobalMapPointDecision]);

impl Serialize for ValidatedGlobalMapPointDecisions<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        super::validation::global_map_point_decisions::serialize(self.0, serializer)
    }
}

struct ValidatedOptionalContent<'a>(&'a Option<String>);

impl Serialize for ValidatedOptionalContent<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        super::validation::optional_content_string::serialize(self.0, serializer)
    }
}

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
#[serde(try_from = "CandidateCollectionWire")]
#[non_exhaustive]
pub struct CandidatesRetrieved {
    /// Category shared by the returned candidates.
    record_type: ExplainabilityRecordType,
    /// Candidates in provider result order.
    #[serde(default, with = "super::validation::candidates")]
    candidates: Vec<ExplainabilityCandidate>,
}

impl CandidatesRetrieved {
    /// Create a retrieval payload after validating its homogeneous candidate category.
    ///
    /// Empty candidate collections are valid; `record_type` still identifies what was retrieved.
    ///
    /// # Errors
    ///
    /// Returns [`ExplainabilityContractError::CandidateTypeMismatch`] for the first candidate
    /// whose category differs from `record_type`.
    pub fn try_new(
        record_type: ExplainabilityRecordType,
        candidates: Vec<ExplainabilityCandidate>,
    ) -> Result<Self, ExplainabilityContractError> {
        validate_candidate_types(record_type, &candidates)?;
        Ok(Self {
            record_type,
            candidates,
        })
    }

    /// Return the homogeneous category represented by this collection.
    #[must_use]
    pub const fn record_type(&self) -> ExplainabilityRecordType {
        self.record_type
    }

    /// Borrow the candidates in provider result order.
    #[must_use]
    pub fn candidates(&self) -> &[ExplainabilityCandidate] {
        &self.candidates
    }
}

/// Candidate filters produced their accepted and rejected result metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "CandidateCollectionWire")]
#[non_exhaustive]
pub struct CandidatesFiltered {
    /// Category shared by the filtered candidates.
    record_type: ExplainabilityRecordType,
    /// Candidates annotated with actual selection state and reason where known.
    #[serde(default, with = "super::validation::candidates")]
    candidates: Vec<ExplainabilityCandidate>,
}

impl CandidatesFiltered {
    /// Create a candidate-filter payload after validating its homogeneous candidate category.
    ///
    /// Empty candidate collections are valid; `record_type` still identifies what was filtered.
    ///
    /// # Errors
    ///
    /// Returns [`ExplainabilityContractError::CandidateTypeMismatch`] for the first candidate
    /// whose category differs from `record_type`.
    pub fn try_new(
        record_type: ExplainabilityRecordType,
        candidates: Vec<ExplainabilityCandidate>,
    ) -> Result<Self, ExplainabilityContractError> {
        validate_candidate_types(record_type, &candidates)?;
        Ok(Self {
            record_type,
            candidates,
        })
    }

    /// Return the homogeneous category represented by this collection.
    #[must_use]
    pub const fn record_type(&self) -> ExplainabilityRecordType {
        self.record_type
    }

    /// Borrow the candidates in provider result order.
    #[must_use]
    pub fn candidates(&self) -> &[ExplainabilityCandidate] {
        &self.candidates
    }
}

#[derive(Deserialize)]
struct CandidateCollectionWire {
    record_type: ExplainabilityRecordType,
    #[serde(default, with = "super::validation::candidates")]
    candidates: Vec<ExplainabilityCandidate>,
}

impl TryFrom<CandidateCollectionWire> for CandidatesRetrieved {
    type Error = ExplainabilityContractError;

    fn try_from(wire: CandidateCollectionWire) -> Result<Self, Self::Error> {
        Self::try_new(wire.record_type, wire.candidates)
    }
}

impl TryFrom<CandidateCollectionWire> for CandidatesFiltered {
    type Error = ExplainabilityContractError;

    fn try_from(wire: CandidateCollectionWire) -> Result<Self, Self::Error> {
        Self::try_new(wire.record_type, wire.candidates)
    }
}

fn validate_candidate_types(
    expected: ExplainabilityRecordType,
    candidates: &[ExplainabilityCandidate],
) -> Result<(), ExplainabilityContractError> {
    if let Some((candidate_index, candidate)) = candidates
        .iter()
        .enumerate()
        .find(|(_, candidate)| candidate.record_type != expected)
    {
        return Err(ExplainabilityContractError::CandidateTypeMismatch {
            expected,
            actual: candidate.record_type,
            candidate_index,
        });
    }
    Ok(())
}

macro_rules! candidate_selection_payload {
    ($name:ident, $field:ident, $record_type:expr, $docs:literal, $field_docs:literal) => {
        #[doc = $docs]
        #[derive(Debug, Clone, PartialEq, Serialize)]
        #[non_exhaustive]
        pub struct $name {
            #[doc = $field_docs]
            #[serde(default, with = "super::validation::candidates")]
            $field: Vec<ExplainabilityCandidate>,
        }

        impl $name {
            #[doc = concat!(
                "Create a validated `",
                stringify!($name),
                "` payload.\n\n# Errors\n\nReturns ",
                "[`ExplainabilityContractError::CandidateTypeMismatch`] when a candidate has ",
                "the wrong record category."
            )]
            pub fn try_new(
                $field: Vec<ExplainabilityCandidate>,
            ) -> Result<Self, ExplainabilityContractError> {
                validate_candidate_types($record_type, &$field)?;
                Ok(Self { $field })
            }

            #[doc = concat!("Borrow the `", stringify!($field), "` in effective order.")]
            #[must_use]
            pub fn $field(&self) -> &[ExplainabilityCandidate] {
                &self.$field
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                #[derive(Deserialize)]
                struct Wire {
                    #[serde(default, with = "super::validation::candidates")]
                    $field: Vec<ExplainabilityCandidate>,
                }

                let wire = Wire::deserialize(deserializer)?;
                Self::try_new(wire.$field).map_err(serde::de::Error::custom)
            }
        }
    };
}

candidate_selection_payload!(
    EntitiesSelected,
    entities,
    ExplainabilityRecordType::Entity,
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
    ExplainabilityRecordType::Relationship,
    "Relationships selected during graph expansion.",
    "Selected relationship metadata in context order."
);
candidate_selection_payload!(
    CommunityReportsSelected,
    community_reports,
    ExplainabilityRecordType::CommunityReport,
    "Community reports selected through entity membership.",
    "Selected community-report metadata in context order."
);
candidate_selection_payload!(
    CovariatesSelected,
    covariates,
    ExplainabilityRecordType::Covariate,
    "Covariates selected through entity references.",
    "Selected covariate metadata in context order."
);
candidate_selection_payload!(
    TextUnitsSelected,
    text_units,
    ExplainabilityRecordType::TextUnit,
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

/// Global community context completed construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct GlobalContextBuilt {
    /// Number of real context batches produced.
    pub batch_count: u32,
    /// Number of stable CommunityReport IDs across the batches.
    pub report_count: u32,
}

/// Source that populated one real Dynamic Global traversal queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DynamicTraversalWaveSource {
    /// Report-backed level-zero communities.
    Initial,
    /// Report-backed children of communities that passed the threshold.
    ChildExpansion,
    /// The existing max-level fallback branch.
    Fallback,
}

/// Dynamic Global community selection began with a valid initial queue.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "DynamicCommunitySelectionStartedWire")]
#[non_exhaustive]
pub struct DynamicCommunitySelectionStarted {
    /// Number of report-backed communities in the initial queue.
    pub initial_community_count: u32,
    /// Configured relevance threshold.
    pub threshold: i64,
    /// Configured deepest fallback hierarchy level.
    pub max_level: i64,
    /// Whether relevant parents remain selected after child traversal.
    pub keep_parent: bool,
    /// Whether rating prompts use report summaries instead of full content.
    pub use_summary: bool,
    /// Number of sequential rating requests per community.
    pub num_repeats: u32,
}

#[derive(Serialize, Deserialize)]
struct DynamicCommunitySelectionStartedWire {
    initial_community_count: u32,
    threshold: i64,
    max_level: i64,
    keep_parent: bool,
    use_summary: bool,
    num_repeats: u32,
}

impl TryFrom<DynamicCommunitySelectionStartedWire> for DynamicCommunitySelectionStarted {
    type Error = ExplainabilityContractError;

    fn try_from(wire: DynamicCommunitySelectionStartedWire) -> Result<Self, Self::Error> {
        let event = Self {
            initial_community_count: wire.initial_community_count,
            threshold: wire.threshold,
            max_level: wire.max_level,
            keep_parent: wire.keep_parent,
            use_summary: wire.use_summary,
            num_repeats: wire.num_repeats,
        };
        event.validate()?;
        Ok(event)
    }
}

impl Serialize for DynamicCommunitySelectionStarted {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(S::Error::custom)?;
        DynamicCommunitySelectionStartedWire {
            initial_community_count: self.initial_community_count,
            threshold: self.threshold,
            max_level: self.max_level,
            keep_parent: self.keep_parent,
            use_summary: self.use_summary,
            num_repeats: self.num_repeats,
        }
        .serialize(serializer)
    }
}

impl DynamicCommunitySelectionStarted {
    /// Create validated Dynamic Global selection-start metadata.
    pub fn try_new(
        initial_community_count: u32,
        threshold: i64,
        max_level: i64,
        keep_parent: bool,
        use_summary: bool,
        num_repeats: u32,
    ) -> Result<Self, ExplainabilityContractError> {
        let event = Self {
            initial_community_count,
            threshold,
            max_level,
            keep_parent,
            use_summary,
            num_repeats,
        };
        event.validate()?;
        Ok(event)
    }

    fn validate(&self) -> Result<(), ExplainabilityContractError> {
        if self.initial_community_count == 0 {
            return Err(ExplainabilityContractError::InvalidDynamicSelection {
                reason: "initial community count must be greater than zero",
            });
        }
        if self.max_level < 0 {
            return Err(ExplainabilityContractError::InvalidDynamicSelection {
                reason: "max level must be non-negative",
            });
        }
        if self.num_repeats == 0 {
            return Err(ExplainabilityContractError::InvalidDynamicSelection {
                reason: "repeat count must be greater than zero",
            });
        }
        if usize::try_from(self.initial_community_count).unwrap_or(usize::MAX)
            > super::validation::MAX_DYNAMIC_COMMUNITIES
        {
            return Err(ExplainabilityContractError::InvalidDynamicSelection {
                reason: "initial community count exceeds the contract limit",
            });
        }
        Ok(())
    }
}

/// One real Dynamic Global traversal queue began rating.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "DynamicCommunityTraversalWaveStartedWire")]
#[non_exhaustive]
pub struct DynamicCommunityTraversalWaveStarted {
    /// Zero-based traversal-wave identity.
    pub wave_index: u32,
    /// Proven source of this queue.
    pub source: DynamicTraversalWaveSource,
    /// Hierarchy community identities in the real queue order.
    #[serde(default, with = "super::validation::dynamic_community_ids")]
    pub community_ids: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct DynamicCommunityTraversalWaveStartedWire {
    wave_index: u32,
    source: DynamicTraversalWaveSource,
    #[serde(default, with = "super::validation::dynamic_community_ids")]
    community_ids: Vec<String>,
}

impl TryFrom<DynamicCommunityTraversalWaveStartedWire> for DynamicCommunityTraversalWaveStarted {
    type Error = ExplainabilityContractError;

    fn try_from(wire: DynamicCommunityTraversalWaveStartedWire) -> Result<Self, Self::Error> {
        let event = Self {
            wave_index: wire.wave_index,
            source: wire.source,
            community_ids: wire.community_ids,
        };
        event.validate()?;
        Ok(event)
    }
}

impl Serialize for DynamicCommunityTraversalWaveStarted {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(S::Error::custom)?;
        DynamicCommunityTraversalWaveStartedWire {
            wave_index: self.wave_index,
            source: self.source,
            community_ids: self.community_ids.clone(),
        }
        .serialize(serializer)
    }
}

impl DynamicCommunityTraversalWaveStarted {
    /// Create validated evidence for one real traversal queue.
    pub fn try_new(
        wave_index: u32,
        source: DynamicTraversalWaveSource,
        community_ids: Vec<String>,
    ) -> Result<Self, ExplainabilityContractError> {
        let event = Self {
            wave_index,
            source,
            community_ids,
        };
        event.validate()?;
        Ok(event)
    }

    fn validate(&self) -> Result<(), ExplainabilityContractError> {
        if self.community_ids.is_empty() {
            return Err(ExplainabilityContractError::InvalidDynamicSelection {
                reason: "traversal wave must contain at least one community",
            });
        }
        if self.community_ids.len() > super::validation::MAX_DYNAMIC_COMMUNITIES {
            return Err(ExplainabilityContractError::InvalidDynamicSelection {
                reason: "traversal wave exceeds the community contract limit",
            });
        }
        let mut unique_ids = HashSet::with_capacity(self.community_ids.len());
        for community_id in &self.community_ids {
            super::validation::validate_dynamic_id(community_id, "Dynamic community ID").map_err(
                |_| ExplainabilityContractError::InvalidIdentifier {
                    kind: "Dynamic community ID",
                    reason: "value is empty, too long, or contains disallowed bytes",
                },
            )?;
            if !unique_ids.insert(community_id) {
                return Err(ExplainabilityContractError::InvalidDynamicSelection {
                    reason: "traversal wave contains duplicate community identities",
                });
            }
        }
        Ok(())
    }
}

/// One real Dynamic Global rating repeat is about to build and issue its LLM request.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "DynamicCommunityRatingAttemptStartedWire")]
#[non_exhaustive]
pub struct DynamicCommunityRatingAttemptStarted {
    /// Hierarchy/traversal community identity.
    pub community_id: String,
    /// Stable persisted CommunityReport identity rated by this request.
    pub report_id: String,
    /// Zero-based repeat identity within this community.
    pub repeat_index: u32,
    /// Configured number of sequential repeats for this community.
    pub repeat_count: u32,
}

#[derive(Serialize, Deserialize)]
struct DynamicCommunityRatingAttemptStartedWire {
    community_id: String,
    report_id: String,
    repeat_index: u32,
    repeat_count: u32,
}

impl TryFrom<DynamicCommunityRatingAttemptStartedWire> for DynamicCommunityRatingAttemptStarted {
    type Error = ExplainabilityContractError;

    fn try_from(wire: DynamicCommunityRatingAttemptStartedWire) -> Result<Self, Self::Error> {
        let event = Self {
            community_id: wire.community_id,
            report_id: wire.report_id,
            repeat_index: wire.repeat_index,
            repeat_count: wire.repeat_count,
        };
        event.validate()?;
        Ok(event)
    }
}

impl Serialize for DynamicCommunityRatingAttemptStarted {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(S::Error::custom)?;
        DynamicCommunityRatingAttemptStartedWire {
            community_id: self.community_id.clone(),
            report_id: self.report_id.clone(),
            repeat_index: self.repeat_index,
            repeat_count: self.repeat_count,
        }
        .serialize(serializer)
    }
}

impl DynamicCommunityRatingAttemptStarted {
    /// Create validated evidence for one real rating repeat.
    pub fn try_new(
        community_id: String,
        report_id: String,
        repeat_index: u32,
        repeat_count: u32,
    ) -> Result<Self, ExplainabilityContractError> {
        let event = Self {
            community_id,
            report_id,
            repeat_index,
            repeat_count,
        };
        event.validate()?;
        Ok(event)
    }

    fn validate(&self) -> Result<(), ExplainabilityContractError> {
        super::validation::validate_dynamic_id(&self.community_id, "Dynamic community ID")
            .map_err(|_| ExplainabilityContractError::InvalidIdentifier {
                kind: "Dynamic community ID",
                reason: "value is empty, too long, or contains disallowed bytes",
            })?;
        super::validation::validate_dynamic_id(&self.report_id, "CommunityReport ID").map_err(
            |_| ExplainabilityContractError::InvalidIdentifier {
                kind: "CommunityReport ID",
                reason: "value is empty, too long, or contains disallowed bytes",
            },
        )?;
        if self.repeat_count == 0 || self.repeat_index >= self.repeat_count {
            return Err(ExplainabilityContractError::InvalidDynamicRatingRepeat);
        }
        Ok(())
    }
}

/// Dynamic Global community selection completed with final retained decisions.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "DynamicCommunitySelectionCompletedWire")]
#[non_exhaustive]
pub struct DynamicCommunitySelectionCompleted {
    /// Number of communities rated by the traversal.
    pub visited_count: u32,
    /// Number of visited communities whose majority rating passed the threshold.
    pub threshold_passed_count: u32,
    /// Number of communities retained in the final selection output.
    pub selected_count: u32,
    /// Final hierarchy community identities in Dynamic selection output order.
    #[serde(default, with = "super::validation::dynamic_community_ids")]
    pub selected_community_ids: Vec<String>,
    /// Final stable CommunityReport identities in Dynamic selection output order.
    #[serde(default, with = "super::validation::dynamic_community_ids")]
    pub selected_report_ids: Vec<String>,
    /// Final rating decisions in traversal visit order.
    #[serde(default, with = "super::validation::dynamic_rating_evidence")]
    pub ratings: Vec<DynamicCommunityRatingEvidence>,
}

#[derive(Serialize, Deserialize)]
struct DynamicCommunitySelectionCompletedWire {
    visited_count: u32,
    threshold_passed_count: u32,
    selected_count: u32,
    #[serde(default, with = "super::validation::dynamic_community_ids")]
    selected_community_ids: Vec<String>,
    #[serde(default, with = "super::validation::dynamic_community_ids")]
    selected_report_ids: Vec<String>,
    #[serde(default, with = "super::validation::dynamic_rating_evidence")]
    ratings: Vec<DynamicCommunityRatingEvidence>,
}

impl TryFrom<DynamicCommunitySelectionCompletedWire> for DynamicCommunitySelectionCompleted {
    type Error = ExplainabilityContractError;

    fn try_from(wire: DynamicCommunitySelectionCompletedWire) -> Result<Self, Self::Error> {
        let event = Self {
            visited_count: wire.visited_count,
            threshold_passed_count: wire.threshold_passed_count,
            selected_count: wire.selected_count,
            selected_community_ids: wire.selected_community_ids,
            selected_report_ids: wire.selected_report_ids,
            ratings: wire.ratings,
        };
        event.validate()?;
        Ok(event)
    }
}

impl Serialize for DynamicCommunitySelectionCompleted {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(S::Error::custom)?;
        DynamicCommunitySelectionCompletedWire {
            visited_count: self.visited_count,
            threshold_passed_count: self.threshold_passed_count,
            selected_count: self.selected_count,
            selected_community_ids: self.selected_community_ids.clone(),
            selected_report_ids: self.selected_report_ids.clone(),
            ratings: self.ratings.clone(),
        }
        .serialize(serializer)
    }
}

impl DynamicCommunitySelectionCompleted {
    /// Create validated final Dynamic Global selection evidence.
    pub fn try_new(
        selected_community_ids: Vec<String>,
        selected_report_ids: Vec<String>,
        ratings: Vec<DynamicCommunityRatingEvidence>,
    ) -> Result<Self, ExplainabilityContractError> {
        let visited_count = u32::try_from(ratings.len()).unwrap_or(u32::MAX);
        let threshold_passed_count = u32::try_from(
            ratings
                .iter()
                .filter(|rating| rating.threshold_passed)
                .count(),
        )
        .unwrap_or(u32::MAX);
        let selected_count = u32::try_from(ratings.iter().filter(|rating| rating.selected).count())
            .unwrap_or(u32::MAX);
        let event = Self {
            visited_count,
            threshold_passed_count,
            selected_count,
            selected_community_ids,
            selected_report_ids,
            ratings,
        };
        event.validate()?;
        Ok(event)
    }

    fn validate(&self) -> Result<(), ExplainabilityContractError> {
        if self.ratings.len() > super::validation::MAX_DYNAMIC_COMMUNITIES
            || self.selected_community_ids.len() > super::validation::MAX_DYNAMIC_COMMUNITIES
            || self.selected_report_ids.len() > super::validation::MAX_DYNAMIC_COMMUNITIES
        {
            return Err(ExplainabilityContractError::InvalidDynamicSelection {
                reason: "selection evidence exceeds the community contract limit",
            });
        }
        let visited = u32::try_from(self.ratings.len()).unwrap_or(u32::MAX);
        let passed = u32::try_from(
            self.ratings
                .iter()
                .filter(|rating| rating.threshold_passed)
                .count(),
        )
        .unwrap_or(u32::MAX);
        let selected = self
            .ratings
            .iter()
            .filter(|rating| rating.selected)
            .collect::<Vec<_>>();
        let selected_count = u32::try_from(selected.len()).unwrap_or(u32::MAX);
        let mut community_ids = HashSet::with_capacity(self.ratings.len());
        let mut report_ids = HashSet::with_capacity(self.ratings.len());
        for rating in &self.ratings {
            if !community_ids.insert(&rating.community_id) || !report_ids.insert(&rating.report_id)
            {
                return Err(ExplainabilityContractError::InvalidDynamicSelection {
                    reason: "selection evidence contains duplicate community or report identities",
                });
            }
        }
        if self.visited_count != visited {
            return Err(ExplainabilityContractError::CollectionCountMismatch {
                collection: "Dynamic visited community",
            });
        }
        if self.threshold_passed_count != passed {
            return Err(ExplainabilityContractError::CollectionCountMismatch {
                collection: "Dynamic threshold-passed community",
            });
        }
        if self.selected_count != selected_count
            || self.selected_community_ids.len() != selected.len()
            || self.selected_report_ids.len() != selected.len()
        {
            return Err(ExplainabilityContractError::CollectionCountMismatch {
                collection: "Dynamic selected community",
            });
        }
        for ((rating, community_id), report_id) in selected
            .into_iter()
            .zip(&self.selected_community_ids)
            .zip(&self.selected_report_ids)
        {
            if !rating.threshold_passed {
                return Err(ExplainabilityContractError::InvalidDynamicRatingDecision);
            }
            if rating.community_id != *community_id || rating.report_id != *report_id {
                return Err(ExplainabilityContractError::InvalidDynamicSelection {
                    reason: "selected identities disagree with final rating evidence",
                });
            }
        }
        Ok(())
    }
}

impl GlobalContextBuilt {
    /// Create a Global context summary.
    #[must_use]
    pub const fn new(batch_count: u32, report_count: u32) -> Self {
        Self {
            batch_count,
            report_count,
        }
    }
}

/// Global map fan-out began for the constructed batches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct GlobalMapStarted {
    /// Number of map analyst calls scheduled by the real orchestration.
    pub batch_count: u32,
}

impl GlobalMapStarted {
    /// Create a map-stage start summary.
    #[must_use]
    pub const fn new(batch_count: u32) -> Self {
        Self { batch_count }
    }
}

/// One actual Global context batch completed construction for a map analyst.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "GlobalMapBatchBuiltWire")]
#[non_exhaustive]
pub struct GlobalMapBatchBuilt {
    /// Stable zero-based semantic batch identity.
    pub batch_index: u32,
    /// Number of CommunityReports in this batch.
    pub report_count: u32,
    /// Stable CommunityReport IDs in the exact batch-local order.
    #[serde(default, with = "super::validation::record_ids")]
    pub report_ids: Vec<String>,
    /// Tokens in the exact map `context_data` string.
    pub tokens_used: u64,
    /// Configured per-batch token budget.
    pub token_budget: u64,
    /// Exact map `context_data` when content disclosure is enabled.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub context: Option<String>,
}

#[derive(Deserialize)]
struct GlobalMapBatchBuiltWire {
    batch_index: u32,
    report_count: u32,
    #[serde(default, with = "super::validation::record_ids")]
    report_ids: Vec<String>,
    tokens_used: u64,
    token_budget: u64,
    #[serde(default, with = "super::validation::optional_content_string")]
    context: Option<String>,
}

impl TryFrom<GlobalMapBatchBuiltWire> for GlobalMapBatchBuilt {
    type Error = ExplainabilityContractError;

    fn try_from(wire: GlobalMapBatchBuiltWire) -> Result<Self, Self::Error> {
        if usize::try_from(wire.report_count).ok() != Some(wire.report_ids.len()) {
            return Err(ExplainabilityContractError::CollectionCountMismatch {
                collection: "Global map batch report",
            });
        }
        Ok(Self {
            batch_index: wire.batch_index,
            report_count: wire.report_count,
            report_ids: wire.report_ids,
            tokens_used: wire.tokens_used,
            token_budget: wire.token_budget,
            context: wire.context,
        })
    }
}

impl Serialize for GlobalMapBatchBuilt {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(S::Error::custom)?;
        let field_count = 5 + usize::from(self.context.is_some());
        let mut state = serializer.serialize_struct("GlobalMapBatchBuilt", field_count)?;
        state.serialize_field("batch_index", &self.batch_index)?;
        state.serialize_field("report_count", &self.report_count)?;
        state.serialize_field("report_ids", &ValidatedRecordIds(&self.report_ids))?;
        state.serialize_field("tokens_used", &self.tokens_used)?;
        state.serialize_field("token_budget", &self.token_budget)?;
        if self.context.is_some() {
            state.serialize_field("context", &ValidatedOptionalContent(&self.context))?;
        }
        state.end()
    }
}

impl GlobalMapBatchBuilt {
    /// Create metadata-only Global map batch evidence.
    #[must_use]
    pub fn new(
        batch_index: u32,
        report_ids: Vec<String>,
        tokens_used: u64,
        token_budget: u64,
    ) -> Self {
        let report_count = u32::try_from(report_ids.len()).unwrap_or(u32::MAX);
        Self {
            batch_index,
            report_count,
            report_ids,
            tokens_used,
            token_budget,
            context: None,
        }
    }

    fn validate(&self) -> Result<(), ExplainabilityContractError> {
        if usize::try_from(self.report_count).ok() != Some(self.report_ids.len()) {
            return Err(ExplainabilityContractError::CollectionCountMismatch {
                collection: "Global map batch report",
            });
        }
        Ok(())
    }
}

/// Parsed points produced by one Global map analyst.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "GlobalMapPointsProducedWire")]
#[non_exhaustive]
pub struct GlobalMapPointsProduced {
    /// Stable zero-based semantic batch identity.
    pub batch_index: u32,
    /// Parsed points in their actual post-parse order.
    #[serde(default, with = "super::validation::global_map_points")]
    pub points: Vec<GlobalMapPointEvidence>,
}

#[derive(Deserialize)]
struct GlobalMapPointsProducedWire {
    batch_index: u32,
    #[serde(default, with = "super::validation::global_map_points")]
    points: Vec<GlobalMapPointEvidence>,
}

impl TryFrom<GlobalMapPointsProducedWire> for GlobalMapPointsProduced {
    type Error = ExplainabilityContractError;

    fn try_from(wire: GlobalMapPointsProducedWire) -> Result<Self, Self::Error> {
        let event = Self {
            batch_index: wire.batch_index,
            points: wire.points,
        };
        event.validate()?;
        Ok(event)
    }
}

impl Serialize for GlobalMapPointsProduced {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(S::Error::custom)?;
        let mut state = serializer.serialize_struct("GlobalMapPointsProduced", 2)?;
        state.serialize_field("batch_index", &self.batch_index)?;
        state.serialize_field("points", &ValidatedGlobalMapPoints(&self.points))?;
        state.end()
    }
}

impl GlobalMapPointsProduced {
    /// Create parsed map-point evidence.
    pub fn try_new(
        batch_index: u32,
        points: Vec<GlobalMapPointEvidence>,
    ) -> Result<Self, ExplainabilityContractError> {
        let event = Self {
            batch_index,
            points,
        };
        event.validate()?;
        Ok(event)
    }

    fn validate(&self) -> Result<(), ExplainabilityContractError> {
        if let Some((point_index, _)) = self
            .points
            .iter()
            .enumerate()
            .find(|(_, point)| point.batch_index != self.batch_index)
        {
            return Err(ExplainabilityContractError::GlobalMapPointBatchMismatch { point_index });
        }
        if let Some((point_index, _)) =
            self.points.iter().enumerate().find(|(point_index, point)| {
                u32::try_from(*point_index).ok() != Some(point.point_index)
            })
        {
            return Err(ExplainabilityContractError::GlobalMapPointOrderMismatch { point_index });
        }
        Ok(())
    }
}

/// Reduce fitting completed against the real parsed map points.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "GlobalReduceContextBuiltWire")]
#[non_exhaustive]
pub struct GlobalReduceContextBuilt {
    /// Total parsed point count, including non-positive points.
    pub candidate_point_count: u64,
    /// Number of points whose score is greater than zero.
    pub positive_point_count: u64,
    /// Number of positive points included in the Reduce context.
    pub selected_point_count: u64,
    /// Configured Reduce data token budget.
    pub token_budget: u64,
    /// Tokens consumed by selected point blocks.
    pub tokens_used: u64,
    /// Whether the first over-budget point stopped selection.
    pub truncated: bool,
    /// Decisions for every parsed point.
    #[serde(default, with = "super::validation::global_map_point_decisions")]
    pub points: Vec<GlobalMapPointDecision>,
    /// Exact `report_data` supplied to the Reduce prompt when content disclosure is enabled.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub context: Option<String>,
}

#[derive(Deserialize)]
struct GlobalReduceContextBuiltWire {
    candidate_point_count: u64,
    positive_point_count: u64,
    selected_point_count: u64,
    token_budget: u64,
    tokens_used: u64,
    truncated: bool,
    #[serde(default, with = "super::validation::global_map_point_decisions")]
    points: Vec<GlobalMapPointDecision>,
    #[serde(default, with = "super::validation::optional_content_string")]
    context: Option<String>,
}

impl TryFrom<GlobalReduceContextBuiltWire> for GlobalReduceContextBuilt {
    type Error = ExplainabilityContractError;

    fn try_from(wire: GlobalReduceContextBuiltWire) -> Result<Self, Self::Error> {
        let event = Self {
            candidate_point_count: wire.candidate_point_count,
            positive_point_count: wire.positive_point_count,
            selected_point_count: wire.selected_point_count,
            token_budget: wire.token_budget,
            tokens_used: wire.tokens_used,
            truncated: wire.truncated,
            points: wire.points,
            context: wire.context,
        };
        event.validate()?;
        Ok(event)
    }
}

impl Serialize for GlobalReduceContextBuilt {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(S::Error::custom)?;
        let field_count = 7 + usize::from(self.context.is_some());
        let mut state = serializer.serialize_struct("GlobalReduceContextBuilt", field_count)?;
        state.serialize_field("candidate_point_count", &self.candidate_point_count)?;
        state.serialize_field("positive_point_count", &self.positive_point_count)?;
        state.serialize_field("selected_point_count", &self.selected_point_count)?;
        state.serialize_field("token_budget", &self.token_budget)?;
        state.serialize_field("tokens_used", &self.tokens_used)?;
        state.serialize_field("truncated", &self.truncated)?;
        state.serialize_field("points", &ValidatedGlobalMapPointDecisions(&self.points))?;
        if self.context.is_some() {
            state.serialize_field("context", &ValidatedOptionalContent(&self.context))?;
        }
        state.end()
    }
}

impl GlobalReduceContextBuilt {
    /// Create validated metadata-only Reduce fitting evidence.
    pub fn try_new(
        token_budget: u64,
        tokens_used: u64,
        truncated: bool,
        points: Vec<GlobalMapPointDecision>,
    ) -> Result<Self, ExplainabilityContractError> {
        let candidate_point_count = u64::try_from(points.len()).unwrap_or(u64::MAX);
        let positive_point_count =
            u64::try_from(points.iter().filter(|point| point.score > 0).count())
                .unwrap_or(u64::MAX);
        let selected_point_count =
            u64::try_from(points.iter().filter(|point| point.selected).count()).unwrap_or(u64::MAX);
        let event = Self {
            candidate_point_count,
            positive_point_count,
            selected_point_count,
            token_budget,
            tokens_used,
            truncated,
            points,
            context: None,
        };
        event.validate()?;
        Ok(event)
    }

    fn validate(&self) -> Result<(), ExplainabilityContractError> {
        let candidate_count = u64::try_from(self.points.len()).unwrap_or(u64::MAX);
        let positive_count =
            u64::try_from(self.points.iter().filter(|point| point.score > 0).count())
                .unwrap_or(u64::MAX);
        let selected_count =
            u64::try_from(self.points.iter().filter(|point| point.selected).count())
                .unwrap_or(u64::MAX);
        if self.candidate_point_count != candidate_count {
            return Err(ExplainabilityContractError::CollectionCountMismatch {
                collection: "Global Reduce candidate point",
            });
        }
        if self.positive_point_count != positive_count {
            return Err(ExplainabilityContractError::CollectionCountMismatch {
                collection: "Global Reduce positive point",
            });
        }
        if self.selected_point_count != selected_count {
            return Err(ExplainabilityContractError::CollectionCountMismatch {
                collection: "Global Reduce selected point",
            });
        }
        if self.tokens_used > self.token_budget {
            return Err(ExplainabilityContractError::GlobalReduceTokensExceedBudget);
        }
        for (point_index, point) in self.points.iter().enumerate() {
            let valid = match point.reason {
                GlobalMapPointDecisionReason::Selected => point.selected && point.score > 0,
                GlobalMapPointDecisionReason::NonPositiveScore => {
                    !point.selected && point.score <= 0
                }
                GlobalMapPointDecisionReason::TokenBudget => !point.selected && point.score > 0,
            };
            if !valid {
                return Err(ExplainabilityContractError::InvalidGlobalReduceDecision {
                    point_index,
                    reason: "score, selected flag, and decision reason disagree",
                });
            }
        }
        let has_budget_exclusion = self
            .points
            .iter()
            .any(|point| point.reason == GlobalMapPointDecisionReason::TokenBudget);
        if self.truncated != has_budget_exclusion {
            return Err(ExplainabilityContractError::InvalidGlobalReduceDecision {
                point_index: 0,
                reason: "truncated flag disagrees with token-budget decisions",
            });
        }
        Ok(())
    }
}

/// Reason the Global Reduce LLM was not called.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GlobalReduceSkipReason {
    /// No parsed map point had a score greater than zero.
    NoPositivePoints,
}

/// Global Reduce was explicitly skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct GlobalReduceSkipped {
    /// Proven reason no Reduce LLM request was made.
    pub reason: GlobalReduceSkipReason,
}

impl GlobalReduceSkipped {
    /// Create a Reduce-skip event.
    #[must_use]
    pub const fn new(reason: GlobalReduceSkipReason) -> Self {
        Self { reason }
    }
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
    /// Rendered system prompt when the content mode permits it.
    ///
    /// This is not the provider's complete request object and never includes headers or secrets.
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
    /// Global community context completed.
    GlobalContextBuilt(GlobalContextBuilt),
    /// Dynamic Global community selection began.
    DynamicCommunitySelectionStarted(DynamicCommunitySelectionStarted),
    /// One real Dynamic Global traversal queue began rating.
    DynamicCommunityTraversalWaveStarted(DynamicCommunityTraversalWaveStarted),
    /// One real Dynamic Global rating repeat began.
    DynamicCommunityRatingAttemptStarted(DynamicCommunityRatingAttemptStarted),
    /// Dynamic Global community selection completed.
    DynamicCommunitySelectionCompleted(DynamicCommunitySelectionCompleted),
    /// Global map fan-out began.
    GlobalMapStarted(GlobalMapStarted),
    /// One actual Global map batch was built.
    GlobalMapBatchBuilt(GlobalMapBatchBuilt),
    /// One map analyst's response was parsed into points.
    GlobalMapPointsProduced(GlobalMapPointsProduced),
    /// Global Reduce context fitting completed.
    GlobalReduceContextBuilt(GlobalReduceContextBuilt),
    /// Global Reduce was skipped.
    GlobalReduceSkipped(GlobalReduceSkipped),
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
        GlobalContextBuilt, GlobalMapBatchBuilt, GlobalMapPointsProduced, GlobalMapStarted,
        GlobalReduceContextBuilt, GlobalReduceSkipReason, GlobalReduceSkipped,
        GraphExpansionStarted, LlmRequestCompleted, LlmRequestStarted, MappingQueryBuilt,
        QueryStarted, RelationshipsSelected, RunCompleted, RunFailed, RunStarted,
        TextUnitsSelected,
    };
    use crate::explainability::{
        ContextSectionKind, ExplainabilityContentMode, ExplainabilityContextSection,
        ExplainabilityQueryMethod, ExplainabilityRecordType, ExplainabilityRunKind,
        GlobalMapPointDecision, GlobalMapPointDecisionReason, GlobalMapPointEvidence,
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
    fn test_should_validate_global_batch_ids_and_allow_empty_point_lists()
    -> Result<(), Box<dyn std::error::Error>> {
        let empty = ExplainabilityEvent::GlobalMapPointsProduced(GlobalMapPointsProduced::try_new(
            7,
            Vec::new(),
        )?);
        assert_eq!(
            serde_json::from_value::<ExplainabilityEvent>(serde_json::to_value(&empty)?)?,
            empty
        );

        let invalid = json!({
            "type": "global_map_batch_built",
            "batch_index": 0,
            "report_count": 1,
            "report_ids": ["x".repeat(257)],
            "tokens_used": 1,
            "token_budget": 2,
        });
        assert!(serde_json::from_value::<ExplainabilityEvent>(invalid).is_err());
        let mismatched_count = json!({
            "type": "global_map_batch_built",
            "batch_index": 0,
            "report_count": 2,
            "report_ids": ["report-1"],
            "tokens_used": 1,
            "token_budget": 2,
        });
        assert!(serde_json::from_value::<ExplainabilityEvent>(mismatched_count).is_err());
        let mismatched_batch = json!({
            "type": "global_map_points_produced",
            "batch_index": 0,
            "points": [{
                "batch_index": 1,
                "point_index": 0,
                "score": 1,
            }],
        });
        assert!(serde_json::from_value::<ExplainabilityEvent>(mismatched_batch).is_err());
        let mismatched_point_order = json!({
            "type": "global_map_points_produced",
            "batch_index": 0,
            "points": [{
                "batch_index": 0,
                "point_index": 1,
                "score": 1,
            }],
        });
        assert!(serde_json::from_value::<ExplainabilityEvent>(mismatched_point_order).is_err());

        let invalid_write = ExplainabilityEvent::GlobalMapBatchBuilt(GlobalMapBatchBuilt {
            batch_index: 0,
            report_count: 2,
            report_ids: vec!["report-1".to_owned()],
            tokens_used: 1,
            token_budget: 2,
            context: None,
        });
        assert!(serde_json::to_value(invalid_write).is_err());
        assert!(
            GlobalMapPointsProduced::try_new(0, vec![GlobalMapPointEvidence::new(1, 0, 1)],)
                .is_err()
        );
        assert!(
            GlobalMapPointsProduced::try_new(0, vec![GlobalMapPointEvidence::new(0, 1, 1)],)
                .is_err()
        );
        let invalid_reduce =
            ExplainabilityEvent::GlobalReduceContextBuilt(GlobalReduceContextBuilt {
                candidate_point_count: 1,
                positive_point_count: 1,
                selected_point_count: 1,
                token_budget: 2,
                tokens_used: 1,
                truncated: false,
                points: vec![GlobalMapPointDecision::new(
                    0,
                    0,
                    1,
                    false,
                    GlobalMapPointDecisionReason::TokenBudget,
                )],
                context: None,
            });
        assert!(serde_json::to_value(invalid_reduce).is_err());
        Ok(())
    }

    #[test]
    fn test_should_keep_every_event_discriminator_stable() -> Result<(), Box<dyn std::error::Error>>
    {
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
                ExplainabilityEvent::CandidatesRetrieved(CandidatesRetrieved::try_new(
                    ExplainabilityRecordType::Entity,
                    Vec::new(),
                )?),
                "candidates_retrieved",
            ),
            (
                ExplainabilityEvent::CandidatesFiltered(CandidatesFiltered::try_new(
                    ExplainabilityRecordType::Entity,
                    Vec::new(),
                )?),
                "candidates_filtered",
            ),
            (
                ExplainabilityEvent::EntitiesSelected(EntitiesSelected::try_new(Vec::new())?),
                "entities_selected",
            ),
            (
                ExplainabilityEvent::GraphExpansionStarted(GraphExpansionStarted::new(Vec::new())),
                "graph_expansion_started",
            ),
            (
                ExplainabilityEvent::RelationshipsSelected(RelationshipsSelected::try_new(
                    Vec::new(),
                )?),
                "relationships_selected",
            ),
            (
                ExplainabilityEvent::CommunityReportsSelected(CommunityReportsSelected::try_new(
                    Vec::new(),
                )?),
                "community_reports_selected",
            ),
            (
                ExplainabilityEvent::CovariatesSelected(CovariatesSelected::try_new(Vec::new())?),
                "covariates_selected",
            ),
            (
                ExplainabilityEvent::TextUnitsSelected(TextUnitsSelected::try_new(Vec::new())?),
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
                ExplainabilityEvent::GlobalContextBuilt(GlobalContextBuilt::new(2, 3)),
                "global_context_built",
            ),
            (
                ExplainabilityEvent::GlobalMapStarted(GlobalMapStarted::new(2)),
                "global_map_started",
            ),
            (
                ExplainabilityEvent::GlobalMapBatchBuilt(GlobalMapBatchBuilt::new(
                    0,
                    vec!["report-1".to_owned()],
                    100,
                    1_000,
                )),
                "global_map_batch_built",
            ),
            (
                ExplainabilityEvent::GlobalMapPointsProduced(GlobalMapPointsProduced::try_new(
                    0,
                    vec![GlobalMapPointEvidence {
                        batch_index: 0,
                        point_index: 0,
                        score: i64::MIN,
                        answer: None,
                    }],
                )?),
                "global_map_points_produced",
            ),
            (
                ExplainabilityEvent::GlobalReduceContextBuilt(GlobalReduceContextBuilt {
                    candidate_point_count: 1,
                    positive_point_count: 1,
                    selected_point_count: 1,
                    token_budget: 1_000,
                    tokens_used: 100,
                    truncated: false,
                    points: vec![GlobalMapPointDecision {
                        batch_index: 0,
                        point_index: 0,
                        score: i64::MAX,
                        selected: true,
                        reason: GlobalMapPointDecisionReason::Selected,
                        answer: None,
                    }],
                    context: None,
                }),
                "global_reduce_context_built",
            ),
            (
                ExplainabilityEvent::GlobalReduceSkipped(GlobalReduceSkipped::new(
                    GlobalReduceSkipReason::NoPositivePoints,
                )),
                "global_reduce_skipped",
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
