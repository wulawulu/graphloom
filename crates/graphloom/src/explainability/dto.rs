//! Stable owned DTOs shared by live and persisted explainability consumers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{ExplainabilityContractError, ExplainabilityRunId};
use crate::query::SearchMethod;

/// Kind of graph or source record represented by a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExplainabilityRecordType {
    /// Input document.
    Document,
    /// Chunked text unit.
    TextUnit,
    /// Graph entity.
    Entity,
    /// Graph relationship.
    Relationship,
    /// Graph community.
    Community,
    /// Generated community report.
    CommunityReport,
    /// Claim or other covariate.
    Covariate,
}

/// Proven reason why a candidate was selected, rejected, or unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SelectionReason {
    /// Returned by approximate-nearest-neighbor retrieval.
    AnnResult,
    /// Explicitly included by the caller.
    ExplicitlyIncluded,
    /// Explicitly excluded by the caller.
    ExplicitlyExcluded,
    /// Reached through graph-neighborhood expansion.
    GraphExpansion,
    /// Reached through an entity's community membership.
    CommunityMembership,
    /// Reached through a source-record reference.
    SourceReference,
    /// Removed because it fell below the accepted rank.
    RankThreshold,
    /// Removed because the section exhausted its token budget.
    TokenBudget,
    /// Referenced identifier was stale.
    StaleReference,
    /// Referenced record was absent.
    MissingRecord,
}

/// Decision applied to one parsed Global map point during Reduce context fitting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GlobalMapPointDecisionReason {
    /// The positive point was selected for the Reduce context.
    Selected,
    /// The point was excluded because its score was zero or negative.
    NonPositiveScore,
    /// The positive point was not included after the first token-budget stop.
    TokenBudget,
}

/// Final semantic decision for one community rated by Dynamic Global Search.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "DynamicCommunityRatingEvidenceWire")]
#[non_exhaustive]
pub struct DynamicCommunityRatingEvidence {
    /// Hierarchy/traversal community identity.
    pub community_id: String,
    /// Stable persisted CommunityReport identity used for this rating.
    pub report_id: String,
    /// Real hierarchy level recorded on the rated community.
    pub level: i64,
    /// Majority-vote rating produced by the existing algorithm.
    pub selected_rating: i64,
    /// Whether the rating met the configured threshold.
    pub threshold_passed: bool,
    /// Whether the community remained in the final selected set.
    pub selected: bool,
}

#[derive(Deserialize)]
struct DynamicCommunityRatingEvidenceWire {
    community_id: String,
    report_id: String,
    level: i64,
    selected_rating: i64,
    threshold_passed: bool,
    selected: bool,
}

impl TryFrom<DynamicCommunityRatingEvidenceWire> for DynamicCommunityRatingEvidence {
    type Error = ExplainabilityContractError;

    fn try_from(wire: DynamicCommunityRatingEvidenceWire) -> Result<Self, Self::Error> {
        let evidence = Self {
            community_id: wire.community_id,
            report_id: wire.report_id,
            level: wire.level,
            selected_rating: wire.selected_rating,
            threshold_passed: wire.threshold_passed,
            selected: wire.selected,
        };
        evidence.validate()?;
        Ok(evidence)
    }
}

impl Serialize for DynamicCommunityRatingEvidence {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::{Error as _, SerializeStruct as _};

        self.validate().map_err(S::Error::custom)?;
        let mut state = serializer.serialize_struct("DynamicCommunityRatingEvidence", 6)?;
        state.serialize_field("community_id", &self.community_id)?;
        state.serialize_field("report_id", &self.report_id)?;
        state.serialize_field("level", &self.level)?;
        state.serialize_field("selected_rating", &self.selected_rating)?;
        state.serialize_field("threshold_passed", &self.threshold_passed)?;
        state.serialize_field("selected", &self.selected)?;
        state.end()
    }
}

impl DynamicCommunityRatingEvidence {
    /// Create validated final rating evidence.
    pub fn try_new(
        community_id: String,
        report_id: String,
        level: i64,
        selected_rating: i64,
        threshold_passed: bool,
        selected: bool,
    ) -> Result<Self, ExplainabilityContractError> {
        let evidence = Self {
            community_id,
            report_id,
            level,
            selected_rating,
            threshold_passed,
            selected,
        };
        evidence.validate()?;
        Ok(evidence)
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
        if self.selected && !self.threshold_passed {
            return Err(ExplainabilityContractError::InvalidDynamicRatingDecision);
        }
        Ok(())
    }
}

/// Parsed evidence produced by one Global map analyst.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct GlobalMapPointEvidence {
    /// Stable zero-based map batch index.
    pub batch_index: u32,
    /// Zero-based point order returned by `parse_map_points` for this batch.
    pub point_index: u32,
    /// Parsed GraphRAG importance score.
    pub score: i64,
    /// Exact parsed point description when content disclosure is enabled.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub answer: Option<String>,
}

impl GlobalMapPointEvidence {
    /// Create metadata-only parsed point evidence.
    #[must_use]
    pub const fn new(batch_index: u32, point_index: u32, score: i64) -> Self {
        Self {
            batch_index,
            point_index,
            score,
            answer: None,
        }
    }
}

/// Reduce-selection decision for one parsed Global map point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct GlobalMapPointDecision {
    /// Stable zero-based map batch index.
    pub batch_index: u32,
    /// Zero-based point order returned by `parse_map_points` for this batch.
    pub point_index: u32,
    /// Parsed GraphRAG importance score.
    pub score: i64,
    /// Whether the point entered the exact Reduce context.
    pub selected: bool,
    /// Proven selection or exclusion reason.
    pub reason: GlobalMapPointDecisionReason,
    /// Exact parsed point description when content disclosure is enabled.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub answer: Option<String>,
}

impl GlobalMapPointDecision {
    /// Create metadata-only Reduce decision evidence.
    #[must_use]
    pub const fn new(
        batch_index: u32,
        point_index: u32,
        score: i64,
        selected: bool,
        reason: GlobalMapPointDecisionReason,
    ) -> Self {
        Self {
            batch_index,
            point_index,
            score,
            selected,
            reason,
            answer: None,
        }
    }
}

/// Logical section of a constructed query context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContextSectionKind {
    /// Prior conversation turns.
    ConversationHistory,
    /// Community reports.
    CommunityReports,
    /// Selected entities.
    Entities,
    /// Selected relationships.
    Relationships,
    /// Selected claims or covariates.
    Covariates,
    /// Shared Local Search budget for entities, relationships, and covariates.
    LocalGraph,
    /// Source text units.
    Sources,
    /// Global-search map input.
    MapContext,
    /// Global-search reduce input.
    ReduceContext,
}

/// Explainable operation category for one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExplainabilityRunKind {
    /// Full indexing.
    Index,
    /// Incremental index update.
    Update,
    /// Query execution.
    Query,
    /// Prompt tuning.
    PromptTune,
}

/// Lifecycle status of one explainability run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExplainabilityRunStatus {
    /// Accepted but not started.
    Pending,
    /// Currently executing.
    Running,
    /// Finished successfully.
    Completed,
    /// Finished with an error.
    Failed,
    /// Cancelled before normal completion.
    Cancelled,
}

/// Stable query-method name independent of CLI parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExplainabilityQueryMethod {
    /// Basic text-unit vector search.
    Basic,
    /// Local graph-neighborhood search.
    Local,
    /// Global map/reduce search.
    Global,
    /// DRIFT exploratory search.
    Drift,
}

impl From<SearchMethod> for ExplainabilityQueryMethod {
    fn from(value: SearchMethod) -> Self {
        match value {
            SearchMethod::Basic => Self::Basic,
            SearchMethod::Local => Self::Local,
            SearchMethod::Global => Self::Global,
            SearchMethod::Drift => Self::Drift,
        }
    }
}

/// Finite ANN or ranking score safe for JSON persistence.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
#[non_exhaustive]
pub struct ExplainabilityScore(f64);

impl ExplainabilityScore {
    /// Return the finite score value.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for ExplainabilityScore {
    type Error = ExplainabilityContractError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        if value.is_finite() {
            Ok(Self(value))
        } else {
            Err(ExplainabilityContractError::NonFiniteScore)
        }
    }
}

impl From<ExplainabilityScore> for f64 {
    fn from(value: ExplainabilityScore) -> Self {
        value.get()
    }
}

/// Provider-neutral candidate metadata owned by the explainability contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExplainabilityCandidate {
    /// Stable record identifier.
    #[serde(with = "super::validation::metadata_string")]
    pub id: String,
    /// Optional human-readable short identifier.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_metadata_string"
    )]
    pub short_id: Option<String>,
    /// Optional display title.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_metadata_string"
    )]
    pub title: Option<String>,
    /// Record category.
    pub record_type: ExplainabilityRecordType,
    /// Optional finite retrieval or ranking score.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<ExplainabilityScore>,
    /// Optional one-based candidate rank.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
    /// Whether the candidate entered the relevant result set.
    pub selected: bool,
    /// Proven selection or rejection reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<SelectionReason>,
    /// Optional source record identifier.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_metadata_string"
    )]
    pub source_id: Option<String>,
    /// Optional relationship used to reach this candidate.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_metadata_string"
    )]
    pub relationship_id: Option<String>,
    /// Optional graph-expansion depth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expansion_depth: Option<u32>,
}

impl ExplainabilityCandidate {
    /// Create unselected candidate metadata with optional fields absent.
    #[must_use]
    pub fn new(id: String, record_type: ExplainabilityRecordType) -> Self {
        Self {
            id,
            short_id: None,
            title: None,
            record_type,
            score: None,
            rank: None,
            selected: false,
            reason: None,
            source_id: None,
            relationship_id: None,
            expansion_depth: None,
        }
    }
}

/// Summary of one logical section in the final context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExplainabilityContextSection {
    /// Logical section category.
    pub section: ContextSectionKind,
    /// Optional low-cardinality section name, such as a covariate group.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_metadata_string"
    )]
    pub name: Option<String>,
    /// Token budget assigned to the section.
    pub token_budget: u64,
    /// Tokens consumed by accepted section content.
    pub tokens_used: u64,
    /// Number of records considered.
    pub candidate_count: u64,
    /// Number of records accepted.
    pub selected_count: u64,
    /// Whether content was omitted because of the budget.
    pub truncated: bool,
    /// Stable identifiers of accepted records, in emitted order.
    #[serde(default, with = "super::validation::record_ids")]
    pub selected_record_ids: Vec<String>,
}

impl ExplainabilityContextSection {
    /// Create an empty section summary for an assigned budget.
    #[must_use]
    pub fn new(section: ContextSectionKind, token_budget: u64) -> Self {
        Self {
            section,
            name: None,
            token_budget,
            tokens_used: 0,
            candidate_count: 0,
            selected_count: 0,
            truncated: false,
            selected_record_ids: Vec::new(),
        }
    }
}

/// Persistable metadata describing one explainable operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExplainabilityRun {
    /// Stable run identity.
    pub run_id: ExplainabilityRunId,
    /// Operation category.
    pub kind: ExplainabilityRunKind,
    /// Current lifecycle state.
    pub status: ExplainabilityRunStatus,
    /// Full query text when the selected content mode permits it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub query: Option<String>,
    /// Query method for query runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_method: Option<ExplainabilityQueryMethod>,
    /// UTC start time.
    #[serde(with = "super::record::datetime_serde")]
    pub started_at: DateTime<Utc>,
    /// UTC completion time once terminal.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::record::datetime_serde::option"
    )]
    pub completed_at: Option<DateTime<Utc>>,
    /// Optional compatibility-profile identifier, never an absolute project path.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_metadata_string"
    )]
    pub compatibility_profile: Option<String>,
    /// Number of persisted envelopes for this run.
    pub event_count: u64,
}

impl ExplainabilityRun {
    /// Create pending run metadata with no content or completion information.
    #[must_use]
    pub fn new(
        run_id: ExplainabilityRunId,
        kind: ExplainabilityRunKind,
        started_at: DateTime<Utc>,
    ) -> Self {
        Self {
            run_id,
            kind,
            status: ExplainabilityRunStatus::Pending,
            query: None,
            query_method: None,
            started_at,
            completed_at: None,
            compatibility_profile: None,
            event_count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use chrono::{TimeZone, Utc};
    use serde::{
        Deserialize, Serialize,
        de::{DeserializeOwned, value::F64Deserializer},
    };
    use serde_json::json;

    use super::{
        ContextSectionKind, ExplainabilityCandidate, ExplainabilityContextSection,
        ExplainabilityQueryMethod, ExplainabilityRecordType, ExplainabilityRun,
        ExplainabilityRunKind, ExplainabilityRunStatus, ExplainabilityScore,
        GlobalMapPointDecision, GlobalMapPointDecisionReason, GlobalMapPointEvidence,
        SelectionReason,
    };
    use crate::{explainability::ExplainabilityRunId, query::SearchMethod};

    #[test]
    fn test_should_keep_enum_schema_values_stable() -> serde_json::Result<()> {
        assert_serialized_names(&[
            (ExplainabilityRecordType::Document, "document"),
            (ExplainabilityRecordType::TextUnit, "text_unit"),
            (ExplainabilityRecordType::Entity, "entity"),
            (ExplainabilityRecordType::Relationship, "relationship"),
            (ExplainabilityRecordType::Community, "community"),
            (
                ExplainabilityRecordType::CommunityReport,
                "community_report",
            ),
            (ExplainabilityRecordType::Covariate, "covariate"),
        ])?;
        assert_serialized_names(&[
            (GlobalMapPointDecisionReason::Selected, "selected"),
            (
                GlobalMapPointDecisionReason::NonPositiveScore,
                "non_positive_score",
            ),
            (GlobalMapPointDecisionReason::TokenBudget, "token_budget"),
        ])?;
        assert_serialized_names(&[
            (SelectionReason::AnnResult, "ann_result"),
            (SelectionReason::ExplicitlyIncluded, "explicitly_included"),
            (SelectionReason::ExplicitlyExcluded, "explicitly_excluded"),
            (SelectionReason::GraphExpansion, "graph_expansion"),
            (SelectionReason::CommunityMembership, "community_membership"),
            (SelectionReason::SourceReference, "source_reference"),
            (SelectionReason::RankThreshold, "rank_threshold"),
            (SelectionReason::TokenBudget, "token_budget"),
            (SelectionReason::StaleReference, "stale_reference"),
            (SelectionReason::MissingRecord, "missing_record"),
        ])?;
        assert_serialized_names(&[
            (
                ContextSectionKind::ConversationHistory,
                "conversation_history",
            ),
            (ContextSectionKind::CommunityReports, "community_reports"),
            (ContextSectionKind::Entities, "entities"),
            (ContextSectionKind::Relationships, "relationships"),
            (ContextSectionKind::Covariates, "covariates"),
            (ContextSectionKind::LocalGraph, "local_graph"),
            (ContextSectionKind::Sources, "sources"),
            (ContextSectionKind::MapContext, "map_context"),
            (ContextSectionKind::ReduceContext, "reduce_context"),
        ])?;
        assert_serialized_names(&[
            (ExplainabilityRunKind::Index, "index"),
            (ExplainabilityRunKind::Update, "update"),
            (ExplainabilityRunKind::Query, "query"),
            (ExplainabilityRunKind::PromptTune, "prompt_tune"),
        ])?;
        assert_serialized_names(&[
            (ExplainabilityRunStatus::Pending, "pending"),
            (ExplainabilityRunStatus::Running, "running"),
            (ExplainabilityRunStatus::Completed, "completed"),
            (ExplainabilityRunStatus::Failed, "failed"),
            (ExplainabilityRunStatus::Cancelled, "cancelled"),
        ])?;
        assert_serialized_names(&[
            (ExplainabilityQueryMethod::Basic, "basic"),
            (ExplainabilityQueryMethod::Local, "local"),
            (ExplainabilityQueryMethod::Global, "global"),
            (ExplainabilityQueryMethod::Drift, "drift"),
        ])?;
        assert_eq!(
            ExplainabilityQueryMethod::from(SearchMethod::Local),
            ExplainabilityQueryMethod::Local
        );
        assert!(serde_json::from_value::<ExplainabilityRecordType>(json!("unknown")).is_err());
        assert!(serde_json::from_value::<SelectionReason>(json!("unknown")).is_err());
        assert!(serde_json::from_value::<ContextSectionKind>(json!("unknown")).is_err());
        assert!(serde_json::from_value::<ExplainabilityRunKind>(json!("unknown")).is_err());
        assert!(serde_json::from_value::<ExplainabilityRunStatus>(json!("unknown")).is_err());
        assert!(serde_json::from_value::<ExplainabilityQueryMethod>(json!("unknown")).is_err());
        Ok(())
    }

    #[test]
    fn test_should_round_trip_global_point_identity_scores_and_optional_content()
    -> Result<(), Box<dyn std::error::Error>> {
        let evidence = GlobalMapPointEvidence {
            batch_index: u32::MAX,
            point_index: u32::MAX,
            score: i64::MIN,
            answer: Some("exact point".to_owned()),
        };
        let decision = GlobalMapPointDecision {
            batch_index: 2,
            point_index: 3,
            score: i64::MAX,
            selected: false,
            reason: GlobalMapPointDecisionReason::TokenBudget,
            answer: None,
        };
        assert_eq!(
            serde_json::from_value::<GlobalMapPointEvidence>(serde_json::to_value(&evidence)?)?,
            evidence
        );
        assert_eq!(
            serde_json::from_value::<GlobalMapPointDecision>(serde_json::to_value(&decision)?)?,
            decision
        );
        assert!(
            serde_json::from_value::<GlobalMapPointEvidence>(json!({
                "batch_index": u64::from(u32::MAX) + 1,
                "point_index": 0,
                "score": 1,
            }))
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn test_should_reject_non_finite_scores_at_construction_and_deserialization() {
        for score in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(ExplainabilityScore::try_from(score).is_err());
            let deserializer = F64Deserializer::<serde::de::value::Error>::new(score);
            assert!(ExplainabilityScore::deserialize(deserializer).is_err());
        }
        assert!(serde_json::from_str::<ExplainabilityScore>("null").is_err());
    }

    #[test]
    fn test_should_round_trip_candidate_and_preserve_absent_options()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut candidate =
            ExplainabilityCandidate::new("entity-1".to_owned(), ExplainabilityRecordType::Entity);
        candidate.score = Some(ExplainabilityScore::try_from(0.875)?);
        candidate.rank = Some(1);
        candidate.selected = true;
        candidate.reason = Some(SelectionReason::AnnResult);

        let value = serde_json::to_value(&candidate)?;
        assert_eq!(
            value.get("score").and_then(serde_json::Value::as_f64),
            Some(0.875)
        );
        assert_eq!(
            value.get("rank").and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert_eq!(
            value.get("selected").and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert!(value.get("short_id").is_none());
        assert!(value.get("source_id").is_none());
        assert_eq!(
            serde_json::from_value::<ExplainabilityCandidate>(value)?,
            candidate
        );
        Ok(())
    }

    #[test]
    fn test_should_round_trip_empty_context_section_as_empty_array()
    -> Result<(), Box<dyn std::error::Error>> {
        let section = ExplainabilityContextSection::new(ContextSectionKind::Sources, 1_024);
        let value = serde_json::to_value(&section)?;
        assert_eq!(value.get("selected_record_ids"), Some(&json!([])));
        assert_eq!(
            value
                .get("selected_count")
                .and_then(serde_json::Value::as_u64),
            Some(0)
        );
        assert_eq!(
            serde_json::from_value::<ExplainabilityContextSection>(value)?,
            section
        );
        Ok(())
    }

    #[test]
    fn test_should_round_trip_bounded_covariate_section_name() -> serde_json::Result<()> {
        let mut section = ExplainabilityContextSection::new(ContextSectionKind::Covariates, 1_024);
        section.name = Some("claims".to_owned());
        let value = serde_json::to_value(&section)?;
        assert_eq!(value.get("name"), Some(&json!("claims")));
        assert_eq!(
            serde_json::from_value::<ExplainabilityContextSection>(value)?,
            section
        );

        section.name = Some("x".repeat(257));
        assert!(serde_json::to_value(section).is_err());
        Ok(())
    }

    #[test]
    fn test_should_round_trip_run_with_omitted_metadata_content()
    -> Result<(), Box<dyn std::error::Error>> {
        let started_at = Utc
            .with_ymd_and_hms(2026, 8, 3, 9, 0, 0)
            .single()
            .ok_or("test timestamp must be representable")?;
        let run = ExplainabilityRun::new(
            ExplainabilityRunId::from_str("run-1")?,
            ExplainabilityRunKind::Query,
            started_at,
        );
        let value = serde_json::to_value(&run)?;
        assert!(value.get("query").is_none());
        assert!(value.get("query_method").is_none());
        assert!(value.get("completed_at").is_none());
        assert_eq!(serde_json::from_value::<ExplainabilityRun>(value)?, run);
        Ok(())
    }

    fn assert_serialized_names<T>(values: &[(T, &str)]) -> serde_json::Result<()>
    where
        T: Copy + std::fmt::Debug + PartialEq + Serialize + DeserializeOwned,
    {
        for (value, expected) in values {
            let json = json!(*expected);
            assert_eq!(serde_json::to_value(value)?, json);
            assert_eq!(serde_json::from_value::<T>(json)?, *value);
        }
        Ok(())
    }
}
