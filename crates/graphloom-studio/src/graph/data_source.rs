use std::{collections::HashSet, fmt};

use async_trait::async_trait;
use thiserror::Error;

use super::{
    GraphCommunity, GraphCommunityReportDetail, GraphEntityDetail, GraphRelationshipDetail,
};

/// A typed, Query-visible graph snapshot.
///
/// Parquet implementations obtain these records from the same Core adapters
/// consumed by Query runtimes. Studio owns the records so custom datasource
/// implementations can construct snapshots without depending on Parquet or
/// non-exhaustive Core model constructors.
#[derive(Clone)]
#[non_exhaustive]
pub struct GraphDataSnapshot {
    /// Query-visible entities.
    pub entities: Vec<GraphEntityDetail>,
    /// Query-visible relationships.
    pub relationships: Vec<GraphRelationshipDetail>,
    /// Report-backed, Query-visible communities.
    pub communities: Vec<GraphCommunity>,
    /// Query-readable community reports without hydrated embeddings.
    pub community_reports: Vec<GraphCommunityReportDetail>,
}

impl fmt::Debug for GraphDataSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GraphDataSnapshot")
            .field("entities", &self.entities.len())
            .field("relationships", &self.relationships.len())
            .field("communities", &self.communities.len())
            .field("community_reports", &self.community_reports.len())
            .finish()
    }
}

impl GraphDataSnapshot {
    /// Create and validate a custom Query-visible graph snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`GraphDataSourceError::Unavailable`] when any required stable
    /// identity is duplicated.
    pub fn new(
        entities: Vec<GraphEntityDetail>,
        relationships: Vec<GraphRelationshipDetail>,
        communities: Vec<GraphCommunity>,
        community_reports: Vec<GraphCommunityReportDetail>,
    ) -> Result<Self, GraphDataSourceError> {
        let snapshot = Self {
            entities,
            relationships,
            communities,
            community_reports,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub(crate) fn validate(&self) -> Result<(), GraphDataSourceError> {
        unique(self.entities.iter().map(|entity| entity.id.as_str()))?;
        unique(
            self.relationships
                .iter()
                .map(|relationship| relationship.id.as_str()),
        )?;
        unique(
            self.communities
                .iter()
                .map(|community| community.id.as_str()),
        )?;
        unique(
            self.communities
                .iter()
                .map(|community| community.short_id.as_str()),
        )?;
        unique(
            self.community_reports
                .iter()
                .map(|report| report.id.as_str()),
        )?;
        unique(
            self.community_reports
                .iter()
                .map(|report| report.community_id.as_str()),
        )?;
        Ok(())
    }
}

fn unique<'a>(values: impl Iterator<Item = &'a str>) -> Result<(), GraphDataSourceError> {
    let mut seen = HashSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(GraphDataSourceError::Unavailable);
        }
    }
    Ok(())
}

/// Safe, low-information graph loading failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum GraphDataSourceError {
    /// The graph output is absent, unreadable, incompatible, or internally inconsistent.
    #[error("graph data is unavailable")]
    Unavailable,
}

/// Backend-independent source of one current Query-visible graph snapshot.
#[async_trait]
pub trait GraphDataSource: Send + Sync + fmt::Debug {
    /// Load the current graph snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`GraphDataSourceError::Unavailable`] when required graph data
    /// cannot be loaded or violates snapshot identity invariants.
    async fn load_snapshot(&self) -> Result<GraphDataSnapshot, GraphDataSourceError>;
}
