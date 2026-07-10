//! `DataFrame` codecs for community report operations.

use polars_core::prelude::*;
use serde_json::{Value, json};

use super::{
    ClaimContextRow, CommunityInputRow, CommunityReportFindingRow, CommunityReportRow,
    EntityContextRow, RelationshipContextRow,
};
use crate::{
    Result,
    dataframe::{
        i64_column_value, i64_list_column, i64_list_column_at, invalid_data, list_column_at,
        string_value,
    },
};

const COMMUNITY_REPORTS_CONTEXT: &str = "create_community_reports";
const NO_DESCRIPTION: &str = "No Description";

pub(crate) fn read_entity_context_rows(dataframe: &DataFrame) -> Result<Vec<EntityContextRow>> {
    let ids = dataframe.column("id")?.str()?;
    let titles = dataframe.column("title")?.str()?;
    let descriptions = dataframe.column("description")?;
    let mut rows = Vec::with_capacity(dataframe.height());
    for index in 0..dataframe.height() {
        rows.push(EntityContextRow {
            id: string_value(ids.get(index), "id", COMMUNITY_REPORTS_CONTEXT)?,
            human_readable_id: i64_column_value(
                dataframe,
                index,
                "human_readable_id",
                COMMUNITY_REPORTS_CONTEXT,
            )?,
            title: string_value(titles.get(index), "title", COMMUNITY_REPORTS_CONTEXT)?,
            description: optional_string(descriptions, index)?.unwrap_or_else(no_description),
            degree: i64_column_value(dataframe, index, "degree", COMMUNITY_REPORTS_CONTEXT)?,
        });
    }
    Ok(rows)
}

pub(crate) fn read_relationship_context_rows(
    dataframe: &DataFrame,
) -> Result<Vec<RelationshipContextRow>> {
    let ids = dataframe.column("id")?.str()?;
    let sources = dataframe.column("source")?.str()?;
    let targets = dataframe.column("target")?.str()?;
    let descriptions = dataframe.column("description")?;
    let mut rows = Vec::with_capacity(dataframe.height());
    for index in 0..dataframe.height() {
        rows.push(RelationshipContextRow {
            id: string_value(ids.get(index), "id", COMMUNITY_REPORTS_CONTEXT)?,
            human_readable_id: i64_column_value(
                dataframe,
                index,
                "human_readable_id",
                COMMUNITY_REPORTS_CONTEXT,
            )?,
            source: string_value(sources.get(index), "source", COMMUNITY_REPORTS_CONTEXT)?,
            target: string_value(targets.get(index), "target", COMMUNITY_REPORTS_CONTEXT)?,
            description: optional_string(descriptions, index)?.unwrap_or_else(no_description),
            combined_degree: i64_column_value(
                dataframe,
                index,
                "combined_degree",
                COMMUNITY_REPORTS_CONTEXT,
            )?,
        });
    }
    Ok(rows)
}

pub(crate) fn read_community_input_rows(dataframe: &DataFrame) -> Result<Vec<CommunityInputRow>> {
    let periods = dataframe.column("period")?.str()?;
    let mut rows = Vec::with_capacity(dataframe.height());
    for index in 0..dataframe.height() {
        rows.push(CommunityInputRow {
            community: i64_column_value(dataframe, index, "community", COMMUNITY_REPORTS_CONTEXT)?,
            level: i64_column_value(dataframe, index, "level", COMMUNITY_REPORTS_CONTEXT)?,
            parent: i64_column_value(dataframe, index, "parent", COMMUNITY_REPORTS_CONTEXT)?,
            children: i64_list_column_at(dataframe, index, "children", COMMUNITY_REPORTS_CONTEXT)?,
            entity_ids: list_column_at(dataframe, index, "entity_ids", COMMUNITY_REPORTS_CONTEXT)?,
            period: string_value(periods.get(index), "period", COMMUNITY_REPORTS_CONTEXT)?,
            size: i64_column_value(dataframe, index, "size", COMMUNITY_REPORTS_CONTEXT)?,
        });
    }
    Ok(rows)
}

pub(crate) fn read_claim_context_rows(dataframe: &DataFrame) -> Result<Vec<ClaimContextRow>> {
    let mut rows = Vec::with_capacity(dataframe.height());
    for index in 0..dataframe.height() {
        if let Some(subject_id) = optional_string(dataframe.column("subject_id")?, index)? {
            rows.push(ClaimContextRow {
                human_readable_id: i64_column_value(
                    dataframe,
                    index,
                    "human_readable_id",
                    COMMUNITY_REPORTS_CONTEXT,
                )?,
                subject_id,
                claim_type: optional_string(dataframe.column("type")?, index)?
                    .unwrap_or_else(String::new),
                status: optional_string(dataframe.column("status")?, index)?
                    .unwrap_or_else(String::new),
                description: optional_string(dataframe.column("description")?, index)?
                    .unwrap_or_else(no_description),
            });
        }
    }
    Ok(rows)
}

pub(crate) fn community_reports_dataframe(rows: &[CommunityReportRow]) -> Result<DataFrame> {
    let mut dataframe = df!(
        "id" => rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        "human_readable_id" => rows.iter().map(|row| row.human_readable_id).collect::<Vec<_>>(),
        "community" => rows.iter().map(|row| row.community).collect::<Vec<_>>(),
        "level" => rows.iter().map(|row| row.level).collect::<Vec<_>>(),
        "parent" => rows.iter().map(|row| row.parent).collect::<Vec<_>>(),
        "title" => rows.iter().map(|row| row.title.as_str()).collect::<Vec<_>>(),
        "summary" => rows.iter().map(|row| row.summary.as_str()).collect::<Vec<_>>(),
        "full_content" => rows.iter().map(|row| row.full_content.as_str()).collect::<Vec<_>>(),
        "rank" => rows.iter().map(|row| row.rank).collect::<Vec<_>>(),
        "rating_explanation" => rows.iter().map(|row| row.rating_explanation.as_str()).collect::<Vec<_>>(),
        "full_content_json" => rows.iter().map(|row| row.full_content_json.as_str()).collect::<Vec<_>>(),
        "period" => rows.iter().map(|row| row.period.as_str()).collect::<Vec<_>>(),
        "size" => rows.iter().map(|row| row.size).collect::<Vec<_>>(),
    )?;
    dataframe.insert_column(
        5,
        i64_list_column(
            "children",
            &rows
                .iter()
                .map(|row| row.children.clone())
                .collect::<Vec<_>>(),
        ),
    )?;
    dataframe.insert_column(
        11,
        findings_column(
            "findings",
            &rows
                .iter()
                .map(|row| row.findings.clone())
                .collect::<Vec<_>>(),
        )?,
    )?;
    Ok(dataframe)
}

pub(crate) fn community_report_value(row: &CommunityReportRow) -> Value {
    json!({
        "id": row.id,
        "human_readable_id": row.human_readable_id,
        "community": row.community,
        "level": row.level,
        "parent": row.parent,
        "children": row.children,
        "title": row.title,
        "summary": row.summary,
        "full_content": row.full_content,
        "rank": row.rank,
        "rating_explanation": row.rating_explanation,
        "findings": row.findings.iter().map(|finding| json!({
            "summary": finding.summary,
            "explanation": finding.explanation,
        })).collect::<Vec<_>>(),
        "full_content_json": row.full_content_json,
        "period": row.period,
        "size": row.size,
    })
}

fn findings_column(name: &str, rows: &[Vec<CommunityReportFindingRow>]) -> Result<Column> {
    if rows.is_empty() {
        return Ok(Series::new_empty(name.into(), &findings_dtype()).into());
    }
    let series_rows = rows
        .iter()
        .map(|findings| {
            let summaries = findings
                .iter()
                .map(|finding| finding.summary.as_str())
                .collect::<Vec<_>>();
            let explanations = findings
                .iter()
                .map(|finding| finding.explanation.as_str())
                .collect::<Vec<_>>();
            let fields = [
                Series::new("summary".into(), summaries),
                Series::new("explanation".into(), explanations),
            ];
            StructChunked::from_series("item".into(), findings.len(), fields.iter())
                .map(StructChunked::into_series)
        })
        .collect::<PolarsResult<Vec<_>>>()?;
    Ok(Series::new(name.into(), series_rows).into())
}

fn findings_dtype() -> DataType {
    DataType::List(Box::new(DataType::Struct(vec![
        Field::new("summary".into(), DataType::String),
        Field::new("explanation".into(), DataType::String),
    ])))
}

fn optional_string(column: &Column, index: usize) -> Result<Option<String>> {
    match column.get(index)? {
        AnyValue::String(value) => Ok(Some(value.to_owned())),
        AnyValue::StringOwned(value) => Ok(Some(value.to_string())),
        AnyValue::Null => Ok(None),
        _ => Err(invalid_data(
            COMMUNITY_REPORTS_CONTEXT,
            &format!("expected string column {}", column.name()),
        )),
    }
}

fn no_description() -> String {
    NO_DESCRIPTION.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_write_community_report_schema_in_graphrag_order() {
        let rows = vec![CommunityReportRow {
            id: "report-1".to_owned(),
            human_readable_id: 3,
            community: 3,
            level: 1,
            parent: 0,
            children: vec![4, 5],
            title: "Title".to_owned(),
            summary: "Summary".to_owned(),
            full_content: "# Title\n\nSummary".to_owned(),
            rank: 7.0,
            rating_explanation: "Reason".to_owned(),
            findings: vec![CommunityReportFindingRow {
                summary: "Finding".to_owned(),
                explanation: "Explanation".to_owned(),
            }],
            full_content_json: "{}".to_owned(),
            period: "2026-07-08".to_owned(),
            size: 2,
        }];

        let dataframe = community_reports_dataframe(&rows).expect("dataframe should build");

        assert_eq!(
            column_names(&dataframe),
            [
                "id",
                "human_readable_id",
                "community",
                "level",
                "parent",
                "children",
                "title",
                "summary",
                "full_content",
                "rank",
                "rating_explanation",
                "findings",
                "full_content_json",
                "period",
                "size",
            ]
        );
        assert_eq!(
            dataframe.column("human_readable_id").expect("hrid").dtype(),
            &DataType::Int64
        );
        assert_eq!(
            dataframe.column("community").expect("community").dtype(),
            &DataType::Int64
        );
        assert_eq!(
            dataframe.column("level").expect("level").dtype(),
            &DataType::Int64
        );
        assert_eq!(
            dataframe.column("parent").expect("parent").dtype(),
            &DataType::Int64
        );
        assert_eq!(
            dataframe.column("children").expect("children").dtype(),
            &DataType::List(Box::new(DataType::Int64))
        );
        assert_eq!(
            dataframe.column("rank").expect("rank").dtype(),
            &DataType::Float64
        );
        assert_eq!(
            dataframe.column("findings").expect("findings").dtype(),
            &findings_dtype()
        );
        assert_eq!(
            dataframe.column("size").expect("size").dtype(),
            &DataType::Int64
        );
    }

    fn column_names(dataframe: &DataFrame) -> Vec<&str> {
        dataframe
            .get_column_names()
            .into_iter()
            .map(PlSmallStr::as_str)
            .collect()
    }
}
