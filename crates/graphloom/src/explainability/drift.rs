//! DRIFT-specific explainability payloads and contract validation.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{ExplainabilityContractError, ExplainabilityScore};

fn invalid(reason: &'static str) -> ExplainabilityContractError {
    ExplainabilityContractError::InvalidDriftMetadata { reason }
}

/// The real randomly selected community-report template used for HyDE.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "DriftHydeStartedWire")]
#[non_exhaustive]
pub struct DriftHydeStarted {
    /// Stable persisted [`CommunityReport`](crate::query::CommunityReport) identifier.
    #[serde(with = "super::validation::metadata_string")]
    pub template_report_id: String,
    /// Human-readable report short identifier.
    #[serde(with = "super::validation::metadata_string")]
    pub template_short_id: String,
    /// Community identifier attached to the report.
    #[serde(with = "super::validation::metadata_string")]
    pub template_community_id: String,
    /// Zero-based index selected from the real report array.
    pub template_index: u32,
    /// Number of reports available to the random selection.
    pub report_count: u32,
}

#[derive(Deserialize)]
struct DriftHydeStartedWire {
    #[serde(with = "super::validation::metadata_string")]
    template_report_id: String,
    #[serde(with = "super::validation::metadata_string")]
    template_short_id: String,
    #[serde(with = "super::validation::metadata_string")]
    template_community_id: String,
    template_index: u32,
    report_count: u32,
}

impl DriftHydeStarted {
    /// Create validated HyDE template evidence.
    ///
    /// # Errors
    ///
    /// Returns an error when the report count is zero or the selected index is out of range.
    pub fn try_new(
        template_report_id: String,
        template_short_id: String,
        template_community_id: String,
        template_index: u32,
        report_count: u32,
    ) -> Result<Self, ExplainabilityContractError> {
        if report_count == 0 || template_index >= report_count {
            return Err(invalid(
                "HyDE template index must be within a non-empty report array",
            ));
        }
        Ok(Self {
            template_report_id,
            template_short_id,
            template_community_id,
            template_index,
            report_count,
        })
    }
}

impl TryFrom<DriftHydeStartedWire> for DriftHydeStarted {
    type Error = ExplainabilityContractError;

    fn try_from(wire: DriftHydeStartedWire) -> Result<Self, Self::Error> {
        Self::try_new(
            wire.template_report_id,
            wire.template_short_id,
            wire.template_community_id,
            wire.template_index,
            wire.report_count,
        )
    }
}

/// HyDE expansion completed and chose its effective embedding input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DriftHydeCompleted {
    /// Whether an empty provider output caused the original query to be used.
    pub used_original_query: bool,
}

/// One report in the exact effective cosine ranking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DriftRankedReportEvidence {
    /// Stable persisted report identifier.
    #[serde(with = "super::validation::metadata_string")]
    pub report_id: String,
    /// Human-readable report short identifier.
    #[serde(with = "super::validation::metadata_string")]
    pub short_id: String,
    /// Community identifier attached to the report.
    #[serde(with = "super::validation::metadata_string")]
    pub community_id: String,
    /// Finite cosine similarity used by the real ranking.
    pub similarity: ExplainabilityScore,
    /// One-based effective rank.
    pub rank: u32,
}

/// The exact truncated community-report ranking used by Primer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "DriftReportsRankedWire")]
#[non_exhaustive]
pub struct DriftReportsRanked {
    /// Reports in real ranking order; the frontend must not re-sort them.
    #[serde(default, with = "super::validation::drift_ranked_reports")]
    pub reports: Vec<DriftRankedReportEvidence>,
}

#[derive(Deserialize)]
struct DriftReportsRankedWire {
    #[serde(default, with = "super::validation::drift_ranked_reports")]
    reports: Vec<DriftRankedReportEvidence>,
}

impl DriftReportsRanked {
    /// Create ranking evidence with consecutive one-based ranks.
    ///
    /// # Errors
    ///
    /// Returns an error when a report's rank differs from its position.
    pub fn try_new(
        reports: Vec<DriftRankedReportEvidence>,
    ) -> Result<Self, ExplainabilityContractError> {
        if reports
            .iter()
            .enumerate()
            .any(|(index, report)| u32::try_from(index.saturating_add(1)).ok() != Some(report.rank))
        {
            return Err(invalid(
                "ranked reports must have consecutive one-based ranks",
            ));
        }
        Ok(Self { reports })
    }
}

impl TryFrom<DriftReportsRankedWire> for DriftReportsRanked {
    type Error = ExplainabilityContractError;

    fn try_from(wire: DriftReportsRankedWire) -> Result<Self, Self::Error> {
        Self::try_new(wire.reports)
    }
}

/// Primer fan-out began with the effective NumPy-compatible fold count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "DriftPrimerStartedWire")]
#[non_exhaustive]
pub struct DriftPrimerStarted {
    /// Number of real fold requests, including empty folds.
    pub fold_count: u32,
    /// Number of ranked reports split across the folds.
    pub ranked_report_count: u32,
}

#[derive(Deserialize)]
struct DriftPrimerStartedWire {
    fold_count: u32,
    ranked_report_count: u32,
}

impl DriftPrimerStarted {
    /// Create Primer stage evidence.
    ///
    /// # Errors
    ///
    /// Returns an error when the effective fold count is zero.
    pub fn try_new(
        fold_count: u32,
        ranked_report_count: u32,
    ) -> Result<Self, ExplainabilityContractError> {
        if fold_count == 0 {
            return Err(invalid("Primer fold count must be greater than zero"));
        }
        Ok(Self {
            fold_count,
            ranked_report_count,
        })
    }
}

impl TryFrom<DriftPrimerStartedWire> for DriftPrimerStarted {
    type Error = ExplainabilityContractError;

    fn try_from(wire: DriftPrimerStartedWire) -> Result<Self, Self::Error> {
        Self::try_new(wire.fold_count, wire.ranked_report_count)
    }
}

/// One real Primer fold request began.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "DriftPrimerFoldStartedWire")]
#[non_exhaustive]
pub struct DriftPrimerFoldStarted {
    /// Zero-based fold index.
    pub fold_index: u32,
    /// Total real fold count.
    pub fold_count: u32,
    /// Stable report IDs in this fold; an empty array is valid.
    #[serde(default, with = "super::validation::record_ids")]
    pub report_ids: Vec<String>,
}

#[derive(Deserialize)]
struct DriftPrimerFoldStartedWire {
    fold_index: u32,
    fold_count: u32,
    #[serde(default, with = "super::validation::record_ids")]
    report_ids: Vec<String>,
}

impl DriftPrimerFoldStarted {
    /// Create validated fold identity evidence.
    ///
    /// # Errors
    ///
    /// Returns an error when the fold index is outside the declared fold count.
    pub fn try_new(
        fold_index: u32,
        fold_count: u32,
        report_ids: Vec<String>,
    ) -> Result<Self, ExplainabilityContractError> {
        if fold_count == 0 || fold_index >= fold_count {
            return Err(invalid("Primer fold index must be within the fold count"));
        }
        Ok(Self {
            fold_index,
            fold_count,
            report_ids,
        })
    }
}

impl TryFrom<DriftPrimerFoldStartedWire> for DriftPrimerFoldStarted {
    type Error = ExplainabilityContractError;

    fn try_from(wire: DriftPrimerFoldStartedWire) -> Result<Self, Self::Error> {
        Self::try_new(wire.fold_index, wire.fold_count, wire.report_ids)
    }
}

/// One parsed Primer fold response completed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "DriftPrimerFoldCompletedWire")]
#[non_exhaustive]
pub struct DriftPrimerFoldCompleted {
    /// Zero-based fold index.
    pub fold_index: u32,
    /// Parsed finite Primer score.
    pub score: ExplainabilityScore,
    /// Number of parsed follow-up queries.
    pub follow_up_count: u64,
    /// Parsed intermediate answer when content capture is enabled.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub intermediate_answer: Option<String>,
    /// Parsed follow-up queries when content capture is enabled.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_strings"
    )]
    pub follow_up_queries: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct DriftPrimerFoldCompletedWire {
    fold_index: u32,
    score: ExplainabilityScore,
    follow_up_count: u64,
    #[serde(default, with = "super::validation::optional_content_string")]
    intermediate_answer: Option<String>,
    #[serde(default, with = "super::validation::optional_content_strings")]
    follow_up_queries: Option<Vec<String>>,
}

impl DriftPrimerFoldCompleted {
    /// Create parsed fold evidence.
    ///
    /// # Errors
    ///
    /// Returns an error when captured follow-ups contradict the metadata count.
    pub fn try_new(
        fold_index: u32,
        score: ExplainabilityScore,
        follow_up_count: u64,
        intermediate_answer: Option<String>,
        follow_up_queries: Option<Vec<String>>,
    ) -> Result<Self, ExplainabilityContractError> {
        validate_optional_count(
            follow_up_count,
            follow_up_queries.as_deref(),
            "Primer fold follow-up count must match captured queries",
        )?;
        Ok(Self {
            fold_index,
            score,
            follow_up_count,
            intermediate_answer,
            follow_up_queries,
        })
    }
}

impl TryFrom<DriftPrimerFoldCompletedWire> for DriftPrimerFoldCompleted {
    type Error = ExplainabilityContractError;

    fn try_from(wire: DriftPrimerFoldCompletedWire) -> Result<Self, Self::Error> {
        Self::try_new(
            wire.fold_index,
            wire.score,
            wire.follow_up_count,
            wire.intermediate_answer,
            wire.follow_up_queries,
        )
    }
}

/// The backend's real Primer aggregate and root graph application completed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "DriftPrimerCompletedWire")]
#[non_exhaustive]
pub struct DriftPrimerCompleted {
    /// Backend-computed aggregate score.
    pub score: ExplainabilityScore,
    /// Root action node ID.
    pub root_action_id: u64,
    /// Aggregate follow-up count.
    pub follow_up_count: u64,
    /// Target IDs in edge insertion order; duplicates are intentionally preserved.
    #[serde(default, with = "super::validation::action_ids")]
    pub follow_up_action_ids: Vec<u64>,
    /// Aggregate answer when content capture is enabled.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub answer: Option<String>,
    /// Aggregate follow-up queries when content capture is enabled.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_strings"
    )]
    pub follow_up_queries: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct DriftPrimerCompletedWire {
    score: ExplainabilityScore,
    root_action_id: u64,
    follow_up_count: u64,
    #[serde(default, with = "super::validation::action_ids")]
    follow_up_action_ids: Vec<u64>,
    #[serde(default, with = "super::validation::optional_content_string")]
    answer: Option<String>,
    #[serde(default, with = "super::validation::optional_content_strings")]
    follow_up_queries: Option<Vec<String>>,
}

impl DriftPrimerCompleted {
    /// Create aggregate/root evidence while preserving duplicate target IDs.
    ///
    /// # Errors
    ///
    /// Returns an error when target IDs or captured queries contradict the aggregate count.
    pub fn try_new(
        score: ExplainabilityScore,
        root_action_id: u64,
        follow_up_count: u64,
        follow_up_action_ids: Vec<u64>,
        answer: Option<String>,
        follow_up_queries: Option<Vec<String>>,
    ) -> Result<Self, ExplainabilityContractError> {
        validate_count(
            follow_up_count,
            follow_up_action_ids.len(),
            "Primer target ID count must match follow-up count",
        )?;
        validate_optional_count(
            follow_up_count,
            follow_up_queries.as_deref(),
            "Primer follow-up count must match captured queries",
        )?;
        Ok(Self {
            score,
            root_action_id,
            follow_up_count,
            follow_up_action_ids,
            answer,
            follow_up_queries,
        })
    }
}

impl TryFrom<DriftPrimerCompletedWire> for DriftPrimerCompleted {
    type Error = ExplainabilityContractError;

    fn try_from(wire: DriftPrimerCompletedWire) -> Result<Self, Self::Error> {
        Self::try_new(
            wire.score,
            wire.root_action_id,
            wire.follow_up_count,
            wire.follow_up_action_ids,
            wire.answer,
            wire.follow_up_queries,
        )
    }
}

/// Exploration configuration anchored to the root action graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DriftExplorationStarted {
    /// Configured number of zero-based depth waves; zero is valid.
    pub max_depth: u32,
    /// Maximum selected incomplete actions per wave.
    pub selection_limit: u64,
    /// Primer root action ID.
    pub root_action_id: u64,
}

/// The real random action selection decision for one depth wave.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "DriftDepthActionsSelectedWire")]
#[non_exhaustive]
pub struct DriftDepthActionsSelected {
    /// Zero-based DRIFT loop depth.
    pub depth_index: u32,
    /// Incomplete IDs before the real shuffle, in state insertion order.
    #[serde(default, with = "super::validation::action_ids")]
    pub candidate_action_ids: Vec<u64>,
    /// IDs after the same real shuffle and truncation.
    #[serde(default, with = "super::validation::action_ids")]
    pub selected_action_ids: Vec<u64>,
    /// Configured truncation limit.
    pub selection_limit: u64,
}

#[derive(Deserialize)]
struct DriftDepthActionsSelectedWire {
    depth_index: u32,
    #[serde(default, with = "super::validation::action_ids")]
    candidate_action_ids: Vec<u64>,
    #[serde(default, with = "super::validation::action_ids")]
    selected_action_ids: Vec<u64>,
    selection_limit: u64,
}

impl DriftDepthActionsSelected {
    /// Create a real depth selection decision.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate candidates, a non-subset selection, or excess selection.
    pub fn try_new(
        depth_index: u32,
        candidate_action_ids: Vec<u64>,
        selected_action_ids: Vec<u64>,
        selection_limit: u64,
    ) -> Result<Self, ExplainabilityContractError> {
        let candidates = candidate_action_ids.iter().copied().collect::<HashSet<_>>();
        if candidates.len() != candidate_action_ids.len() {
            return Err(invalid("depth candidate action IDs must be unique"));
        }
        let selected = selected_action_ids.iter().copied().collect::<HashSet<_>>();
        if selected.len() != selected_action_ids.len() || !selected.is_subset(&candidates) {
            return Err(invalid(
                "selected action IDs must be a unique subset of candidates",
            ));
        }
        if u64::try_from(selected_action_ids.len())
            .ok()
            .is_none_or(|count| count > selection_limit)
        {
            return Err(invalid(
                "selected action count must not exceed the selection limit",
            ));
        }
        Ok(Self {
            depth_index,
            candidate_action_ids,
            selected_action_ids,
            selection_limit,
        })
    }
}

impl TryFrom<DriftDepthActionsSelectedWire> for DriftDepthActionsSelected {
    type Error = ExplainabilityContractError;

    fn try_from(wire: DriftDepthActionsSelectedWire) -> Result<Self, Self::Error> {
        Self::try_new(
            wire.depth_index,
            wire.candidate_action_ids,
            wire.selected_action_ids,
            wire.selection_limit,
        )
    }
}

/// One real action attempt began on its own span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DriftActionAttemptStarted {
    /// Zero-based DRIFT loop depth.
    pub depth_index: u32,
    /// Stable action node ID within this Run.
    pub action_id: u64,
    /// Exact follow-up query when content capture is enabled.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub query: Option<String>,
}

/// The exact Local context used by one action attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DriftActionContextBuilt {
    /// Action node ID.
    pub action_id: u64,
    /// Exact Local context when content capture is enabled.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub context: Option<String>,
}

/// One parsed action response was successfully applied to the state graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "DriftActionAttemptCompletedWire")]
#[non_exhaustive]
pub struct DriftActionAttemptCompleted {
    /// Zero-based DRIFT loop depth.
    pub depth_index: u32,
    /// Action node ID.
    pub action_id: u64,
    /// Whether the parsed response contained `Some(answer)`.
    pub answer_present: bool,
    /// Whether that present answer was not the empty string; whitespace is not trimmed.
    pub answer_non_empty: bool,
    /// Finite parsed action score, absent for `-inf`, `inf`, or NaN.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<ExplainabilityScore>,
    /// Parsed follow-up count.
    pub follow_up_count: u64,
    /// Applied target IDs in edge insertion order; duplicates are preserved.
    #[serde(default, with = "super::validation::action_ids")]
    pub target_action_ids: Vec<u64>,
    /// Parsed answer when content capture is enabled.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub answer: Option<String>,
    /// Parsed follow-up query strings when content capture is enabled.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_strings"
    )]
    pub follow_up_queries: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct DriftActionAttemptCompletedWire {
    depth_index: u32,
    action_id: u64,
    answer_present: bool,
    answer_non_empty: bool,
    #[serde(default)]
    score: Option<ExplainabilityScore>,
    follow_up_count: u64,
    #[serde(default, with = "super::validation::action_ids")]
    target_action_ids: Vec<u64>,
    #[serde(default, with = "super::validation::optional_content_string")]
    answer: Option<String>,
    #[serde(default, with = "super::validation::optional_content_strings")]
    follow_up_queries: Option<Vec<String>>,
}

impl DriftActionAttemptCompleted {
    /// Create applied action-attempt evidence.
    ///
    /// # Errors
    ///
    /// Returns an error for contradictory answer flags or follow-up collection counts.
    #[allow(
        clippy::too_many_arguments,
        reason = "the fallible constructor validates one wire payload atomically; grouping fields \
                  would weaken the public event contract"
    )]
    pub fn try_new(
        depth_index: u32,
        action_id: u64,
        answer_present: bool,
        answer_non_empty: bool,
        score: Option<ExplainabilityScore>,
        follow_up_count: u64,
        target_action_ids: Vec<u64>,
        answer: Option<String>,
        follow_up_queries: Option<Vec<String>>,
    ) -> Result<Self, ExplainabilityContractError> {
        if answer_non_empty && !answer_present {
            return Err(invalid("a non-empty answer must be present"));
        }
        if let Some(captured) = answer.as_deref()
            && (!answer_present || captured.is_empty() == answer_non_empty)
        {
            return Err(invalid(
                "captured answer must match answer presence and empty-string flags",
            ));
        }
        validate_count(
            follow_up_count,
            target_action_ids.len(),
            "action target ID count must match follow-up count",
        )?;
        validate_optional_count(
            follow_up_count,
            follow_up_queries.as_deref(),
            "action follow-up count must match captured queries",
        )?;
        Ok(Self {
            depth_index,
            action_id,
            answer_present,
            answer_non_empty,
            score,
            follow_up_count,
            target_action_ids,
            answer,
            follow_up_queries,
        })
    }
}

impl TryFrom<DriftActionAttemptCompletedWire> for DriftActionAttemptCompleted {
    type Error = ExplainabilityContractError;

    fn try_from(wire: DriftActionAttemptCompletedWire) -> Result<Self, Self::Error> {
        Self::try_new(
            wire.depth_index,
            wire.action_id,
            wire.answer_present,
            wire.answer_non_empty,
            wire.score,
            wire.follow_up_count,
            wire.target_action_ids,
            wire.answer,
            wire.follow_up_queries,
        )
    }
}

/// Exact final state and Python-list Reduce inputs built by the backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "DriftReduceContextBuiltWire")]
#[non_exhaustive]
pub struct DriftReduceContextBuilt {
    /// Number of query-identity action nodes.
    pub node_count: u64,
    /// Number of multigraph edges, including duplicates.
    pub edge_count: u64,
    /// Number of answers included in Reduce.
    pub included_answer_count: u64,
    /// Included action IDs in node insertion order.
    #[serde(default, with = "super::validation::action_ids")]
    pub included_action_ids: Vec<u64>,
    /// Exact `state.to_json()` output when content capture is enabled.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub state_context: Option<String>,
    /// Exact `python_list_repr()` output when content capture is enabled.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::validation::optional_content_string"
    )]
    pub reduce_context: Option<String>,
}

#[derive(Deserialize)]
struct DriftReduceContextBuiltWire {
    node_count: u64,
    edge_count: u64,
    included_answer_count: u64,
    #[serde(default, with = "super::validation::action_ids")]
    included_action_ids: Vec<u64>,
    #[serde(default, with = "super::validation::optional_content_string")]
    state_context: Option<String>,
    #[serde(default, with = "super::validation::optional_content_string")]
    reduce_context: Option<String>,
}

impl DriftReduceContextBuilt {
    /// Create final Reduce-selection evidence.
    ///
    /// # Errors
    ///
    /// Returns an error when the included-answer count differs from the action-ID collection.
    pub fn try_new(
        node_count: u64,
        edge_count: u64,
        included_answer_count: u64,
        included_action_ids: Vec<u64>,
        state_context: Option<String>,
        reduce_context: Option<String>,
    ) -> Result<Self, ExplainabilityContractError> {
        validate_count(
            included_answer_count,
            included_action_ids.len(),
            "included action ID count must match included answer count",
        )?;
        Ok(Self {
            node_count,
            edge_count,
            included_answer_count,
            included_action_ids,
            state_context,
            reduce_context,
        })
    }
}

impl TryFrom<DriftReduceContextBuiltWire> for DriftReduceContextBuilt {
    type Error = ExplainabilityContractError;

    fn try_from(wire: DriftReduceContextBuiltWire) -> Result<Self, Self::Error> {
        Self::try_new(
            wire.node_count,
            wire.edge_count,
            wire.included_answer_count,
            wire.included_action_ids,
            wire.state_context,
            wire.reduce_context,
        )
    }
}

fn validate_count(
    expected: u64,
    actual: usize,
    reason: &'static str,
) -> Result<(), ExplainabilityContractError> {
    if u64::try_from(actual).ok() != Some(expected) {
        return Err(invalid(reason));
    }
    Ok(())
}

fn validate_optional_count(
    expected: u64,
    actual: Option<&[String]>,
    reason: &'static str,
) -> Result<(), ExplainabilityContractError> {
    if let Some(actual) = actual {
        validate_count(expected, actual.len(), reason)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::explainability::{ExplainabilityEvent, ExplainabilityScore};

    fn score(value: f64) -> ExplainabilityScore {
        ExplainabilityScore::try_from(value).expect("finite test score")
    }

    #[test]
    fn test_should_round_trip_drift_events_with_snake_case_discriminators()
    -> Result<(), Box<dyn std::error::Error>> {
        let events = vec![
            ExplainabilityEvent::DriftHydeStarted(DriftHydeStarted::try_new(
                "report-id".to_owned(),
                "R1".to_owned(),
                "community-id".to_owned(),
                0,
                1,
            )?),
            ExplainabilityEvent::DriftHydeCompleted(DriftHydeCompleted {
                used_original_query: true,
            }),
            ExplainabilityEvent::DriftReportsRanked(DriftReportsRanked::try_new(vec![
                DriftRankedReportEvidence {
                    report_id: "report-id".to_owned(),
                    short_id: "R1".to_owned(),
                    community_id: "community-id".to_owned(),
                    similarity: score(0.75),
                    rank: 1,
                },
            ])?),
            ExplainabilityEvent::DriftPrimerStarted(DriftPrimerStarted::try_new(2, 1)?),
            ExplainabilityEvent::DriftPrimerFoldStarted(DriftPrimerFoldStarted::try_new(
                1,
                2,
                Vec::new(),
            )?),
            ExplainabilityEvent::DriftPrimerFoldCompleted(DriftPrimerFoldCompleted::try_new(
                1,
                score(80.0),
                0,
                None,
                None,
            )?),
            ExplainabilityEvent::DriftPrimerCompleted(DriftPrimerCompleted::try_new(
                score(75.0),
                0,
                2,
                vec![1, 1],
                None,
                None,
            )?),
            ExplainabilityEvent::DriftExplorationStarted(DriftExplorationStarted {
                max_depth: 0,
                selection_limit: 2,
                root_action_id: 0,
            }),
            ExplainabilityEvent::DriftDepthActionsSelected(DriftDepthActionsSelected::try_new(
                0,
                vec![1, 2, 3],
                vec![3, 1],
                2,
            )?),
            ExplainabilityEvent::DriftActionAttemptStarted(DriftActionAttemptStarted {
                depth_index: 0,
                action_id: 3,
                query: None,
            }),
            ExplainabilityEvent::DriftActionContextBuilt(DriftActionContextBuilt {
                action_id: 3,
                context: None,
            }),
            ExplainabilityEvent::DriftActionAttemptCompleted(DriftActionAttemptCompleted::try_new(
                0,
                3,
                true,
                false,
                None,
                2,
                vec![4, 4],
                None,
                None,
            )?),
            ExplainabilityEvent::DriftReduceContextBuilt(DriftReduceContextBuilt::try_new(
                5,
                4,
                2,
                vec![0, 3],
                None,
                None,
            )?),
        ];
        let expected = [
            "drift_hyde_started",
            "drift_hyde_completed",
            "drift_reports_ranked",
            "drift_primer_started",
            "drift_primer_fold_started",
            "drift_primer_fold_completed",
            "drift_primer_completed",
            "drift_exploration_started",
            "drift_depth_actions_selected",
            "drift_action_attempt_started",
            "drift_action_context_built",
            "drift_action_attempt_completed",
            "drift_reduce_context_built",
        ];

        for (event, expected_type) in events.into_iter().zip(expected) {
            let value = serde_json::to_value(&event)?;
            assert_eq!(value["type"], expected_type);
            assert_eq!(serde_json::from_value::<ExplainabilityEvent>(value)?, event);
        }
        Ok(())
    }

    #[test]
    fn test_should_preserve_duplicate_graph_edges_and_allow_empty_folds()
    -> Result<(), Box<dyn std::error::Error>> {
        let fold = DriftPrimerFoldStarted::try_new(3, 4, Vec::new())?;
        assert!(fold.report_ids.is_empty());

        let primer = DriftPrimerCompleted::try_new(
            score(80.0),
            0,
            2,
            vec![7, 7],
            Some("primer".to_owned()),
            Some(vec!["same".to_owned(), "same".to_owned()]),
        )?;
        assert_eq!(primer.follow_up_action_ids, [7, 7]);

        let action = DriftActionAttemptCompleted::try_new(
            1,
            2,
            true,
            true,
            Some(score(70.0)),
            2,
            vec![7, 7],
            Some("answer".to_owned()),
            Some(vec!["same".to_owned(), "same".to_owned()]),
        )?;
        assert_eq!(action.target_action_ids, [7, 7]);
        Ok(())
    }

    #[test]
    fn test_should_validate_answer_and_collection_semantics() {
        assert!(
            DriftActionAttemptCompleted::try_new(
                0,
                1,
                false,
                true,
                None,
                0,
                Vec::new(),
                None,
                None,
            )
            .is_err()
        );
        assert!(
            DriftActionAttemptCompleted::try_new(
                0,
                1,
                true,
                false,
                None,
                0,
                Vec::new(),
                Some(String::new()),
                Some(Vec::new()),
            )
            .is_ok()
        );
        assert!(
            DriftActionAttemptCompleted::try_new(
                0,
                1,
                true,
                true,
                None,
                0,
                Vec::new(),
                Some("   ".to_owned()),
                Some(Vec::new()),
            )
            .is_ok()
        );
        assert!(DriftDepthActionsSelected::try_new(0, vec![1, 2], vec![2, 3], 2).is_err());
        assert!(DriftReduceContextBuilt::try_new(2, 1, 2, vec![0], None, None).is_err());
    }

    #[test]
    fn test_should_redact_all_content_fields_in_metadata_payloads()
    -> Result<(), Box<dyn std::error::Error>> {
        let values = [
            serde_json::to_value(DriftPrimerFoldCompleted::try_new(
                0,
                score(50.0),
                1,
                None,
                None,
            )?)?,
            serde_json::to_value(DriftActionAttemptCompleted::try_new(
                0,
                1,
                false,
                false,
                None,
                0,
                Vec::new(),
                None,
                None,
            )?)?,
            serde_json::to_value(DriftReduceContextBuilt::try_new(
                2,
                1,
                1,
                vec![0],
                None,
                None,
            )?)?,
        ];
        let forbidden = [
            "intermediate_answer",
            "follow_up_queries",
            "answer",
            "state_context",
            "reduce_context",
        ];
        for value in values {
            let object = value.as_object().ok_or("payload must be an object")?;
            for field in forbidden {
                assert!(
                    !object.contains_key(field),
                    "unexpected field {field}: {value}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn test_should_reject_invalid_deserialized_drift_contracts() {
        let invalid_rank = json!({
            "reports": [{
                "report_id": "report-id",
                "short_id": "R1",
                "community_id": "community-id",
                "similarity": 0.5,
                "rank": 2
            }]
        });
        assert!(serde_json::from_value::<DriftReportsRanked>(invalid_rank).is_err());
        let invalid_answer: Value = json!({
            "depth_index": 0,
            "action_id": 1,
            "answer_present": true,
            "answer_non_empty": true,
            "follow_up_count": 0,
            "target_action_ids": [],
            "answer": ""
        });
        assert!(serde_json::from_value::<DriftActionAttemptCompleted>(invalid_answer).is_err());
    }
}
