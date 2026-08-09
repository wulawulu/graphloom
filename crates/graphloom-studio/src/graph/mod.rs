//! Query-visible graph snapshots and stable HTTP data-transfer objects.
//!
//! The datasource contract is object-safe because Studio services accept custom
//! backends through `Arc<dyn GraphDataSource>`. Per the workspace async-trait
//! policy, this is the reason the trait uses `async-trait`.

mod data_source;
mod dto;
mod parquet;

pub use data_source::{GraphDataSnapshot, GraphDataSource, GraphDataSourceError};
pub use dto::{
    GraphCommunity, GraphCommunityReportDetail, GraphCommunityReportSummary, GraphEntity,
    GraphEntityDetail, GraphRelationship, GraphRelationshipDetail, GraphSummary,
};
pub use parquet::ParquetGraphDataSource;
