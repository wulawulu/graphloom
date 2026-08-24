use std::{fmt, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use graphloom::{
    query::{
        SearchMethod, read_indexer_communities, read_indexer_entities, read_indexer_relationships,
        read_indexer_reports, read_indexer_text_units,
    },
    storage::{FileStorage, ParquetTableProvider, TableProvider},
};

use super::{
    GraphCommunity, GraphCommunityReportDetail, GraphDataSnapshot, GraphDataSource,
    GraphDataSourceError, GraphEntityDetail, GraphRelationshipDetail, GraphTextUnitDetail,
};

const ENTITIES_TABLE: &str = "entities";
const RELATIONSHIPS_TABLE: &str = "relationships";
const COMMUNITIES_TABLE: &str = "communities";
const COMMUNITY_REPORTS_TABLE: &str = "community_reports";
const TEXT_UNITS_TABLE: &str = "text_units";

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

    async fn load_text_units(
        &self,
        ids: &[String],
    ) -> Result<Vec<GraphTextUnitDetail>, GraphDataSourceError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let storage = FileStorage::existing(&self.table_root)
            .map_err(|_| GraphDataSourceError::Unavailable)?;
        let provider = ParquetTableProvider::from_storage(Arc::new(storage));
        load_text_units_from_provider(&provider, ids).await
    }
}

async fn load_text_units_from_provider(
    provider: &dyn TableProvider,
    ids: &[String],
) -> Result<Vec<GraphTextUnitDetail>, GraphDataSourceError> {
    let dataframe = provider
        .read_dataframe(TEXT_UNITS_TABLE)
        .await
        .map_err(|_| GraphDataSourceError::Unavailable)?;
    let units = read_indexer_text_units(&dataframe, SearchMethod::Local)
        .map_err(|_| GraphDataSourceError::Unavailable)?;
    let mut unique_ids = std::collections::HashSet::with_capacity(units.len());
    if units
        .iter()
        .any(|unit| !unique_ids.insert(unit.id.as_str()))
    {
        return Err(GraphDataSourceError::Unavailable);
    }
    let requested = ids
        .iter()
        .map(String::as_str)
        .collect::<std::collections::HashSet<_>>();
    Ok(units
        .iter()
        .filter(|unit| requested.contains(unit.id.as_str()))
        .map(GraphTextUnitDetail::from)
        .collect())
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
    use polars_core::df;
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

    async fn write_text_units(target: &Path) -> TestResult {
        let provider = ParquetTableProvider::new(target)?;
        let dataframe = df!(
            "id" => ["text-1", "text-2"],
            "text" => ["Exact first source", "第二条原文"],
            "n_tokens" => [Some(18_i64), None],
            "document_id" => [Some("document-1"), None],
        )?;
        provider
            .write_dataframe(TEXT_UNITS_TABLE, dataframe)
            .await?;
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
        assert_eq!(source.load_text_units(&[]).await, Ok(Vec::new()));
        assert!(matches!(
            source.load_snapshot().await,
            Err(GraphDataSourceError::Unavailable)
        ));
        assert!(!format!("{source:?}").contains("GRAPH_PATH_SECRET_SENTINEL"));
    }

    #[tokio::test]
    async fn test_should_load_requested_text_units_with_core_adapter_semantics() -> TestResult {
        let tempdir = TempDir::new()?;
        write_text_units(tempdir.path()).await?;
        let source = ParquetGraphDataSource::new(tempdir.path().to_path_buf());

        let units = source
            .load_text_units(&[
                "text-2".to_owned(),
                "missing".to_owned(),
                "text-1".to_owned(),
            ])
            .await?;

        assert_eq!(
            units
                .iter()
                .map(|unit| unit.id.as_str())
                .collect::<Vec<_>>(),
            ["text-1", "text-2"]
        );
        assert_eq!(units[0].short_id, "0");
        assert_eq!(units[0].text, "Exact first source");
        assert_eq!(units[0].n_tokens, Some(18));
        assert_eq!(units[0].document_id.as_deref(), Some("document-1"));
        assert_eq!(units[1].short_id, "1");
        assert_eq!(units[1].text, "第二条原文");
        assert_eq!(units[1].n_tokens, None);
        assert_eq!(units[1].document_id, None);
        Ok(())
    }

    #[tokio::test]
    async fn test_should_reject_duplicate_text_unit_stable_ids() -> TestResult {
        let tempdir = TempDir::new()?;
        let provider = ParquetTableProvider::new(tempdir.path())?;
        provider
            .write_dataframe(
                TEXT_UNITS_TABLE,
                df!("id" => ["duplicate", "duplicate"], "text" => ["A", "B"] )?,
            )
            .await?;
        let source = ParquetGraphDataSource::new(tempdir.path().to_path_buf());

        assert!(matches!(
            source.load_text_units(&["duplicate".to_owned()]).await,
            Err(GraphDataSourceError::Unavailable)
        ));
        Ok(())
    }
}
