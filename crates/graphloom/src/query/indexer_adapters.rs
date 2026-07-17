//! `GraphRAG` 3.1 index-table to Query-model adapters.

use std::collections::{BTreeMap, BTreeSet};

use graphloom_vectors::{VectorError, VectorIndexSchema, VectorStore};
use polars_core::prelude::{AnyValue, Column, DataFrame};

use super::{
    Community, CommunityReport, Covariate, Entity, QueryError, QueryTableErrorDetails,
    Relationship, Result, SearchMethod, TextUnit,
};

/// Adapt index text units while assigning reset-row-number short identifiers.
///
/// # Errors
///
/// Returns a typed table error for missing or incompatible columns.
pub fn read_indexer_text_units(
    dataframe: &DataFrame,
    method: SearchMethod,
) -> Result<Vec<TextUnit>> {
    let mut rows = Vec::with_capacity(dataframe.height());
    for row in 0..dataframe.height() {
        rows.push(TextUnit {
            id: required_string(dataframe, row, "id", method, "text_units")?,
            short_id: row.to_string(),
            text: required_string(dataframe, row, "text", method, "text_units")?,
            entity_ids: string_list(dataframe, row, "entity_ids", method, "text_units")?,
            relationship_ids: string_list(
                dataframe,
                row,
                "relationship_ids",
                method,
                "text_units",
            )?,
            covariate_ids: string_list(dataframe, row, "covariate_ids", method, "text_units")?,
            n_tokens: optional_i64(dataframe, row, "n_tokens", method, "text_units")?,
            document_id: optional_string(dataframe, row, "document_id", method, "text_units")?,
        });
    }
    Ok(rows)
}

/// Adapt index relationships to Query relationships.
///
/// # Errors
///
/// Returns a typed table error for missing or incompatible columns.
pub fn read_indexer_relationships(
    dataframe: &DataFrame,
    method: SearchMethod,
) -> Result<Vec<Relationship>> {
    let mut rows = Vec::with_capacity(dataframe.height());
    for row in 0..dataframe.height() {
        rows.push(Relationship {
            id: required_string(dataframe, row, "id", method, "relationships")?,
            short_id: optional_stringish(
                dataframe,
                row,
                "human_readable_id",
                method,
                "relationships",
            )?,
            source: required_string(dataframe, row, "source", method, "relationships")?,
            target: required_string(dataframe, row, "target", method, "relationships")?,
            description: optional_string(dataframe, row, "description", method, "relationships")?,
            weight: optional_f64(dataframe, row, "weight", method, "relationships")?,
            rank: optional_i64(dataframe, row, "combined_degree", method, "relationships")?,
            text_unit_ids: string_list(dataframe, row, "text_unit_ids", method, "relationships")?,
        });
    }
    Ok(rows)
}

/// Adapt index covariates, accepting string and integral identifiers.
///
/// # Errors
///
/// Returns a typed table error for missing or incompatible columns.
pub fn read_indexer_covariates(
    dataframe: &DataFrame,
    method: SearchMethod,
) -> Result<Vec<Covariate>> {
    let mut rows = Vec::with_capacity(dataframe.height());
    for row in 0..dataframe.height() {
        rows.push(Covariate {
            id: required_stringish(dataframe, row, "id", method, "covariates")?,
            short_id: optional_stringish(
                dataframe,
                row,
                "human_readable_id",
                method,
                "covariates",
            )?,
            subject_id: required_string(dataframe, row, "subject_id", method, "covariates")?,
            covariate_type: required_string(dataframe, row, "type", method, "covariates")?,
            object_id: optional_string(dataframe, row, "object_id", method, "covariates")?,
            status: optional_string(dataframe, row, "status", method, "covariates")?,
            start_date: optional_string(dataframe, row, "start_date", method, "covariates")?,
            end_date: optional_string(dataframe, row, "end_date", method, "covariates")?,
            description: optional_string(dataframe, row, "description", method, "covariates")?,
        });
    }
    Ok(rows)
}

/// Adapt entities and aggregate their level-filtered community memberships.
///
/// # Errors
///
/// Returns a typed table error for missing or incompatible columns.
pub fn read_indexer_entities(
    entities: &DataFrame,
    communities: &DataFrame,
    community_level: i64,
    method: SearchMethod,
) -> Result<Vec<Entity>> {
    let mut membership = BTreeMap::<String, BTreeSet<i64>>::new();
    for row in community_memberships(communities, method)? {
        if row.level <= community_level {
            for entity_id in row.entity_ids {
                membership
                    .entry(entity_id)
                    .or_default()
                    .insert(row.community);
            }
        }
    }
    let mut rows = Vec::with_capacity(entities.height());
    let mut seen_ids = BTreeSet::new();
    for row in 0..entities.height() {
        let id = required_string(entities, row, "id", method, "entities")?;
        if !seen_ids.insert(id.clone()) {
            continue;
        }
        let community_ids = membership.get(&id).map_or_else(
            || vec!["-1".to_owned()],
            |values| values.iter().map(ToString::to_string).collect(),
        );
        rows.push(Entity {
            id,
            short_id: optional_stringish(entities, row, "human_readable_id", method, "entities")?,
            title: required_string(entities, row, "title", method, "entities")?,
            entity_type: optional_string(entities, row, "type", method, "entities")?,
            description: optional_string(entities, row, "description", method, "entities")?,
            community_ids,
            text_unit_ids: string_list(entities, row, "text_unit_ids", method, "entities")?,
            rank: optional_i64(entities, row, "degree", method, "entities")?,
        });
    }
    Ok(rows)
}

/// Adapt community reports with `GraphRAG` dynamic or roll-up selection.
///
/// # Errors
///
/// Returns a typed table error for missing or incompatible columns.
pub fn read_indexer_reports(
    reports: &DataFrame,
    communities: &DataFrame,
    community_level: i64,
    dynamic: bool,
    method: SearchMethod,
) -> Result<Vec<CommunityReport>> {
    let memberships = community_memberships(communities, method)?;
    let allowed = if dynamic {
        memberships
            .iter()
            .filter(|row| row.level <= community_level)
            .map(|row| row.community)
            .collect::<BTreeSet<_>>()
    } else {
        let mut entity_max = BTreeMap::<String, i64>::new();
        for row in memberships
            .iter()
            .filter(|row| row.level <= community_level)
        {
            for entity_id in &row.entity_ids {
                entity_max
                    .entry(entity_id.clone())
                    .and_modify(|value| *value = (*value).max(row.community))
                    .or_insert(row.community);
            }
        }
        entity_max.into_values().collect()
    };
    let mut rows = Vec::new();
    for row in 0..reports.height() {
        let level = required_i64(reports, row, "level", method, "community_reports")?;
        let community = required_i64(reports, row, "community", method, "community_reports")?;
        if level > community_level || !allowed.contains(&community) {
            continue;
        }
        rows.push(CommunityReport {
            id: required_string(reports, row, "id", method, "community_reports")?,
            short_id: community.to_string(),
            community_id: community.to_string(),
            title: required_string(reports, row, "title", method, "community_reports")?,
            summary: required_string(reports, row, "summary", method, "community_reports")?,
            full_content: required_string(
                reports,
                row,
                "full_content",
                method,
                "community_reports",
            )?,
            rank: optional_f64(reports, row, "rank", method, "community_reports")?,
            full_content_embedding: None,
        });
    }
    Ok(rows)
}

/// Restore report-backed Query communities and their hierarchy fields.
///
/// # Errors
///
/// Returns a typed table error for missing or incompatible columns.
pub fn read_indexer_communities(
    communities: &DataFrame,
    reports: &DataFrame,
    method: SearchMethod,
) -> Result<Vec<Community>> {
    let mut report_ids = BTreeSet::new();
    for row in 0..reports.height() {
        report_ids.insert(required_i64(
            reports,
            row,
            "community",
            method,
            "community_reports",
        )?);
    }
    let mut rows = Vec::new();
    for row in 0..communities.height() {
        let community = required_i64(communities, row, "community", method, "communities")?;
        if !report_ids.contains(&community) {
            tracing::warn!(method = %method, community, "community has no report and is unavailable to query");
            continue;
        }
        rows.push(Community {
            id: required_string(communities, row, "id", method, "communities")?,
            short_id: community.to_string(),
            title: required_string(communities, row, "title", method, "communities")?,
            level: required_i64(communities, row, "level", method, "communities")?,
            parent: required_i64(communities, row, "parent", method, "communities")?,
            children: i64_list(communities, row, "children", method, "communities")?,
        });
    }
    Ok(rows)
}

/// Hydrate report embeddings by report id and return the number not found.
///
/// Missing individual ids preserve upstream behavior by leaving the report
/// embedding empty. Provider and schema failures remain typed errors.
///
/// # Errors
///
/// Returns a typed vector error when the required index is missing or invalid.
pub async fn read_indexer_report_embeddings(
    reports: &mut [CommunityReport],
    store: &dyn VectorStore,
    schema: &VectorIndexSchema,
    method: SearchMethod,
) -> Result<usize> {
    let mut missing = 0_usize;
    for report in reports {
        match store.get_by_id(schema, &report.id).await {
            Ok(Some(document)) => report.full_content_embedding = Some(document.vector),
            Ok(None) => {
                report.full_content_embedding = None;
                missing = missing.saturating_add(1);
            }
            Err(source @ VectorError::MissingIndex { .. }) => {
                return Err(QueryError::MissingVectorIndex {
                    method,
                    operation: "hydrate community report embeddings",
                    index: schema.index_name.clone(),
                    source: Box::new(source),
                });
            }
            Err(source) => {
                return Err(QueryError::InvalidVectorIndex {
                    method,
                    operation: "hydrate community report embeddings",
                    index: schema.index_name.clone(),
                    source: Box::new(source),
                });
            }
        }
    }
    Ok(missing)
}

#[derive(Debug)]
struct CommunityMembership {
    community: i64,
    level: i64,
    entity_ids: Vec<String>,
}

fn community_memberships(
    dataframe: &DataFrame,
    method: SearchMethod,
) -> Result<Vec<CommunityMembership>> {
    let mut rows = Vec::with_capacity(dataframe.height());
    for row in 0..dataframe.height() {
        rows.push(CommunityMembership {
            community: required_i64(dataframe, row, "community", method, "communities")?,
            level: required_i64(dataframe, row, "level", method, "communities")?,
            entity_ids: string_list(dataframe, row, "entity_ids", method, "communities")?,
        });
    }
    Ok(rows)
}

fn column<'a>(
    dataframe: &'a DataFrame,
    name: &str,
    method: SearchMethod,
    table: &'static str,
    expected: &'static str,
) -> Result<&'a Column> {
    dataframe
        .column(name)
        .map_err(|_| QueryError::InvalidQueryTable {
            method,
            operation: "adapt query table",
            details: Box::new(QueryTableErrorDetails {
                table,
                column: name.to_owned(),
                expected,
                actual: "missing".to_owned(),
                row: String::new(),
                message: "required column is absent".to_owned(),
                source: None,
            }),
        })
}

fn value<'a>(
    dataframe: &'a DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
    expected: &'static str,
) -> Result<(AnyValue<'a>, String)> {
    let column = column(dataframe, name, method, table, expected)?;
    let actual = column.dtype().to_string();
    column
        .get(row)
        .map(|value| (value, actual.clone()))
        .map_err(|source| {
            invalid_value(
                method,
                table,
                name,
                expected,
                &actual,
                row,
                &source.to_string(),
            )
        })
}

fn required_string(
    dataframe: &DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
) -> Result<String> {
    let (value, actual) = value(dataframe, row, name, method, table, "string")?;
    string_value(value).ok_or_else(|| {
        invalid_value(
            method,
            table,
            name,
            "string",
            &actual,
            row,
            "value is null or not UTF-8",
        )
    })
}

fn required_stringish(
    dataframe: &DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
) -> Result<String> {
    let (value, actual) = value(dataframe, row, name, method, table, "string or integer")?;
    stringish_value(&value).ok_or_else(|| {
        invalid_value(
            method,
            table,
            name,
            "string or integer",
            &actual,
            row,
            "value is null or incompatible",
        )
    })
}

fn optional_string(
    dataframe: &DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
) -> Result<Option<String>> {
    let (value, actual) = value(dataframe, row, name, method, table, "nullable string")?;
    match value {
        AnyValue::Null => Ok(None),
        value => string_value(value).map(Some).ok_or_else(|| {
            invalid_value(
                method,
                table,
                name,
                "nullable string",
                &actual,
                row,
                "incompatible value",
            )
        }),
    }
}

fn optional_stringish(
    dataframe: &DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
) -> Result<Option<String>> {
    let (value, actual) = value(
        dataframe,
        row,
        name,
        method,
        table,
        "nullable string or integer",
    )?;
    match value {
        AnyValue::Null => Ok(None),
        value => stringish_value(&value).map(Some).ok_or_else(|| {
            invalid_value(
                method,
                table,
                name,
                "nullable string or integer",
                &actual,
                row,
                "incompatible value",
            )
        }),
    }
}

fn required_i64(
    dataframe: &DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
) -> Result<i64> {
    optional_i64(dataframe, row, name, method, table)?
        .ok_or_else(|| invalid_value(method, table, name, "integer", "null", row, "value is null"))
}

fn optional_i64(
    dataframe: &DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
) -> Result<Option<i64>> {
    let (value, actual) = value(dataframe, row, name, method, table, "nullable integer")?;
    integer_value(&value).map_err(|message| {
        invalid_value(
            method,
            table,
            name,
            "nullable integer",
            &actual,
            row,
            &message,
        )
    })
}

fn optional_f64(
    dataframe: &DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
) -> Result<Option<f64>> {
    let (value, actual) = value(dataframe, row, name, method, table, "nullable float")?;
    let converted = match value {
        AnyValue::Null => return Ok(None),
        AnyValue::Float32(value) => f64::from(value),
        AnyValue::Float64(value) => value,
        _ => {
            return Err(invalid_value(
                method,
                table,
                name,
                "nullable float",
                &actual,
                row,
                "incompatible value",
            ));
        }
    };
    if converted.is_finite() {
        Ok(Some(converted))
    } else {
        Err(invalid_value(
            method,
            table,
            name,
            "finite float",
            &actual,
            row,
            "value is non-finite",
        ))
    }
}

fn string_list(
    dataframe: &DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
) -> Result<Vec<String>> {
    let (value, actual) = value(dataframe, row, name, method, table, "nullable list<string>")?;
    match value {
        AnyValue::Null => Ok(Vec::new()),
        AnyValue::List(series) => {
            let mut output = Vec::with_capacity(series.len());
            for index in 0..series.len() {
                let item = series.get(index).map_err(|source| {
                    invalid_value(
                        method,
                        table,
                        name,
                        "list<string>",
                        &actual,
                        row,
                        &source.to_string(),
                    )
                })?;
                if let Some(item) = string_value(item.clone()) {
                    output.push(item);
                } else if !matches!(item, AnyValue::Null) {
                    return Err(invalid_value(
                        method,
                        table,
                        name,
                        "list<string>",
                        &actual,
                        row,
                        "list contains a non-string value",
                    ));
                }
            }
            Ok(output)
        }
        _ => Err(invalid_value(
            method,
            table,
            name,
            "nullable list<string>",
            &actual,
            row,
            "incompatible value",
        )),
    }
}

fn i64_list(
    dataframe: &DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
) -> Result<Vec<i64>> {
    let (value, actual) = value(
        dataframe,
        row,
        name,
        method,
        table,
        "nullable list<integer>",
    )?;
    match value {
        AnyValue::Null => Ok(Vec::new()),
        AnyValue::List(series) => {
            let mut output = Vec::with_capacity(series.len());
            for index in 0..series.len() {
                let item = series.get(index).map_err(|source| {
                    invalid_value(
                        method,
                        table,
                        name,
                        "list<integer>",
                        &actual,
                        row,
                        &source.to_string(),
                    )
                })?;
                if let Some(item) = integer_value(&item).map_err(|message| {
                    invalid_value(method, table, name, "list<integer>", &actual, row, &message)
                })? {
                    output.push(item);
                }
            }
            Ok(output)
        }
        _ => Err(invalid_value(
            method,
            table,
            name,
            "nullable list<integer>",
            &actual,
            row,
            "incompatible value",
        )),
    }
}

fn string_value(value: AnyValue<'_>) -> Option<String> {
    match value {
        AnyValue::String(value) => Some(value.to_owned()),
        AnyValue::StringOwned(value) => Some(value.to_string()),
        _ => None,
    }
}

fn stringish_value(value: &AnyValue<'_>) -> Option<String> {
    string_value(value.clone()).or_else(|| {
        integer_value(value)
            .ok()
            .flatten()
            .map(|value| value.to_string())
    })
}

fn integer_value(value: &AnyValue<'_>) -> std::result::Result<Option<i64>, String> {
    match value {
        AnyValue::Null => Ok(None),
        AnyValue::Int32(value) => Ok(Some(i64::from(*value))),
        AnyValue::Int64(value) => Ok(Some(*value)),
        AnyValue::UInt32(value) => Ok(Some(i64::from(*value))),
        AnyValue::UInt64(value) => i64::try_from(*value)
            .map(Some)
            .map_err(|_| format!("unsigned value {value} exceeds i64")),
        _ => Err("incompatible integer value".to_owned()),
    }
}

fn invalid_value(
    method: SearchMethod,
    table: &'static str,
    column: &str,
    expected: &'static str,
    actual: &str,
    row: usize,
    message: &str,
) -> QueryError {
    QueryError::InvalidQueryTable {
        method,
        operation: "adapt query table",
        details: Box::new(QueryTableErrorDetails {
            table,
            column: column.to_owned(),
            expected,
            actual: actual.to_owned(),
            row: format!(" at row {row}"),
            message: message.to_owned(),
            source: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use graphloom_vectors::{LanceDbVectorStore, VectorDocument, VectorStoreConfig};
    use polars_core::prelude::{DataFrame, DataType, NamedFrom, Series, df};
    use tempfile::TempDir;

    use super::*;
    use crate::dataframe::{i64_list_column, list_column};

    fn text_units(n_tokens: Series, null_lists: bool) -> DataFrame {
        let mut dataframe = df!(
            "id" => ["tu-a", "tu-b"],
            "text" => ["A|quoted \"text\"", "B\nUnicode 世界"],
            "document_id" => [Some("doc-a"), None],
        )
        .expect("text units");
        let entity_ids = if null_lists {
            Series::new_null("entity_ids".into(), 2).into()
        } else {
            list_column(
                "entity_ids",
                &[vec!["e-a".to_owned()], Vec::<String>::new()],
            )
        };
        dataframe.with_column(entity_ids).expect("entity ids");
        dataframe
            .with_column(list_column(
                "relationship_ids",
                &[vec!["r-a".to_owned()], Vec::<String>::new()],
            ))
            .expect("relationship ids");
        dataframe
            .with_column(list_column(
                "covariate_ids",
                &[Vec::<String>::new(), vec!["c-b".to_owned()]],
            ))
            .expect("covariate ids");
        dataframe.with_column(n_tokens.into()).expect("n_tokens");
        dataframe
    }

    fn communities() -> DataFrame {
        let mut dataframe = df!(
            "id" => ["co-1", "co-3", "co-2", "co-9"],
            "community" => [1_u32, 3, 2, 9],
            "level" => [0_i32, 1, 1, 3],
            "title" => ["Community 1", "Community 3", "Community 2", "Community 9"],
            "parent" => [-1_i64, 1, 1, 3],
        )
        .expect("communities");
        dataframe
            .with_column(list_column(
                "entity_ids",
                &[
                    vec!["e-1".to_owned(), "e-2".to_owned()],
                    vec!["e-1".to_owned(), "e-1".to_owned()],
                    vec!["e-2".to_owned()],
                    vec!["e-3".to_owned()],
                ],
            ))
            .expect("entity ids");
        dataframe
            .with_column(i64_list_column(
                "children",
                &[vec![2, 3], Vec::new(), Vec::new(), Vec::new()],
            ))
            .expect("children");
        dataframe
    }

    fn reports() -> DataFrame {
        df!(
            "id" => ["rp-1", "rp-2", "rp-3"],
            "community" => [1_u64, 2, 3],
            "level" => [0_u32, 1, 1],
            "title" => ["Report 1", "Report 2", "Report 3"],
            "summary" => ["S1", "S2", "S3"],
            "full_content" => ["F1", "F2", "F3"],
            "rank" => [1.0_f32, 2.0, 3.0],
        )
        .expect("reports")
    }

    #[test]
    fn test_should_adapt_graphrag_and_graphloom_text_unit_physical_types() {
        let graph_rag = text_units(Series::new("n_tokens".into(), [7_i32, 8]), false);
        let graphloom = text_units(Series::new("n_tokens".into(), [7_u32, 8]), true);

        let graph_rag_rows =
            read_indexer_text_units(&graph_rag, SearchMethod::Basic).expect("GraphRAG fixture");
        let graphloom_rows =
            read_indexer_text_units(&graphloom, SearchMethod::Basic).expect("GraphLoom fixture");

        assert_eq!(graph_rag_rows[0].short_id, "0");
        assert_eq!(graph_rag_rows[1].short_id, "1");
        assert_eq!(graph_rag_rows[0].n_tokens, Some(7));
        assert_eq!(graphloom_rows[0].n_tokens, Some(7));
        assert!(graphloom_rows[0].entity_ids.is_empty());
        assert_eq!(graph_rag_rows[1].document_id, None);
    }

    #[test]
    fn test_should_aggregate_entities_and_roll_up_reports_by_entity() {
        let communities = communities();
        let mut entities = df!(
            "id" => ["e-1", "e-1", "e-2", "e-missing"],
            "human_readable_id" => [1_u64, 99, 2, 4],
            "title" => ["Alice", "duplicate", "Bob", "Nobody"],
            "type" => ["PERSON", "PERSON", "PERSON", "PERSON"],
            "description" => ["A", "duplicate", "B", "N"],
            "degree" => [4_i32, 99, 5, 0],
        )
        .expect("entities");
        entities
            .with_column(list_column(
                "text_unit_ids",
                &[
                    vec!["tu-a".to_owned()],
                    vec!["duplicate".to_owned()],
                    vec!["tu-b".to_owned()],
                    Vec::new(),
                ],
            ))
            .expect("text unit ids");

        let adapted = read_indexer_entities(&entities, &communities, 1, SearchMethod::Local)
            .expect("entities");
        assert_eq!(adapted.len(), 3);
        assert_eq!(adapted[0].community_ids, ["1", "3"]);
        assert_eq!(adapted[1].community_ids, ["1", "2"]);
        assert_eq!(adapted[2].community_ids, ["-1"]);
        assert_eq!(adapted[0].short_id.as_deref(), Some("1"));

        let rolled_up =
            read_indexer_reports(&reports(), &communities, 1, false, SearchMethod::Global)
                .expect("rolled-up reports");
        assert_eq!(
            rolled_up
                .iter()
                .map(|report| report.community_id.as_str())
                .collect::<Vec<_>>(),
            ["2", "3"]
        );
        let dynamic = read_indexer_reports(&reports(), &communities, 1, true, SearchMethod::Global)
            .expect("dynamic reports");
        assert_eq!(dynamic.len(), 3);
    }

    #[test]
    fn test_should_adapt_relationship_covariate_and_report_backed_hierarchy() {
        let mut relationships = df!(
            "id" => ["r-1"],
            "human_readable_id" => [7_u32],
            "source" => ["Alice"],
            "target" => ["Bob"],
            "description" => ["knows"],
            "weight" => [0.75_f32],
            "combined_degree" => [9_u64],
        )
        .expect("relationships");
        relationships
            .with_column(list_column("text_unit_ids", &[vec!["tu-a".to_owned()]]))
            .expect("text units");
        let relationship = read_indexer_relationships(&relationships, SearchMethod::Local)
            .expect("relationships")
            .remove(0);
        assert_eq!(relationship.short_id.as_deref(), Some("7"));
        assert_eq!(relationship.rank, Some(9));
        assert_eq!(relationship.weight, Some(0.75));

        let covariates = df!(
            "id" => [42_u64],
            "human_readable_id" => [11_i32],
            "subject_id" => ["e-1"],
            "type" => ["claim"],
            "object_id" => [Some("e-2")],
            "status" => [Some("TRUE")],
            "start_date" => [None::<&str>],
            "end_date" => [None::<&str>],
            "description" => [Some("Alice knows Bob")],
        )
        .expect("covariates");
        let covariate = read_indexer_covariates(&covariates, SearchMethod::Local)
            .expect("covariates")
            .remove(0);
        assert_eq!(covariate.id, "42");
        assert_eq!(covariate.short_id.as_deref(), Some("11"));

        let adapted = read_indexer_communities(&communities(), &reports(), SearchMethod::Global)
            .expect("communities");
        assert_eq!(adapted.len(), 3);
        assert_eq!(adapted[0].parent, -1);
        assert_eq!(adapted[0].children, [2, 3]);
    }

    #[test]
    fn test_should_include_precise_table_type_and_row_in_adapter_errors() {
        let mut invalid = text_units(Series::new("n_tokens".into(), [7_i64, 8]), false);
        invalid
            .replace("text", Series::new("text".into(), [1_i64, 2]).into())
            .expect("replace text");
        let error = read_indexer_text_units(&invalid, SearchMethod::Basic)
            .expect_err("invalid text should fail");
        let message = error.to_string();
        assert!(message.contains("basic"));
        assert!(message.contains("text_units"));
        assert!(message.contains("column text"));
        assert!(message.contains("expected string"));
        assert!(message.contains("actual i64"));
        assert!(message.contains("row 0"));

        let too_large = Series::new("value".into(), [u64::MAX]);
        assert_eq!(too_large.dtype(), &DataType::UInt64);
    }

    #[tokio::test]
    async fn test_should_hydrate_report_embeddings_and_count_missing_ids() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut config = VectorStoreConfig::default();
        config.db_uri = tempdir.path().join("lancedb").display().to_string();
        config.vector_size = 2;
        let schema = VectorIndexSchema::for_embedding_name("community_full_content", 2);
        let store = LanceDbVectorStore::connect(&config).await.expect("LanceDB");
        store.ensure_index(&schema).await.expect("index");
        store
            .upsert_documents(
                &schema,
                &[VectorDocument {
                    id: "rp-1".to_owned(),
                    vector: vec![0.2, 0.8],
                }],
            )
            .await
            .expect("embedding");
        let mut adapted =
            read_indexer_reports(&reports(), &communities(), 1, true, SearchMethod::Drift)
                .expect("reports");

        let missing =
            read_indexer_report_embeddings(&mut adapted, &store, &schema, SearchMethod::Drift)
                .await
                .expect("hydrate embeddings");

        assert_eq!(missing, 2);
        assert_eq!(adapted[0].full_content_embedding, Some(vec![0.2, 0.8]));
        assert!(adapted[1].full_content_embedding.is_none());
        assert!(adapted[2].full_content_embedding.is_none());
    }
}
