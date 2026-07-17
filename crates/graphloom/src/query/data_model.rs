//! Strongly typed Query-side data model.

/// Query entity adapted from the final entities table.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Entity {
    /// Stable entity UUID.
    pub id: String,
    /// Human-readable entity id.
    pub short_id: Option<String>,
    /// Entity title.
    pub title: String,
    /// Optional entity type.
    pub entity_type: Option<String>,
    /// Optional description.
    pub description: Option<String>,
    /// Community ids represented as decimal strings.
    pub community_ids: Vec<String>,
    /// Referenced text-unit ids.
    pub text_unit_ids: Vec<String>,
    /// Entity rank derived from degree.
    pub rank: Option<i64>,
}

/// Query relationship adapted from the final relationships table.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Relationship {
    /// Stable relationship UUID.
    pub id: String,
    /// Human-readable relationship id.
    pub short_id: Option<String>,
    /// Source entity title.
    pub source: String,
    /// Target entity title.
    pub target: String,
    /// Optional relationship description.
    pub description: Option<String>,
    /// Optional relationship weight.
    pub weight: Option<f64>,
    /// Relationship rank derived from combined degree.
    pub rank: Option<i64>,
    /// Referenced text-unit ids.
    pub text_unit_ids: Vec<String>,
}

/// Query community adapted from the final communities table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Community {
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
}

/// Query community report.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CommunityReport {
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
    /// Optional report rank.
    pub rank: Option<f64>,
    /// Optional hydrated report embedding.
    pub full_content_embedding: Option<Vec<f32>>,
}

/// Query text unit.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TextUnit {
    /// Stable text-unit UUID.
    pub id: String,
    /// Row-number id assigned after resetting the input `DataFrame` index.
    pub short_id: String,
    /// Text content.
    pub text: String,
    /// Referenced entity ids.
    pub entity_ids: Vec<String>,
    /// Referenced relationship ids.
    pub relationship_ids: Vec<String>,
    /// Referenced covariate ids.
    pub covariate_ids: Vec<String>,
    /// Optional token count.
    pub n_tokens: Option<i64>,
    /// Optional source document id.
    pub document_id: Option<String>,
}

/// Query covariate/claim.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Covariate {
    /// Stable covariate identifier interpreted as text.
    pub id: String,
    /// Human-readable covariate id.
    pub short_id: Option<String>,
    /// Claim subject.
    pub subject_id: String,
    /// Covariate type.
    pub covariate_type: String,
    /// Optional object id.
    pub object_id: Option<String>,
    /// Optional status.
    pub status: Option<String>,
    /// Optional start date.
    pub start_date: Option<String>,
    /// Optional end date.
    pub end_date: Option<String>,
    /// Optional description.
    pub description: Option<String>,
}
