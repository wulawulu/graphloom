use std::{fmt, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use graphloom::{
    query::{
        SearchMethod, read_indexer_communities, read_indexer_entities, read_indexer_relationships,
        read_indexer_reports,
    },
    storage::{FileStorage, ParquetTableProvider, TableProvider},
};

use super::{
    GraphCommunity, GraphCommunityReportDetail, GraphDataSnapshot, GraphDataSource,
    GraphDataSourceError, GraphEntityDetail, GraphRelationshipDetail,
};

const ENTITIES_TABLE: &str = "entities";
const RELATIONSHIPS_TABLE: &str = "relationships";
const COMMUNITIES_TABLE: &str = "communities";
const COMMUNITY_REPORTS_TABLE: &str = "community_reports";

/// Query-visible graph datasource backed by `GraphLoom` output Parquet tables.
#[derive(Clone)]
pub struct ParquetGraphDataSource {
    table_root: PathBuf,
}

impl fmt::Debug for ParquetGraphDataSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ParquetGraphDataSource { .. }")
    }
}

impl ParquetGraphDataSource {
    /// Create a lazy datasource rooted at a host-configured table directory.
    ///
    /// Construction performs no filesystem access. Missing or invalid output is
    /// reported by [`GraphDataSource::load_snapshot`].
    #[must_use]
    pub fn new(table_root: PathBuf) -> Self {
        Self { table_root }
    }
}

#[async_trait]
impl GraphDataSource for ParquetGraphDataSource {
    async fn load_snapshot(&self) -> Result<GraphDataSnapshot, GraphDataSourceError> {
        let storage = FileStorage::existing(&self.table_root)
            .map_err(|_| GraphDataSourceError::Unavailable)?;
        let provider = ParquetTableProvider::from_storage(Arc::new(storage));
        load_from_provider(&provider).await
    }
}

async fn load_from_provider(
    provider: &dyn TableProvider,
) -> Result<GraphDataSnapshot, GraphDataSourceError> {
    let (entities, relationships, communities, reports) = tokio::try_join!(
        provider.read_dataframe(ENTITIES_TABLE),
        provider.read_dataframe(RELATIONSHIPS_TABLE),
        provider.read_dataframe(COMMUNITIES_TABLE),
        provider.read_dataframe(COMMUNITY_REPORTS_TABLE),
    )
    .map_err(|_| GraphDataSourceError::Unavailable)?;

    GraphDataSnapshot::new(
        read_indexer_entities(&entities, &communities, i64::MAX, SearchMethod::Local)
            .map_err(|_| GraphDataSourceError::Unavailable)?
            .iter()
            .map(GraphEntityDetail::from)
            .collect(),
        read_indexer_relationships(&relationships, SearchMethod::Local)
            .map_err(|_| GraphDataSourceError::Unavailable)?
            .iter()
            .map(GraphRelationshipDetail::from)
            .collect(),
        read_indexer_communities(&communities, &reports, SearchMethod::Local)
            .map_err(|_| GraphDataSourceError::Unavailable)?
            .iter()
            .map(GraphCommunity::from)
            .collect(),
        // Dynamic mode intentionally bypasses GraphRAG's non-dynamic,
        // title-based report roll-up. MAX preserves every readable level.
        read_indexer_reports(&reports, &communities, i64::MAX, true, SearchMethod::Local)
            .map_err(|_| GraphDataSourceError::Unavailable)?
            .iter()
            .map(GraphCommunityReportDetail::from)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use std::{error::Error, path::Path};

    use graphloom::query::{
        SearchMethod, read_indexer_communities, read_indexer_entities, read_indexer_relationships,
        read_indexer_reports,
    };
    use tempfile::TempDir;

    use super::*;

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/graph")
    }

    async fn copy_fixture_with_parquet_provider(target: &Path) -> TestResult {
        let source =
            ParquetTableProvider::from_storage(Arc::new(FileStorage::existing(fixture_root())?));
        let target = ParquetTableProvider::new(target)?;
        for table in [
            ENTITIES_TABLE,
            RELATIONSHIPS_TABLE,
            COMMUNITIES_TABLE,
            COMMUNITY_REPORTS_TABLE,
        ] {
            let dataframe = source.read_dataframe(table).await?;
            target.write_dataframe(table, dataframe).await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_should_load_real_parquet_with_exact_core_adapter_parity() -> TestResult {
        let tempdir = TempDir::new()?;
        copy_fixture_with_parquet_provider(tempdir.path()).await?;
        let source = ParquetGraphDataSource::new(tempdir.path().to_path_buf());
        let snapshot = source.load_snapshot().await?;

        let provider =
            ParquetTableProvider::from_storage(Arc::new(FileStorage::existing(tempdir.path())?));
        let entities = provider.read_dataframe(ENTITIES_TABLE).await?;
        let relationships = provider.read_dataframe(RELATIONSHIPS_TABLE).await?;
        let communities = provider.read_dataframe(COMMUNITIES_TABLE).await?;
        let reports = provider.read_dataframe(COMMUNITY_REPORTS_TABLE).await?;
        assert_eq!(
            snapshot.entities,
            read_indexer_entities(&entities, &communities, i64::MAX, SearchMethod::Local,)?
                .iter()
                .map(GraphEntityDetail::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            snapshot.relationships,
            read_indexer_relationships(&relationships, SearchMethod::Local)?
                .iter()
                .map(GraphRelationshipDetail::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            snapshot.communities,
            read_indexer_communities(&communities, &reports, SearchMethod::Local)?
                .iter()
                .map(GraphCommunity::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            snapshot.community_reports,
            read_indexer_reports(&reports, &communities, i64::MAX, true, SearchMethod::Local,)?
                .iter()
                .map(GraphCommunityReportDetail::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(snapshot.entities.len(), 3);
        assert!(
            snapshot
                .entities
                .iter()
                .any(|entity| entity.degree.is_some())
        );
        assert!(
            snapshot
                .entities
                .iter()
                .all(|entity| entity.degree == entity.rank)
        );
        assert_eq!(snapshot.relationships.len(), 3);
        assert_eq!(snapshot.communities.len(), 2);
        assert_eq!(snapshot.community_reports.len(), 2);
        assert!(
            snapshot
                .community_reports
                .iter()
                .all(|report| report.title == "Shared")
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_should_construct_lazily_and_report_missing_output_safely() {
        let root = PathBuf::from("GRAPH_PATH_SECRET_SENTINEL/missing");
        let source = ParquetGraphDataSource::new(root);

        assert_eq!(format!("{source:?}"), "ParquetGraphDataSource { .. }");
        assert!(matches!(
            source.load_snapshot().await,
            Err(GraphDataSourceError::Unavailable)
        ));
        assert!(!format!("{source:?}").contains("GRAPH_PATH_SECRET_SENTINEL"));
    }
}
