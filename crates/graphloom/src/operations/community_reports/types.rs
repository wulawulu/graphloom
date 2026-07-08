//! Domain rows used by community report operations.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EntityContextRow {
    pub(crate) id: String,
    pub(crate) human_readable_id: i64,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) degree: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelationshipContextRow {
    pub(crate) id: String,
    pub(crate) human_readable_id: i64,
    pub(crate) source: String,
    pub(crate) target: String,
    pub(crate) description: String,
    pub(crate) combined_degree: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaimContextRow {
    pub(crate) human_readable_id: i64,
    pub(crate) subject_id: String,
    pub(crate) claim_type: String,
    pub(crate) status: String,
    pub(crate) description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommunityInputRow {
    pub(crate) community: i64,
    pub(crate) level: i64,
    pub(crate) parent: i64,
    pub(crate) children: Vec<i64>,
    pub(crate) entity_ids: Vec<String>,
    pub(crate) period: String,
    pub(crate) size: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExplodedEntityRow {
    pub(crate) community: i64,
    pub(crate) level: i64,
    pub(crate) entity: EntityContextRow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommunityLocalContext {
    pub(crate) community: i64,
    pub(crate) context: String,
    pub(crate) token_count: usize,
    pub(crate) exceeds_limit: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CommunityReportFindingRow {
    pub(crate) summary: String,
    pub(crate) explanation: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CommunityReportRow {
    pub(crate) id: String,
    pub(crate) human_readable_id: i64,
    pub(crate) community: i64,
    pub(crate) level: i64,
    pub(crate) parent: i64,
    pub(crate) children: Vec<i64>,
    pub(crate) title: String,
    pub(crate) summary: String,
    pub(crate) full_content: String,
    pub(crate) rank: f64,
    pub(crate) rating_explanation: String,
    pub(crate) findings: Vec<CommunityReportFindingRow>,
    pub(crate) full_content_json: String,
    pub(crate) period: String,
    pub(crate) size: i64,
}
