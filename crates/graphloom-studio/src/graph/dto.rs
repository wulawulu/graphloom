use std::{collections::BTreeMap, fmt};

use graphloom::query::{Community, CommunityReport, Entity, Relationship};
use serde::Serialize;

/// Bounded graph overview.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct GraphSummary {
    /// Number of Query-visible entities.
    pub entity_count: u64,
    /// Number of Query-visible relationships.
    pub relationship_count: u64,
    /// Number of report-backed communities.
    pub community_count: u64,
    /// Number of Query-readable community reports.
    pub community_report_count: u64,
    /// Distinct hierarchy levels in ascending order.
    pub community_levels: Vec<i64>,
    /// Typed entity counts ordered by the exact type string.
    pub entity_types: BTreeMap<String, u64>,
    /// Number of entities without a type.
    pub untyped_entity_count: u64,
}

/// Compact entity list item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct GraphEntity {
    /// Stable entity UUID.
    pub id: String,
    /// Human-readable id.
    pub short_id: Option<String>,
    /// Entity title.
    pub title: String,
    /// Optional entity type.
    pub entity_type: Option<String>,
    /// Optional degree-derived rank.
    pub rank: Option<i64>,
    /// Query-visible community memberships.
    pub community_ids: Vec<String>,
}

/// Full entity detail, excluding embedding data.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct GraphEntityDetail {
    /// Stable entity UUID.
    pub id: String,
    /// Human-readable id.
    pub short_id: Option<String>,
    /// Entity title.
    pub title: String,
    /// Optional entity type.
    pub entity_type: Option<String>,
    /// Optional entity description.
    pub description: Option<String>,
    /// Optional degree-derived rank.
    pub rank: Option<i64>,
    /// Query-visible community memberships.
    pub community_ids: Vec<String>,
    /// Referenced text-unit ids.
    pub text_unit_ids: Vec<String>,
}

impl fmt::Debug for GraphEntityDetail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GraphEntityDetail { .. }")
    }
}

impl GraphEntityDetail {
    /// Create a minimal entity detail with optional fields and collections empty.
    #[must_use]
    pub fn new(id: String, title: String) -> Self {
        Self {
            id,
            short_id: None,
            title,
            entity_type: None,
            description: None,
            rank: None,
            community_ids: Vec::new(),
            text_unit_ids: Vec::new(),
        }
    }

    /// Set the human-readable id.
    #[must_use]
    pub fn with_short_id(mut self, short_id: String) -> Self {
        self.short_id = Some(short_id);
        self
    }

    /// Set the exact entity type.
    #[must_use]
    pub fn with_entity_type(mut self, entity_type: String) -> Self {
        self.entity_type = Some(entity_type);
        self
    }

    /// Set the entity description.
    #[must_use]
    pub fn with_description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }

    /// Set the degree-derived rank.
    #[must_use]
    pub const fn with_rank(mut self, rank: i64) -> Self {
        self.rank = Some(rank);
        self
    }

    /// Set Query-visible community memberships.
    #[must_use]
    pub fn with_community_ids(mut self, community_ids: Vec<String>) -> Self {
        self.community_ids = community_ids;
        self
    }

    /// Set referenced text-unit ids.
    #[must_use]
    pub fn with_text_unit_ids(mut self, text_unit_ids: Vec<String>) -> Self {
        self.text_unit_ids = text_unit_ids;
        self
    }
}

/// Compact relationship list item.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct GraphRelationship {
    /// Stable relationship UUID.
    pub id: String,
    /// Human-readable id.
    pub short_id: Option<String>,
    /// Source entity title.
    pub source: String,
    /// Target entity title.
    pub target: String,
    /// Optional weight.
    pub weight: Option<f64>,
    /// Optional combined-degree rank.
    pub rank: Option<i64>,
}

/// Full relationship detail.
#[derive(Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct GraphRelationshipDetail {
    /// Stable relationship UUID.
    pub id: String,
    /// Human-readable id.
    pub short_id: Option<String>,
    /// Source entity title.
    pub source: String,
    /// Target entity title.
    pub target: String,
    /// Optional description.
    pub description: Option<String>,
    /// Optional weight.
    pub weight: Option<f64>,
    /// Optional combined-degree rank.
    pub rank: Option<i64>,
    /// Referenced text-unit ids.
    pub text_unit_ids: Vec<String>,
}

impl fmt::Debug for GraphRelationshipDetail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GraphRelationshipDetail { .. }")
    }
}

impl GraphRelationshipDetail {
    /// Create a minimal relationship detail between two entity titles.
    #[must_use]
    pub fn new(id: String, source: String, target: String) -> Self {
        Self {
            id,
            short_id: None,
            source,
            target,
            description: None,
            weight: None,
            rank: None,
            text_unit_ids: Vec::new(),
        }
    }

    /// Set the human-readable id.
    #[must_use]
    pub fn with_short_id(mut self, short_id: String) -> Self {
        self.short_id = Some(short_id);
        self
    }

    /// Set the relationship description.
    #[must_use]
    pub fn with_description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }

    /// Set the relationship weight.
    #[must_use]
    pub const fn with_weight(mut self, weight: f64) -> Self {
        self.weight = Some(weight);
        self
    }

    /// Set the combined-degree rank.
    #[must_use]
    pub const fn with_rank(mut self, rank: i64) -> Self {
        self.rank = Some(rank);
        self
    }

    /// Set referenced text-unit ids.
    #[must_use]
    pub fn with_text_unit_ids(mut self, text_unit_ids: Vec<String>) -> Self {
        self.text_unit_ids = text_unit_ids;
        self
    }
}

/// Query-visible, report-backed community.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct GraphCommunity {
    /// Stable community UUID.
    pub id: String,
    /// Decimal community id.
    pub short_id: String,
    /// Community title.
    pub title: String,
    /// Hierarchy level.
    pub level: i64,
    /// Parent community id, or `-1` for roots.
    pub parent: i64,
    /// Child community ids.
    pub children: Vec<i64>,
    /// Query-readable report summary, when one is present.
    pub report: Option<GraphCommunityReportSummary>,
}

impl GraphCommunity {
    /// Create a root-level community without children or an attached report summary.
    #[must_use]
    pub fn new(id: String, short_id: String, title: String) -> Self {
        Self {
            id,
            short_id,
            title,
            level: 0,
            parent: -1,
            children: Vec::new(),
            report: None,
        }
    }

    /// Set hierarchy fields without recomputing them.
    #[must_use]
    pub fn with_hierarchy(mut self, level: i64, parent: i64, children: Vec<i64>) -> Self {
        self.level = level;
        self.parent = parent;
        self.children = children;
        self
    }
}

/// Compact community report fields suitable for lists.
#[derive(Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct GraphCommunityReportSummary {
    /// Stable report UUID.
    pub id: String,
    /// Decimal community id.
    pub short_id: String,
    /// Community identifier.
    pub community_id: String,
    /// Report title.
    pub title: String,
    /// Report summary.
    pub summary: String,
    /// Optional rank.
    pub rank: Option<f64>,
}

impl fmt::Debug for GraphCommunityReportSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GraphCommunityReportSummary { .. }")
    }
}

/// Full community report, excluding embeddings.
#[derive(Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct GraphCommunityReportDetail {
    /// Stable report UUID.
    pub id: String,
    /// Decimal community id.
    pub short_id: String,
    /// Community identifier.
    pub community_id: String,
    /// Report title.
    pub title: String,
    /// Report summary.
    pub summary: String,
    /// Full report content.
    pub full_content: String,
    /// Optional rank.
    pub rank: Option<f64>,
}

impl fmt::Debug for GraphCommunityReportDetail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GraphCommunityReportDetail { .. }")
    }
}

impl GraphCommunityReportDetail {
    /// Create a report detail without rank.
    #[must_use]
    pub fn new(
        id: String,
        community_id: String,
        title: String,
        summary: String,
        full_content: String,
    ) -> Self {
        Self {
            id,
            short_id: community_id.clone(),
            community_id,
            title,
            summary,
            full_content,
            rank: None,
        }
    }

    /// Override the human-readable report id.
    #[must_use]
    pub fn with_short_id(mut self, short_id: String) -> Self {
        self.short_id = short_id;
        self
    }

    /// Set the report rank.
    #[must_use]
    pub const fn with_rank(mut self, rank: f64) -> Self {
        self.rank = Some(rank);
        self
    }
}

impl From<&Entity> for GraphEntity {
    fn from(entity: &Entity) -> Self {
        Self {
            id: entity.id.clone(),
            short_id: entity.short_id.clone(),
            title: entity.title.clone(),
            entity_type: entity.entity_type.clone(),
            rank: entity.rank,
            community_ids: entity.community_ids.clone(),
        }
    }
}

impl From<&Entity> for GraphEntityDetail {
    fn from(entity: &Entity) -> Self {
        Self {
            id: entity.id.clone(),
            short_id: entity.short_id.clone(),
            title: entity.title.clone(),
            entity_type: entity.entity_type.clone(),
            description: entity.description.clone(),
            rank: entity.rank,
            community_ids: entity.community_ids.clone(),
            text_unit_ids: entity.text_unit_ids.clone(),
        }
    }
}

impl From<&GraphEntityDetail> for GraphEntity {
    fn from(entity: &GraphEntityDetail) -> Self {
        Self {
            id: entity.id.clone(),
            short_id: entity.short_id.clone(),
            title: entity.title.clone(),
            entity_type: entity.entity_type.clone(),
            rank: entity.rank,
            community_ids: entity.community_ids.clone(),
        }
    }
}

impl From<&Relationship> for GraphRelationship {
    fn from(relationship: &Relationship) -> Self {
        Self {
            id: relationship.id.clone(),
            short_id: relationship.short_id.clone(),
            source: relationship.source.clone(),
            target: relationship.target.clone(),
            weight: relationship.weight,
            rank: relationship.rank,
        }
    }
}

impl From<&Relationship> for GraphRelationshipDetail {
    fn from(relationship: &Relationship) -> Self {
        Self {
            id: relationship.id.clone(),
            short_id: relationship.short_id.clone(),
            source: relationship.source.clone(),
            target: relationship.target.clone(),
            description: relationship.description.clone(),
            weight: relationship.weight,
            rank: relationship.rank,
            text_unit_ids: relationship.text_unit_ids.clone(),
        }
    }
}

impl From<&GraphRelationshipDetail> for GraphRelationship {
    fn from(relationship: &GraphRelationshipDetail) -> Self {
        Self {
            id: relationship.id.clone(),
            short_id: relationship.short_id.clone(),
            source: relationship.source.clone(),
            target: relationship.target.clone(),
            weight: relationship.weight,
            rank: relationship.rank,
        }
    }
}

impl From<&Community> for GraphCommunity {
    fn from(community: &Community) -> Self {
        Self {
            id: community.id.clone(),
            short_id: community.short_id.clone(),
            title: community.title.clone(),
            level: community.level,
            parent: community.parent,
            children: community.children.clone(),
            report: None,
        }
    }
}

impl GraphCommunity {
    pub(crate) fn with_report(
        community: &Self,
        report: Option<&GraphCommunityReportDetail>,
    ) -> Self {
        let mut value = community.clone();
        value.report = report.map(GraphCommunityReportSummary::from);
        value
    }
}

impl From<&CommunityReport> for GraphCommunityReportSummary {
    fn from(report: &CommunityReport) -> Self {
        Self {
            id: report.id.clone(),
            short_id: report.short_id.clone(),
            community_id: report.community_id.clone(),
            title: report.title.clone(),
            summary: report.summary.clone(),
            rank: report.rank,
        }
    }
}

impl From<&CommunityReport> for GraphCommunityReportDetail {
    fn from(report: &CommunityReport) -> Self {
        Self {
            id: report.id.clone(),
            short_id: report.short_id.clone(),
            community_id: report.community_id.clone(),
            title: report.title.clone(),
            summary: report.summary.clone(),
            full_content: report.full_content.clone(),
            rank: report.rank,
        }
    }
}

impl From<&GraphCommunityReportDetail> for GraphCommunityReportSummary {
    fn from(report: &GraphCommunityReportDetail) -> Self {
        Self {
            id: report.id.clone(),
            short_id: report.short_id.clone(),
            community_id: report.community_id.clone(),
            title: report.title.clone(),
            summary: report.summary.clone(),
            rank: report.rank,
        }
    }
}
