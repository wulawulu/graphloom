//! `GraphRAG` 3.1 index-table to Query-model adapters.

use std::collections::{BTreeMap, BTreeSet};

use graphloom_vectors::{VectorError, VectorIndexSchema, VectorStore};
use polars_core::prelude::{AnyValue, Column, DataFrame, DataType};

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
    validate_schema(
        dataframe,
        method,
        "text_units",
        &[("id", "string"), ("text", "string")],
        &[
            ("entity_ids", "nullable list<string>"),
            ("relationship_ids", "nullable list<string>"),
            ("n_tokens", "nullable integer"),
            ("document_id", "nullable string"),
        ],
    )?;
    let mut rows = Vec::with_capacity(dataframe.height());
    for row in 0..dataframe.height() {
        rows.push(TextUnit {
            id: required_string(dataframe, row, "id", method, "text_units")?,
            short_id: row.to_string(),
            text: required_string(dataframe, row, "text", method, "text_units")?,
            entity_ids: optional_string_list(dataframe, row, "entity_ids", method, "text_units")?,
            relationship_ids: optional_string_list(
                dataframe,
                row,
                "relationship_ids",
                method,
                "text_units",
            )?,
            // GraphRAG 3.1 calls read_text_units(covariates_col=None), so the
            // persisted column is deliberately ignored even when Phase 1 wrote it.
            covariate_ids: Vec::new(),
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
    validate_schema(
        dataframe,
        method,
        "relationships",
        &[("id", "string"), ("source", "string"), ("target", "string")],
        &[
            ("human_readable_id", "nullable string or integer"),
            ("description", "nullable string"),
            ("weight", "nullable float"),
            ("combined_degree", "nullable integer"),
            ("text_unit_ids", "nullable list<string>"),
        ],
    )?;
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
            text_unit_ids: optional_string_list(
                dataframe,
                row,
                "text_unit_ids",
                method,
                "relationships",
            )?,
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
    validate_schema(
        dataframe,
        method,
        "covariates",
        &[
            ("id", "string or integer"),
            ("subject_id", "string"),
            ("type", "string"),
        ],
        &[
            ("human_readable_id", "nullable string or integer"),
            ("object_id", "nullable string"),
            ("status", "nullable string"),
            ("start_date", "nullable string"),
            ("end_date", "nullable string"),
            ("description", "nullable string"),
        ],
    )?;
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
    validate_schema(
        entities,
        method,
        "entities",
        &[("id", "string"), ("title", "string")],
        &[
            ("human_readable_id", "nullable string or integer"),
            ("type", "nullable string"),
            ("description", "nullable string"),
            ("degree", "nullable integer"),
            ("text_unit_ids", "nullable list<string>"),
        ],
    )?;
    validate_schema(
        communities,
        method,
        "communities",
        &[
            ("community", "integer"),
            ("level", "integer"),
            ("entity_ids", "list<string>"),
        ],
        &[],
    )?;
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
            text_unit_ids: optional_string_list(
                entities,
                row,
                "text_unit_ids",
                method,
                "entities",
            )?,
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
    validate_required_columns(
        reports,
        method,
        "community_reports",
        &[
            ("id", "string"),
            ("community", "integer"),
            ("level", "integer"),
            ("title", "string"),
            ("summary", "string"),
            ("full_content", "string"),
        ],
    )?;
    let _ = required_column(reports, 0, "level", method, "community_reports", "integer")?;
    let rollup_columns = if dynamic {
        &[("level", "integer"), ("entity_ids", "any")][..]
    } else {
        &[
            ("level", "integer"),
            ("entity_ids", "any"),
            ("community", "integer"),
            ("title", "string"),
        ][..]
    };
    validate_required_columns(communities, method, "communities", rollup_columns)?;
    let _ = required_column(communities, 0, "level", method, "communities", "integer")?;
    let allowed = if dynamic {
        // GraphRAG still explodes and level-filters the community rows in this
        // path, but it never reads their title before returning the reports.
        validate_dynamic_community_rows(communities, community_level, method)?;
        None
    } else {
        let memberships = community_rollup_rows(communities, community_level, method)?;
        // This title-based roll-up is intentionally non-obvious, but it is the
        // behavior executed by GraphRAG 3.1.0. Do not replace it with an
        // entity-id roll-up without an explicit compatibility decision.
        let mut title_max = Vec::<(String, i64)>::new();
        for row in &memberships {
            if let Some((_, community)) =
                title_max.iter_mut().find(|(title, _)| title == &row.title)
            {
                *community = (*community).max(row.community);
            } else {
                title_max.push((row.title.clone(), row.community));
            }
        }
        let mut communities = Vec::new();
        for (_, community) in title_max {
            if !communities.contains(&community) {
                communities.push(community);
            }
        }
        Some(communities)
    };
    let mut rows = Vec::new();
    for row in 0..reports.height() {
        let level = required_i64(reports, row, "level", method, "community_reports")?;
        if level > community_level {
            continue;
        }
        let community = if dynamic {
            required_i64(reports, row, "community", method, "community_reports")?
        } else {
            let Some(community) =
                optional_community_id(reports, row, "community", method, "community_reports")?
            else {
                continue;
            };
            community
        };
        if allowed
            .as_ref()
            .is_some_and(|communities| !communities.contains(&community))
        {
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
    validate_schema(
        communities,
        method,
        "communities",
        &[
            ("id", "string"),
            ("community", "integer"),
            ("title", "string"),
            ("level", "integer"),
            ("parent", "integer"),
            ("children", "list<integer>"),
        ],
        &[],
    )?;
    validate_schema(
        reports,
        method,
        "community_reports",
        &[("community", "integer")],
        &[],
    )?;
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
            children: required_i64_list(communities, row, "children", method, "communities")?,
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

#[derive(Debug)]
struct CommunityRollupRow {
    community: i64,
    title: String,
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
            entity_ids: required_string_list(dataframe, row, "entity_ids", method, "communities")?,
        });
    }
    Ok(rows)
}

fn community_rollup_rows(
    dataframe: &DataFrame,
    community_level: i64,
    method: SearchMethod,
) -> Result<Vec<CommunityRollupRow>> {
    let mut rows = Vec::with_capacity(dataframe.height());
    for row in 0..dataframe.height() {
        let level = required_i64(dataframe, row, "level", method, "communities")?;
        if level > community_level {
            continue;
        }
        let Some(title) = optional_string(dataframe, row, "title", method, "communities")? else {
            continue;
        };
        rows.push(CommunityRollupRow {
            community: optional_community_id(dataframe, row, "community", method, "communities")?
                .unwrap_or(-1),
            title,
        });
    }
    Ok(rows)
}

fn validate_dynamic_community_rows(
    dataframe: &DataFrame,
    community_level: i64,
    method: SearchMethod,
) -> Result<()> {
    for row in 0..dataframe.height() {
        let level = required_i64(dataframe, row, "level", method, "communities")?;
        if level > community_level {
            continue;
        }
    }
    Ok(())
}

fn validate_schema(
    dataframe: &DataFrame,
    method: SearchMethod,
    table: &'static str,
    required: &[(&str, &'static str)],
    optional: &[(&str, &'static str)],
) -> Result<()> {
    for (name, expected) in required {
        let _ = required_column(dataframe, 0, name, method, table, expected)?;
    }
    for (name, expected) in optional {
        let _ = optional_column(dataframe, 0, name, method, table, expected)?;
    }
    Ok(())
}

fn validate_required_columns(
    dataframe: &DataFrame,
    method: SearchMethod,
    table: &'static str,
    required: &[(&str, &'static str)],
) -> Result<()> {
    for (name, expected) in required {
        if dataframe.get_column_index(name).is_none() {
            return Err(QueryError::InvalidQueryTable {
                method,
                operation: "adapt query table",
                details: Box::new(QueryTableErrorDetails {
                    table,
                    column: (*name).to_owned(),
                    expected,
                    actual: "missing".to_owned(),
                    row: " at row 0".to_owned(),
                    message: "required column is absent".to_owned(),
                    source: None,
                }),
            });
        }
    }
    Ok(())
}

fn required_column<'a>(
    dataframe: &'a DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
    expected: &'static str,
) -> Result<&'a Column> {
    let column = dataframe
        .column(name)
        .map_err(|_| QueryError::InvalidQueryTable {
            method,
            operation: "adapt query table",
            details: Box::new(QueryTableErrorDetails {
                table,
                column: name.to_owned(),
                expected,
                actual: "missing".to_owned(),
                row: format!(" at row {row}"),
                message: "required column is absent".to_owned(),
                source: None,
            }),
        })?;
    validate_column_type(column, row, name, method, table, expected)?;
    Ok(column)
}

fn optional_column<'a>(
    dataframe: &'a DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
    expected: &'static str,
) -> Result<Option<&'a Column>> {
    let Some(column) = dataframe.column(name).ok() else {
        return Ok(None);
    };
    if matches!(column.dtype(), DataType::Null) {
        return Ok(Some(column));
    }
    validate_column_type(column, row, name, method, table, expected)?;
    Ok(Some(column))
}

fn validate_column_type(
    column: &Column,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
    expected: &'static str,
) -> Result<()> {
    let dtype = column.dtype();
    if column_type_matches(dtype, expected) {
        return Ok(());
    }
    Err(invalid_value(
        method,
        table,
        name,
        expected,
        &dtype.to_string(),
        row,
        "column has an incompatible physical type",
    ))
}

fn column_type_matches(dtype: &DataType, expected: &str) -> bool {
    let is_integer = matches!(
        dtype,
        DataType::Int32 | DataType::Int64 | DataType::UInt32 | DataType::UInt64
    );
    match expected {
        "any" => true,
        "string" | "nullable string" => matches!(dtype, DataType::String),
        "string or integer" | "nullable string or integer" => {
            matches!(dtype, DataType::String) || is_integer
        }
        "integer" | "nullable integer" => is_integer,
        "nullable community number" => {
            is_integer || matches!(dtype, DataType::Float32 | DataType::Float64)
        }
        "nullable float" => matches!(dtype, DataType::Float32 | DataType::Float64) || is_integer,
        "list<string>" | "nullable list<string>" => {
            matches!(dtype, DataType::List(inner) if matches!(inner.as_ref(), DataType::String | DataType::Null))
        }
        "list<integer>" | "nullable list<integer>" => {
            matches!(dtype, DataType::List(inner) if matches!(inner.as_ref(), DataType::Null) || column_type_matches(inner, "integer"))
        }
        _ => false,
    }
}

fn required_value<'a>(
    dataframe: &'a DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
    expected: &'static str,
) -> Result<(AnyValue<'a>, String)> {
    let column = required_column(dataframe, row, name, method, table, expected)?;
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

fn optional_value<'a>(
    dataframe: &'a DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
    expected: &'static str,
) -> Result<Option<(AnyValue<'a>, String)>> {
    let Some(column) = optional_column(dataframe, row, name, method, table, expected)? else {
        return Ok(None);
    };
    let actual = column.dtype().to_string();
    column
        .get(row)
        .map(|value| Some((value, actual.clone())))
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
    let (value, actual) = required_value(dataframe, row, name, method, table, "string")?;
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
    let (value, actual) = required_value(dataframe, row, name, method, table, "string or integer")?;
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
    let Some((value, actual)) =
        optional_value(dataframe, row, name, method, table, "nullable string")?
    else {
        return Ok(None);
    };
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
    let Some((value, actual)) = optional_value(
        dataframe,
        row,
        name,
        method,
        table,
        "nullable string or integer",
    )?
    else {
        return Ok(None);
    };
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
    let (value, actual) = required_value(dataframe, row, name, method, table, "integer")?;
    integer_value(&value)
        .map_err(|message| invalid_value(method, table, name, "integer", &actual, row, &message))?
        .ok_or_else(|| {
            invalid_value(
                method,
                table,
                name,
                "integer",
                &actual,
                row,
                "value is null",
            )
        })
}

fn optional_i64(
    dataframe: &DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
) -> Result<Option<i64>> {
    let Some((value, actual)) =
        optional_value(dataframe, row, name, method, table, "nullable integer")?
    else {
        return Ok(None);
    };
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

fn optional_community_id(
    dataframe: &DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
) -> Result<Option<i64>> {
    let Some((value, actual)) = optional_value(
        dataframe,
        row,
        name,
        method,
        table,
        "nullable community number",
    )?
    else {
        return Ok(None);
    };
    match value {
        AnyValue::Null => Ok(None),
        AnyValue::Float32(value) => {
            integral_f64(f64::from(value), method, table, name, &actual, row).map(Some)
        }
        AnyValue::Float64(value) => {
            integral_f64(value, method, table, name, &actual, row).map(Some)
        }
        value => integer_value(&value).map_err(|message| {
            invalid_value(
                method,
                table,
                name,
                "nullable community number",
                &actual,
                row,
                &message,
            )
        }),
    }
}

fn integral_f64(
    value: f64,
    method: SearchMethod,
    table: &'static str,
    name: &str,
    actual: &str,
    row: usize,
) -> Result<i64> {
    if value.is_finite()
        && value.fract() == 0.0
        && value >= i64::MIN as f64
        // `i64::MAX as f64` rounds to 2^63, so the upper bound is exclusive.
        && value < i64::MAX as f64
    {
        // The finite, integral, explicitly bounded value is representable.
        #[allow(clippy::cast_possible_truncation)]
        return Ok(value as i64);
    }
    Err(invalid_value(
        method,
        table,
        name,
        "integral finite community number",
        actual,
        row,
        "value cannot be represented as i64",
    ))
}

fn optional_f64(
    dataframe: &DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
) -> Result<Option<f64>> {
    let Some((value, actual)) =
        optional_value(dataframe, row, name, method, table, "nullable float")?
    else {
        return Ok(None);
    };
    let converted = match value {
        AnyValue::Null => return Ok(None),
        AnyValue::Float32(value) => f64::from(value),
        AnyValue::Float64(value) => value,
        AnyValue::Int32(value) => f64::from(value),
        AnyValue::Int64(value) => value as f64,
        AnyValue::UInt32(value) => f64::from(value),
        AnyValue::UInt64(value) => value as f64,
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

fn optional_string_list(
    dataframe: &DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
) -> Result<Vec<String>> {
    let Some((value, actual)) =
        optional_value(dataframe, row, name, method, table, "nullable list<string>")?
    else {
        return Ok(Vec::new());
    };
    string_list_value(value, method, table, name, &actual, row)
}

fn required_string_list(
    dataframe: &DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
) -> Result<Vec<String>> {
    let (value, actual) = required_value(dataframe, row, name, method, table, "list<string>")?;
    if matches!(value, AnyValue::Null) {
        return Err(invalid_value(
            method,
            table,
            name,
            "list<string>",
            &actual,
            row,
            "value is null",
        ));
    }
    string_list_value(value, method, table, name, &actual, row)
}

fn string_list_value(
    value: AnyValue<'_>,
    method: SearchMethod,
    table: &'static str,
    name: &str,
    actual: &str,
    row: usize,
) -> Result<Vec<String>> {
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
                        actual,
                        row,
                        &source.to_string(),
                    )
                })?;
                let item = string_value(item).ok_or_else(|| {
                    invalid_value(
                        method,
                        table,
                        name,
                        "list<string>",
                        actual,
                        row,
                        "list contains a null or non-string value",
                    )
                })?;
                output.push(item);
            }
            Ok(output)
        }
        _ => Err(invalid_value(
            method,
            table,
            name,
            "nullable list<string>",
            actual,
            row,
            "incompatible value",
        )),
    }
}

fn required_i64_list(
    dataframe: &DataFrame,
    row: usize,
    name: &str,
    method: SearchMethod,
    table: &'static str,
) -> Result<Vec<i64>> {
    let (value, actual) = required_value(dataframe, row, name, method, table, "list<integer>")?;
    match value {
        AnyValue::Null => Err(invalid_value(
            method,
            table,
            name,
            "list<integer>",
            &actual,
            row,
            "value is null",
        )),
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
                let item = integer_value(&item).map_err(|message| {
                    invalid_value(method, table, name, "list<integer>", &actual, row, &message)
                })?;
                output.push(item.ok_or_else(|| {
                    invalid_value(
                        method,
                        table,
                        name,
                        "list<integer>",
                        &actual,
                        row,
                        "list contains a null value",
                    )
                })?);
            }
            Ok(output)
        }
        _ => Err(invalid_value(
            method,
            table,
            name,
            "list<integer>",
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

    fn rollup_communities() -> DataFrame {
        let mut dataframe = df!(
            "id" => ["co-1", "co-4", "co-2", "co-3", "co-9", "co-null", "co-no-title"],
            "community" => [Some(1_i64), Some(4), Some(2), Some(3), Some(9), None, Some(8)],
            "level" => [0_i64, 1, 1, 1, 3, 1, 1],
            "title" => [Some("Alpha"), Some("Alpha"), Some("Beta"), Some("Gamma"), Some("Alpha"), Some("Delta"), None],
            "parent" => [-1_i64, 1, 1, 1, 4, 1, 1],
        )
        .expect("roll-up communities");
        dataframe
            .with_column(list_column(
                "entity_ids",
                &[
                    vec!["entity-x".to_owned()],
                    vec!["entity-y".to_owned()],
                    vec!["entity-x".to_owned()],
                    vec!["entity-y".to_owned()],
                    vec!["entity-z".to_owned()],
                    vec!["entity-null".to_owned()],
                    vec!["entity-no-title".to_owned()],
                ],
            ))
            .expect("entity ids");
        dataframe
            .with_column(i64_list_column(
                "children",
                &[
                    vec![4],
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ],
            ))
            .expect("children");
        dataframe
    }

    fn rollup_reports() -> DataFrame {
        df!(
            "id" => ["rp-3", "rp-1", "rp-4", "rp-2", "rp-9", "rp-neg", "rp-8"],
            "community" => [3_i64, 1, 4, 2, 9, -1, 8],
            "level" => [1_i64, 0, 1, 2, 3, 1, 1],
            "title" => ["Report 3", "Report 1", "Report 4", "Report 2", "Report 9", "Report -1", "Report 8"],
            "summary" => ["S3", "S1", "S4", "S2", "S9", "SN", "S8"],
            "full_content" => ["F3", "F1", "F4", "F2", "F9", "FN", "F8"],
        )
        .expect("roll-up reports")
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
    fn test_should_aggregate_entity_community_memberships() {
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
    }

    #[test]
    fn test_should_match_graphrag_3_1_title_rollup_and_report_order_golden() {
        let communities = rollup_communities();
        let mut reports = rollup_reports();
        reports
            .replace(
                "title",
                Series::new(
                    "title".into(),
                    [
                        Some("Report 3"),
                        Some("Report 1"),
                        Some("Report 4"),
                        Some("Report 2"),
                        None,
                        Some("Report -1"),
                        Some("Report 8"),
                    ],
                )
                .into(),
            )
            .expect("excluded report may have a null title");

        let rolled_up =
            read_indexer_reports(&reports, &communities, 1, false, SearchMethod::Global)
                .expect("title roll-up reports");
        assert_eq!(
            rolled_up
                .iter()
                .map(|report| (report.id.as_str(), report.community_id.as_str()))
                .collect::<Vec<_>>(),
            [("rp-3", "3"), ("rp-4", "4"), ("rp-neg", "-1")]
        );
        let dynamic = read_indexer_reports(&reports, &communities, 1, true, SearchMethod::Global)
            .expect("dynamic reports");
        assert_eq!(
            dynamic
                .iter()
                .map(|report| (report.id.as_str(), report.community_id.as_str()))
                .collect::<Vec<_>>(),
            [
                ("rp-3", "3"),
                ("rp-1", "1"),
                ("rp-4", "4"),
                ("rp-neg", "-1"),
                ("rp-8", "8")
            ]
        );

        let mut dynamic_communities = df!(
            "community" => [1_i64],
            "level" => [0_i64],
        )
        .expect("dynamic communities without title");
        dynamic_communities
            .with_column(list_column("entity_ids", &[vec!["entity-x".to_owned()]]))
            .expect("dynamic entity ids");
        let dynamic = read_indexer_reports(
            &reports,
            &dynamic_communities,
            1,
            true,
            SearchMethod::Global,
        )
        .expect("dynamic selection must not require community titles");
        assert_eq!(dynamic.len(), 5);
    }

    #[test]
    fn test_should_accept_sparse_graphrag_query_tables() {
        let mut text_units = df!(
            "id" => ["tu-1"],
            "text" => ["sparse text"],
            "covariate_ids" => [42_i64],
        )
        .expect("sparse text units");
        let adapted = read_indexer_text_units(&text_units, SearchMethod::Basic)
            .expect("sparse text units should adapt");
        let text_unit = adapted.first().expect("text unit");
        assert_eq!(text_unit.short_id, "0");
        assert!(text_unit.entity_ids.is_empty());
        assert!(text_unit.relationship_ids.is_empty());
        assert!(text_unit.covariate_ids.is_empty());
        assert_eq!(text_unit.n_tokens, None);
        assert_eq!(text_unit.document_id, None);

        let entities = df!(
            "id" => ["entity-1"],
            "title" => ["Sparse Entity"],
        )
        .expect("sparse entities");
        let entities = read_indexer_entities(&entities, &communities(), 1, SearchMethod::Local)
            .expect("sparse entities should adapt");
        let entity = entities.first().expect("entity");
        assert_eq!(entity.short_id, None);
        assert_eq!(entity.entity_type, None);
        assert_eq!(entity.description, None);
        assert_eq!(entity.rank, None);
        assert!(entity.text_unit_ids.is_empty());

        let relationships = df!(
            "id" => ["relationship-1"],
            "source" => ["Sparse Entity"],
            "target" => ["Other Entity"],
        )
        .expect("sparse relationships");
        let relationship = read_indexer_relationships(&relationships, SearchMethod::Local)
            .expect("sparse relationships should adapt")
            .remove(0);
        assert_eq!(relationship.short_id, None);
        assert_eq!(relationship.description, None);
        assert_eq!(relationship.weight, None);
        assert_eq!(relationship.rank, None);
        assert!(relationship.text_unit_ids.is_empty());

        let covariates = df!(
            "id" => ["claim-1"],
            "subject_id" => ["entity-1"],
            "type" => ["claim"],
        )
        .expect("sparse covariates");
        let covariate = read_indexer_covariates(&covariates, SearchMethod::Local)
            .expect("sparse covariates should adapt")
            .remove(0);
        assert_eq!(covariate.short_id, None);
        assert_eq!(covariate.object_id, None);
        assert_eq!(covariate.status, None);
        assert_eq!(covariate.start_date, None);
        assert_eq!(covariate.end_date, None);
        assert_eq!(covariate.description, None);

        text_units
            .replace("text", Series::new("text".into(), [1_i64]).into())
            .expect("replace required text");
        let error = read_indexer_text_units(&text_units, SearchMethod::Basic)
            .expect_err("required type mismatch");
        assert!(error.to_string().contains("actual i64"));

        let missing_required = df!(
            "id" => ["relationship-1"],
            "source" => ["Sparse Entity"],
        )
        .expect("relationship missing target");
        let error = read_indexer_relationships(&missing_required, SearchMethod::Local)
            .expect_err("missing required target must fail");
        let message = error.to_string();
        assert!(message.contains("column target"));
        assert!(message.contains("actual missing"));
        assert!(message.contains("row 0"));
    }

    #[test]
    fn test_should_reject_present_optional_columns_with_incompatible_types() {
        let invalid = df!(
            "id" => ["entity-1"],
            "title" => ["Entity"],
            "description" => [7_i64],
        )
        .expect("invalid optional column");

        let error = read_indexer_entities(&invalid, &communities(), 1, SearchMethod::Local)
            .expect_err("invalid optional description must fail");
        let message = error.to_string();
        assert!(message.contains("local"));
        assert!(message.contains("entities"));
        assert!(message.contains("column description"));
        assert!(message.contains("expected nullable string"));
        assert!(message.contains("actual i64"));
        assert!(message.contains("row 0"));

        let list_with_null = Series::new(
            "text_unit_ids".into(),
            vec![Series::new("item".into(), [Some("tu-1"), None])],
        );
        let mut invalid_list = df!(
            "id" => ["entity-1"],
            "title" => ["Entity"],
        )
        .expect("entity with invalid optional list");
        invalid_list
            .with_column(list_with_null.into())
            .expect("optional list column");
        let error = read_indexer_entities(&invalid_list, &communities(), 1, SearchMethod::Local)
            .expect_err("null optional list element must fail");
        assert!(error.to_string().contains("null or non-string"));
    }

    #[test]
    fn test_should_validate_required_schema_for_empty_tables() {
        let error = read_indexer_text_units(&DataFrame::empty(), SearchMethod::Basic)
            .expect_err("empty table without required columns must fail");
        let message = error.to_string();
        assert!(message.contains("column id"));
        assert!(message.contains("actual missing"));
        assert!(message.contains("row 0"));

        let communities_without_entities = df!(
            "community" => [1_i64],
            "level" => [0_i64],
        )
        .expect("communities missing explode column");
        let error = read_indexer_reports(
            &rollup_reports(),
            &communities_without_entities,
            1,
            true,
            SearchMethod::Global,
        )
        .expect_err("GraphRAG explode requires entity_ids");
        assert!(error.to_string().contains("column entity_ids"));

        let mut null_children = df!(
            "id" => ["co-1"],
            "community" => [1_i64],
            "title" => ["Community"],
            "level" => [0_i64],
            "parent" => [-1_i64],
        )
        .expect("community with null children");
        null_children
            .with_column(Series::new("children".into(), &[None::<Series>]).into())
            .expect("null list column");
        let reports = df!("community" => [1_i64]).expect("community report key");
        let error = read_indexer_communities(&null_children, &reports, SearchMethod::Global)
            .expect_err("required children cannot be null");
        assert!(error.to_string().contains("value is null"));
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
