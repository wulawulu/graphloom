//! Domain rows used by community report operations.

use std::collections::BTreeSet;

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
    pub(crate) full_records: ContextRecords,
    pub(crate) records: ContextRecords,
    pub(crate) context: String,
    pub(crate) token_count: usize,
    pub(crate) full_token_count: usize,
    pub(crate) was_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReportContextRow {
    pub(crate) community: i64,
    pub(crate) full_content: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ContextRecords {
    pub(crate) reports: Vec<ReportContextRow>,
    pub(crate) report_ids: BTreeSet<i64>,
    pub(crate) entities: Vec<EntityContextRow>,
    pub(crate) entity_ids: BTreeSet<i64>,
    pub(crate) claims: Vec<ClaimContextRow>,
    pub(crate) claim_ids: BTreeSet<i64>,
    pub(crate) relationships: Vec<RelationshipContextRow>,
    pub(crate) relationship_ids: BTreeSet<i64>,
}

impl ContextRecords {
    pub(crate) fn add_report(&mut self, row: ReportContextRow) {
        if self.report_ids.insert(row.community) {
            self.reports.push(row);
        }
    }

    pub(crate) fn add_entity(&mut self, row: EntityContextRow) {
        if self.entity_ids.insert(row.human_readable_id) {
            self.entities.push(row);
        }
    }

    pub(crate) fn add_claim(&mut self, row: ClaimContextRow) {
        if self.claim_ids.insert(row.human_readable_id) {
            self.claims.push(row);
        }
    }

    pub(crate) fn add_relationship(&mut self, row: RelationshipContextRow) {
        if self.relationship_ids.insert(row.human_readable_id) {
            self.relationships.push(row);
        }
    }

    pub(crate) fn merge_details(&mut self, other: &Self) {
        for entity in &other.entities {
            self.add_entity(entity.clone());
        }
        for claim in &other.claims {
            self.add_claim(claim.clone());
        }
        for relationship in &other.relationships {
            self.add_relationship(relationship.clone());
        }
    }
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
