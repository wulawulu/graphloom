//! Community report operations and table codecs.

mod context;
mod extraction;
mod tables;
mod types;

pub(crate) use context::build_local_contexts;
pub(crate) use extraction::{
    CommunityReportCallbacks, CommunityReportExtractionConfig, CommunityReportOperationInput,
    create_community_reports,
};
pub(crate) use tables::{
    community_report_value, community_reports_dataframe, read_claim_context_rows,
    read_community_input_rows, read_entity_context_rows, read_relationship_context_rows,
};
pub(crate) use types::{
    ClaimContextRow, CommunityInputRow, CommunityLocalContext, CommunityReportFindingRow,
    CommunityReportRow, ContextRecords, EntityContextRow, ExplodedEntityRow,
    RelationshipContextRow, ReportContextRow,
};
