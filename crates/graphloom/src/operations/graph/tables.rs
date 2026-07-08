//! Graph operation table encoding and decoding.

use polars_core::prelude::*;
use serde_json::{Value, json};

use super::{
    FinalEntityRow, FinalRelationshipRow, RawEntityRow, RawRelationshipRow, SummarizedEntityRow,
    SummarizedRelationshipRow, TextUnitInput,
};
use crate::{
    GraphLoomError, Result,
    workflows::{
        EXTRACT_GRAPH_WORKFLOW, FINALIZE_GRAPH_WORKFLOW,
        common::{
            f64_at, i64_at, list_at, list_column, row_to_static, string_at,
            string_list_or_string_at,
        },
    },
};

pub(crate) fn read_text_units(dataframe: &DataFrame) -> Result<Vec<TextUnitInput>> {
    let ids = dataframe.column("id")?.str()?;
    let texts = dataframe.column("text")?.str()?;
    let mut rows = Vec::with_capacity(dataframe.height());
    for index in 0..dataframe.height() {
        rows.push(TextUnitInput {
            id: ids
                .get(index)
                .ok_or_else(|| invalid_data("missing text unit id"))?
                .to_owned(),
            text: texts
                .get(index)
                .ok_or_else(|| invalid_data("missing text unit text"))?
                .to_owned(),
        });
    }
    Ok(rows)
}

pub(crate) fn read_entity_rows(dataframe: &DataFrame) -> Result<Vec<SummarizedEntityRow>> {
    let mut rows = Vec::with_capacity(dataframe.height());
    for row_index in 0..dataframe.height() {
        let row = row_to_static(dataframe.get_row(row_index)?);
        rows.push(SummarizedEntityRow {
            title: string_at(&row, 0, "title", FINALIZE_GRAPH_WORKFLOW)?,
            entity_type: string_at(&row, 1, "type", FINALIZE_GRAPH_WORKFLOW)?,
            description: string_list_or_string_at(&row, 2, FINALIZE_GRAPH_WORKFLOW).join("\n"),
            text_unit_ids: list_at(&row, 3, FINALIZE_GRAPH_WORKFLOW)?,
            frequency: i64_at(&row, 4, "frequency", FINALIZE_GRAPH_WORKFLOW)?,
        });
    }
    Ok(rows)
}

pub(crate) fn read_relationship_rows(
    dataframe: &DataFrame,
) -> Result<Vec<SummarizedRelationshipRow>> {
    let mut rows = Vec::with_capacity(dataframe.height());
    for row_index in 0..dataframe.height() {
        let row = row_to_static(dataframe.get_row(row_index)?);
        rows.push(SummarizedRelationshipRow {
            source: string_at(&row, 0, "source", FINALIZE_GRAPH_WORKFLOW)?,
            target: string_at(&row, 1, "target", FINALIZE_GRAPH_WORKFLOW)?,
            description: string_list_or_string_at(&row, 2, FINALIZE_GRAPH_WORKFLOW).join("\n"),
            text_unit_ids: list_at(&row, 3, FINALIZE_GRAPH_WORKFLOW)?,
            weight: f64_at(&row, 4, "weight", FINALIZE_GRAPH_WORKFLOW)?,
        });
    }
    Ok(rows)
}

pub(crate) fn raw_entity_dataframe(rows: &[RawEntityRow]) -> Result<DataFrame> {
    Ok(df!(
        "title" => rows.iter().map(|row| row.title.as_str()).collect::<Vec<_>>(),
        "type" => rows.iter().map(|row| row.entity_type.as_str()).collect::<Vec<_>>(),
        "description" => rows.iter().map(|row| row.description.as_str()).collect::<Vec<_>>(),
        "source_id" => rows.iter().map(|row| row.source_id.as_str()).collect::<Vec<_>>(),
    )?)
}

pub(crate) fn raw_relationship_dataframe(rows: &[RawRelationshipRow]) -> Result<DataFrame> {
    Ok(df!(
        "source" => rows.iter().map(|row| row.source.as_str()).collect::<Vec<_>>(),
        "target" => rows.iter().map(|row| row.target.as_str()).collect::<Vec<_>>(),
        "weight" => rows.iter().map(|row| row.weight).collect::<Vec<_>>(),
        "description" => rows.iter().map(|row| row.description.as_str()).collect::<Vec<_>>(),
        "source_id" => rows.iter().map(|row| row.source_id.as_str()).collect::<Vec<_>>(),
    )?)
}

pub(crate) fn entity_intermediate_dataframe(rows: &[SummarizedEntityRow]) -> Result<DataFrame> {
    let mut dataframe = df!(
        "title" => rows.iter().map(|row| row.title.as_str()).collect::<Vec<_>>(),
        "type" => rows.iter().map(|row| row.entity_type.as_str()).collect::<Vec<_>>(),
        "description" => rows.iter().map(|row| row.description.as_str()).collect::<Vec<_>>(),
        "frequency" => rows.iter().map(|row| row.frequency).collect::<Vec<_>>(),
    )?;
    dataframe.insert_column(
        3,
        list_column(
            "text_unit_ids",
            &rows
                .iter()
                .map(|row| row.text_unit_ids.clone())
                .collect::<Vec<_>>(),
        )?,
    )?;
    Ok(dataframe)
}

pub(crate) fn relationship_intermediate_dataframe(
    rows: &[SummarizedRelationshipRow],
) -> Result<DataFrame> {
    let mut dataframe = df!(
        "source" => rows.iter().map(|row| row.source.as_str()).collect::<Vec<_>>(),
        "target" => rows.iter().map(|row| row.target.as_str()).collect::<Vec<_>>(),
        "description" => rows.iter().map(|row| row.description.as_str()).collect::<Vec<_>>(),
        "weight" => rows.iter().map(|row| row.weight).collect::<Vec<_>>(),
    )?;
    dataframe.insert_column(
        3,
        list_column(
            "text_unit_ids",
            &rows
                .iter()
                .map(|row| row.text_unit_ids.clone())
                .collect::<Vec<_>>(),
        )?,
    )?;
    Ok(dataframe)
}

pub(crate) fn final_entities_dataframe(rows: &[FinalEntityRow]) -> Result<DataFrame> {
    let mut dataframe = df!(
        "id" => rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        "human_readable_id" => rows.iter().map(|row| row.human_readable_id as u64).collect::<Vec<_>>(),
        "title" => rows.iter().map(|row| row.title.as_str()).collect::<Vec<_>>(),
        "type" => rows.iter().map(|row| row.entity_type.as_str()).collect::<Vec<_>>(),
        "description" => rows.iter().map(|row| row.description.as_str()).collect::<Vec<_>>(),
        "frequency" => rows.iter().map(|row| row.frequency).collect::<Vec<_>>(),
        "degree" => rows.iter().map(|row| row.degree).collect::<Vec<_>>(),
    )?;
    dataframe.insert_column(
        5,
        list_column(
            "text_unit_ids",
            &rows
                .iter()
                .map(|row| row.text_unit_ids.clone())
                .collect::<Vec<_>>(),
        )?,
    )?;
    Ok(dataframe)
}

pub(crate) fn final_relationships_dataframe(rows: &[FinalRelationshipRow]) -> Result<DataFrame> {
    let mut dataframe = df!(
        "id" => rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        "human_readable_id" => rows.iter().map(|row| row.human_readable_id as u64).collect::<Vec<_>>(),
        "source" => rows.iter().map(|row| row.source.as_str()).collect::<Vec<_>>(),
        "target" => rows.iter().map(|row| row.target.as_str()).collect::<Vec<_>>(),
        "description" => rows.iter().map(|row| row.description.as_str()).collect::<Vec<_>>(),
        "weight" => rows.iter().map(|row| row.weight).collect::<Vec<_>>(),
        "combined_degree" => rows.iter().map(|row| row.combined_degree).collect::<Vec<_>>(),
    )?;
    dataframe.with_column(list_column(
        "text_unit_ids",
        &rows
            .iter()
            .map(|row| row.text_unit_ids.clone())
            .collect::<Vec<_>>(),
    )?)?;
    Ok(dataframe)
}

pub(crate) fn extract_graph_sample(
    entities: &[SummarizedEntityRow],
    relationships: &[SummarizedRelationshipRow],
) -> Vec<Value> {
    vec![
        json!({"entities": entities.iter().take(5).map(entity_value).collect::<Vec<_>>()}),
        json!({"relationships": relationships.iter().take(5).map(relationship_value).collect::<Vec<_>>()}),
    ]
}

pub(crate) fn finalize_graph_sample(
    entities: &[FinalEntityRow],
    relationships: &[FinalRelationshipRow],
) -> Vec<Value> {
    vec![
        json!({"entities": entities.iter().take(5).map(final_entity_value).collect::<Vec<_>>()}),
        json!({"relationships": relationships.iter().take(5).map(final_relationship_value).collect::<Vec<_>>()}),
    ]
}

fn entity_value(row: &SummarizedEntityRow) -> Value {
    json!({
        "title": row.title,
        "type": row.entity_type,
        "description": row.description,
        "text_unit_ids": row.text_unit_ids,
        "frequency": row.frequency,
    })
}

fn relationship_value(row: &SummarizedRelationshipRow) -> Value {
    json!({
        "source": row.source,
        "target": row.target,
        "description": row.description,
        "text_unit_ids": row.text_unit_ids,
        "weight": row.weight,
    })
}

fn final_entity_value(row: &FinalEntityRow) -> Value {
    json!({
        "id": row.id,
        "human_readable_id": row.human_readable_id,
        "title": row.title,
        "type": row.entity_type,
        "description": row.description,
        "text_unit_ids": row.text_unit_ids,
        "frequency": row.frequency,
        "degree": row.degree,
    })
}

fn final_relationship_value(row: &FinalRelationshipRow) -> Value {
    json!({
        "id": row.id,
        "human_readable_id": row.human_readable_id,
        "source": row.source,
        "target": row.target,
        "description": row.description,
        "weight": row.weight,
        "combined_degree": row.combined_degree,
        "text_unit_ids": row.text_unit_ids,
    })
}

fn invalid_data(message: &str) -> GraphLoomError {
    GraphLoomError::InvalidData {
        workflow: EXTRACT_GRAPH_WORKFLOW,
        message: message.to_owned(),
    }
}
