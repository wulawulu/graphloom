//! Query-visible graph snapshots and stable HTTP data-transfer objects.
//!
//! The datasource contract is object-safe because Studio services accept custom
//! backends through `Arc<dyn GraphDataSource>`. Per the workspace async-trait
//! policy, this is the reason the trait uses `async-trait`.

mod data_source;
mod dto;
mod parquet;
mod projection;
mod resolver;

pub use data_source::{GraphDataSnapshot, GraphDataSource, GraphDataSourceError};
pub use dto::{
    GraphCommunity, GraphCommunityRef, GraphCommunityReportDetail, GraphCommunityReportSummary,
    GraphEntity, GraphEntityDetail, GraphEntityRef, GraphProjection, GraphProjectionEntity,
    GraphProjectionRelationship, GraphRelationship, GraphRelationshipDetail, GraphSummary,
    GraphTextUnitDetail, GraphTextUnitRef, GraphTextUnitResolveResponse,
};
pub use parquet::ParquetGraphDataSource;
pub(crate) use projection::{GraphProjectionError, overview, subgraph};
pub(crate) use resolver::{GraphReferenceIndex, GraphTextUnitIndex};
