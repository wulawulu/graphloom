//! Method-aware read-only Query table loader.

use std::sync::Arc;

use graphloom_storage::TableProvider;
use polars_core::prelude::DataFrame;

use super::{
    Community, CommunityReport, Covariate, Entity, QueryError, QueryTableErrorDetails,
    Relationship, Result, SearchMethod, TextUnit,
    indexer_adapters::{
        read_indexer_communities, read_indexer_covariates, read_indexer_entities,
        read_indexer_relationships, read_indexer_reports, read_indexer_text_units,
    },
};

/// Method-aware read-only adapter over persisted Query tables.
#[derive(Debug)]
#[non_exhaustive]
pub struct QueryDataLoader {
    table_provider: Arc<dyn TableProvider>,
}

impl QueryDataLoader {
    /// Create a read-only loader over an existing table provider.
    #[must_use]
    pub fn new(table_provider: Arc<dyn TableProvider>) -> Self {
        Self { table_provider }
    }

    /// Load only the text-unit table required by Basic Search.
    ///
    /// # Errors
    ///
    /// Returns a typed Query error when the required table is absent or invalid.
    pub async fn load_basic(&self) -> Result<BasicQueryData> {
        let method = SearchMethod::Basic;
        let text_units = self.required("text_units", method).await?;
        Ok(BasicQueryData {
            text_units: read_indexer_text_units(&text_units, method)?,
        })
    }

    /// Load and adapt the tables required by Global Search.
    ///
    /// # Errors
    ///
    /// Returns a typed Query error when a required table is absent or invalid.
    pub async fn load_global(
        &self,
        community_level: i64,
        dynamic: bool,
    ) -> Result<GlobalQueryData> {
        let method = SearchMethod::Global;
        let (entities, communities, reports) = tokio::try_join!(
            self.required("entities", method),
            self.required("communities", method),
            self.required("community_reports", method),
        )?;
        Ok(GlobalQueryData {
            entities: read_indexer_entities(&entities, &communities, community_level, method)?,
            reports: read_indexer_reports(
                &reports,
                &communities,
                community_level,
                dynamic,
                method,
            )?,
            communities: read_indexer_communities(&communities, &reports, method)?,
        })
    }

    /// Load and adapt the tables required by Local Search.
    ///
    /// Missing covariates are represented by an empty collection.
    ///
    /// # Errors
    ///
    /// Returns a typed Query error when another required table is absent or invalid.
    pub async fn load_local(&self, community_level: i64) -> Result<LocalQueryData> {
        let method = SearchMethod::Local;
        let (entities, communities, reports, text_units, relationships, covariates) = tokio::try_join!(
            self.required("entities", method),
            self.required("communities", method),
            self.required("community_reports", method),
            self.required("text_units", method),
            self.required("relationships", method),
            self.optional("covariates", method),
        )?;
        Ok(LocalQueryData {
            entities: read_indexer_entities(&entities, &communities, community_level, method)?,
            reports: read_indexer_reports(&reports, &communities, community_level, false, method)?,
            communities: read_indexer_communities(&communities, &reports, method)?,
            text_units: read_indexer_text_units(&text_units, method)?,
            relationships: read_indexer_relationships(&relationships, method)?,
            covariates: covariates.as_ref().map_or_else(
                || Ok(Vec::new()),
                |value| read_indexer_covariates(value, method),
            )?,
        })
    }

    /// Load and adapt the tables required by DRIFT Search.
    ///
    /// # Errors
    ///
    /// Returns a typed Query error when a required table is absent or invalid.
    pub async fn load_drift(&self, community_level: i64) -> Result<DriftQueryData> {
        let method = SearchMethod::Drift;
        let (entities, communities, reports, text_units, relationships) = tokio::try_join!(
            self.required("entities", method),
            self.required("communities", method),
            self.required("community_reports", method),
            self.required("text_units", method),
            self.required("relationships", method),
        )?;
        Ok(DriftQueryData {
            entities: read_indexer_entities(&entities, &communities, community_level, method)?,
            reports: read_indexer_reports(&reports, &communities, community_level, false, method)?,
            communities: read_indexer_communities(&communities, &reports, method)?,
            text_units: read_indexer_text_units(&text_units, method)?,
            relationships: read_indexer_relationships(&relationships, method)?,
        })
    }

    async fn required(&self, table: &'static str, method: SearchMethod) -> Result<DataFrame> {
        self.table_provider
            .read_dataframe(table)
            .await
            .map_err(|source| match source {
                graphloom_storage::StorageError::MissingTable { .. } => {
                    QueryError::MissingQueryTable {
                        method,
                        operation: "load query data",
                        table,
                    }
                }
                source => table_io_error(method, table, "read query table", source),
            })
    }

    async fn optional(
        &self,
        table: &'static str,
        method: SearchMethod,
    ) -> Result<Option<DataFrame>> {
        self.table_provider
            .read_optional_dataframe(table)
            .await
            .map_err(|source| table_io_error(method, table, "read optional query table", source))
    }
}

fn table_io_error(
    method: SearchMethod,
    table: &'static str,
    operation: &'static str,
    source: graphloom_storage::StorageError,
) -> QueryError {
    let message = source.to_string();
    QueryError::InvalidQueryTable {
        method,
        operation,
        details: Box::new(QueryTableErrorDetails {
            table,
            column: "<table>".to_owned(),
            expected: "readable Parquet table",
            actual: "storage error".to_owned(),
            row: String::new(),
            message,
            source: Some(Box::new(source)),
        }),
    }
}

/// Tables adapted for Basic Search.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BasicQueryData {
    /// Adapted text units in original table order.
    pub text_units: Vec<TextUnit>,
}

/// Tables adapted for Global Search.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct GlobalQueryData {
    /// Query entities.
    pub entities: Vec<Entity>,
    /// Report-backed communities.
    pub communities: Vec<Community>,
    /// Community reports after level and roll-up selection.
    pub reports: Vec<CommunityReport>,
}

/// Tables adapted for Local Search.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LocalQueryData {
    /// Query entities.
    pub entities: Vec<Entity>,
    /// Report-backed communities.
    pub communities: Vec<Community>,
    /// Rolled-up community reports.
    pub reports: Vec<CommunityReport>,
    /// Query text units.
    pub text_units: Vec<TextUnit>,
    /// Query relationships.
    pub relationships: Vec<Relationship>,
    /// Optional covariates, empty when the table is absent.
    pub covariates: Vec<Covariate>,
}

/// Tables adapted for DRIFT Search.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DriftQueryData {
    /// Query entities.
    pub entities: Vec<Entity>,
    /// Report-backed communities.
    pub communities: Vec<Community>,
    /// Rolled-up community reports.
    pub reports: Vec<CommunityReport>,
    /// Query text units.
    pub text_units: Vec<TextUnit>,
    /// Query relationships.
    pub relationships: Vec<Relationship>,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use graphloom_storage::{MemoryTableProvider, TableProvider};
    use polars_core::prelude::{DataFrame, df};

    use super::*;
    use crate::dataframe::{i64_list_column, list_column};

    fn text_units() -> DataFrame {
        let mut dataframe = df!(
            "id" => ["tu-1"],
            "text" => ["Only Basic needs this table"],
            "n_tokens" => [6_u32],
            "document_id" => ["doc-1"],
        )
        .expect("text units");
        for name in ["entity_ids", "relationship_ids", "covariate_ids"] {
            dataframe
                .with_column(list_column(name, &[Vec::<String>::new()]))
                .expect("list column");
        }
        dataframe
    }

    fn entities() -> DataFrame {
        let mut dataframe = df!(
            "id" => ["e-1"],
            "human_readable_id" => [1_u32],
            "title" => ["Alice"],
            "type" => ["PERSON"],
            "description" => ["A person"],
            "degree" => [1_i32],
        )
        .expect("entities");
        dataframe
            .with_column(list_column("text_unit_ids", &[vec!["tu-1".to_owned()]]))
            .expect("text units");
        dataframe
    }

    fn communities() -> DataFrame {
        let mut dataframe = df!(
            "id" => ["co-1"],
            "community" => [1_u64],
            "level" => [0_u32],
            "title" => ["Community 1"],
            "parent" => [-1_i64],
        )
        .expect("communities");
        dataframe
            .with_column(list_column("entity_ids", &[vec!["e-1".to_owned()]]))
            .expect("entity ids");
        dataframe
            .with_column(i64_list_column("children", &[Vec::new()]))
            .expect("children");
        dataframe
    }

    fn reports() -> DataFrame {
        df!(
            "id" => ["rp-1"],
            "community" => [1_i32],
            "level" => [0_i32],
            "title" => ["Report 1"],
            "summary" => ["Summary"],
            "full_content" => ["Content"],
            "rank" => [1.0_f64],
        )
        .expect("reports")
    }

    fn relationships() -> DataFrame {
        let mut dataframe = df!(
            "id" => ["r-1"],
            "human_readable_id" => [1_i64],
            "source" => ["Alice"],
            "target" => ["Alice"],
            "description" => ["self"],
            "weight" => [1.0_f64],
            "combined_degree" => [2_u32],
        )
        .expect("relationships");
        dataframe
            .with_column(list_column("text_unit_ids", &[vec!["tu-1".to_owned()]]))
            .expect("text units");
        dataframe
    }

    async fn write(provider: &MemoryTableProvider, name: &str, dataframe: DataFrame) {
        provider
            .write_dataframe(name, dataframe)
            .await
            .expect("write fixture");
    }

    #[tokio::test]
    async fn test_should_load_only_basic_requirements_without_mutating_tables() {
        let provider = MemoryTableProvider::new();
        write(&provider, "text_units", text_units()).await;
        let before = provider.list().await.expect("list before");
        let loader = QueryDataLoader::new(Arc::new(provider.clone()));

        let data = loader.load_basic().await.expect("Basic data");

        assert_eq!(data.text_units.len(), 1);
        assert_eq!(provider.list().await.expect("list after"), before);
        assert!(!provider.has("entities").await.expect("entities check"));
        assert!(
            !provider
                .has("community_reports")
                .await
                .expect("reports check")
        );
    }

    #[tokio::test]
    async fn test_should_treat_covariates_as_optional_only_for_local_search() {
        let provider = MemoryTableProvider::new();
        for (name, dataframe) in [
            ("entities", entities()),
            ("communities", communities()),
            ("community_reports", reports()),
            ("text_units", text_units()),
            ("relationships", relationships()),
        ] {
            write(&provider, name, dataframe).await;
        }
        let loader = QueryDataLoader::new(Arc::new(provider));

        let data = loader.load_local(0).await.expect("Local data");

        assert!(data.covariates.is_empty());
        assert_eq!(data.entities.len(), 1);
        assert_eq!(data.relationships.len(), 1);
    }

    #[tokio::test]
    async fn test_should_report_only_the_active_method_missing_table() {
        let loader = QueryDataLoader::new(Arc::new(MemoryTableProvider::new()));

        let error = loader
            .load_basic()
            .await
            .expect_err("text_units should be required");

        assert!(matches!(
            error,
            QueryError::MissingQueryTable {
                method: SearchMethod::Basic,
                table: "text_units",
                ..
            }
        ));
    }
}
