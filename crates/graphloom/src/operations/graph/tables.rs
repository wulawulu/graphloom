//! Graph operation table encoding and decoding.

use polars_core::prelude::*;
use serde_json::{Value, json};

use super::{
    EntityRow, FinalEntityRow, FinalRelationshipRow, RelationshipRow, SummarizedEntityRow,
    SummarizedRelationshipRow, TextUnitInput,
};
use crate::{
    Result,
    dataframe::{
        f64_column_value, i64_column_value, invalid_data, list_column, list_column_at,
        string_list_or_string_column_at, string_value,
    },
};

const EXTRACT_GRAPH_CONTEXT: &str = "extract_graph";
const FINALIZE_GRAPH_CONTEXT: &str = "finalize_graph";

pub(crate) fn read_text_units(dataframe: &DataFrame) -> Result<Vec<TextUnitInput>> {
    let ids = dataframe.column("id")?.str()?;
    let texts = dataframe.column("text")?.str()?;
    let mut rows = Vec::with_capacity(dataframe.height());
    for index in 0..dataframe.height() {
        rows.push(TextUnitInput {
            id: ids
                .get(index)
                .ok_or_else(|| invalid_data(EXTRACT_GRAPH_CONTEXT, "missing text unit id"))?
                .to_owned(),
            text: texts
                .get(index)
                .ok_or_else(|| invalid_data(EXTRACT_GRAPH_CONTEXT, "missing text unit text"))?
                .to_owned(),
        });
    }
    Ok(rows)
}

pub(crate) fn read_entity_rows(dataframe: &DataFrame) -> Result<Vec<SummarizedEntityRow>> {
    let titles = dataframe.column("title")?.str()?;
    let entity_types = dataframe.column("type")?.str()?;
    let mut rows = Vec::with_capacity(dataframe.height());
    for row_index in 0..dataframe.height() {
        rows.push(SummarizedEntityRow {
            title: string_value(titles.get(row_index), "title", FINALIZE_GRAPH_CONTEXT)?,
            entity_type: string_value(entity_types.get(row_index), "type", FINALIZE_GRAPH_CONTEXT)?,
            description: string_list_or_string_column_at(
                dataframe,
                row_index,
                "description",
                FINALIZE_GRAPH_CONTEXT,
            )?
            .join("\n"),
            text_unit_ids: list_column_at(
                dataframe,
                row_index,
                "text_unit_ids",
                FINALIZE_GRAPH_CONTEXT,
            )?,
            frequency: i64_column_value(dataframe, row_index, "frequency", FINALIZE_GRAPH_CONTEXT)?,
        });
    }
    Ok(rows)
}

pub(crate) fn read_relationship_rows(
    dataframe: &DataFrame,
) -> Result<Vec<SummarizedRelationshipRow>> {
    let sources = dataframe.column("source")?.str()?;
    let targets = dataframe.column("target")?.str()?;
    let mut rows = Vec::with_capacity(dataframe.height());
    for row_index in 0..dataframe.height() {
        rows.push(SummarizedRelationshipRow {
            source: string_value(sources.get(row_index), "source", FINALIZE_GRAPH_CONTEXT)?,
            target: string_value(targets.get(row_index), "target", FINALIZE_GRAPH_CONTEXT)?,
            description: string_list_or_string_column_at(
                dataframe,
                row_index,
                "description",
                FINALIZE_GRAPH_CONTEXT,
            )?
            .join("\n"),
            text_unit_ids: list_column_at(
                dataframe,
                row_index,
                "text_unit_ids",
                FINALIZE_GRAPH_CONTEXT,
            )?,
            weight: f64_column_value(dataframe, row_index, "weight", FINALIZE_GRAPH_CONTEXT)?,
        });
    }
    Ok(rows)
}

pub(crate) fn raw_entity_dataframe(rows: &[EntityRow]) -> Result<DataFrame> {
    let mut dataframe = df!(
        "title" => rows.iter().map(|row| row.title.as_str()).collect::<Vec<_>>(),
        "type" => rows.iter().map(|row| row.entity_type.as_str()).collect::<Vec<_>>(),
        "frequency" => rows.iter().map(|row| row.frequency).collect::<Vec<_>>(),
    )?;
    dataframe.insert_column(
        2,
        list_column(
            "description",
            &rows
                .iter()
                .map(|row| row.description.clone())
                .collect::<Vec<_>>(),
        )?,
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

pub(crate) fn raw_relationship_dataframe(rows: &[RelationshipRow]) -> Result<DataFrame> {
    let mut dataframe = df!(
        "source" => rows.iter().map(|row| row.source.as_str()).collect::<Vec<_>>(),
        "target" => rows.iter().map(|row| row.target.as_str()).collect::<Vec<_>>(),
        "weight" => rows.iter().map(|row| row.weight).collect::<Vec<_>>(),
    )?;
    dataframe.insert_column(
        2,
        list_column(
            "description",
            &rows
                .iter()
                .map(|row| row.description.clone())
                .collect::<Vec<_>>(),
        )?,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_read_entity_rows_by_column_name() {
        let mut dataframe = df!(
            "frequency" => [2i64],
            "description" => ["first summary"],
            "type" => ["person"],
            "title" => ["ALICE"],
        )
        .expect("dataframe should build");
        dataframe
            .with_column(
                list_column(
                    "text_unit_ids",
                    &[vec!["tu-1".to_owned(), "tu-2".to_owned()]],
                )
                .expect("list column should build"),
            )
            .expect("column should append");

        let rows = read_entity_rows(&dataframe).expect("entity rows should decode");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "ALICE");
        assert_eq!(rows[0].entity_type, "person");
        assert_eq!(rows[0].text_unit_ids, vec!["tu-1", "tu-2"]);
        assert_eq!(rows[0].frequency, 2);
    }

    #[test]
    fn test_should_error_on_wrong_entity_frequency_type() {
        let mut dataframe = df!(
            "title" => ["ALICE"],
            "type" => ["person"],
            "description" => ["summary"],
            "frequency" => ["not-an-int"],
        )
        .expect("dataframe should build");
        dataframe
            .with_column(
                list_column("text_unit_ids", &[vec!["tu-1".to_owned()]])
                    .expect("list column should build"),
            )
            .expect("column should append");

        let error = read_entity_rows(&dataframe).expect_err("frequency type should fail");

        assert!(error.to_string().contains("frequency"));
    }

    #[test]
    fn test_should_write_raw_graph_after_merge_schema() {
        let entities = vec![EntityRow {
            title: "ALICE".to_owned(),
            entity_type: "person".to_owned(),
            description: vec!["engineer".to_owned(), "mentor".to_owned()],
            text_unit_ids: vec!["tu-1".to_owned(), "tu-2".to_owned()],
            frequency: 2,
        }];
        let relationships = vec![RelationshipRow {
            source: "ALICE".to_owned(),
            target: "BOB".to_owned(),
            description: vec!["works with".to_owned()],
            text_unit_ids: vec!["tu-1".to_owned()],
            weight: 3.0,
        }];

        let entity_frame = raw_entity_dataframe(&entities).expect("raw entities should build");
        let relationship_frame =
            raw_relationship_dataframe(&relationships).expect("raw relationships should build");

        assert!(entity_frame.column("frequency").is_ok());
        assert!(entity_frame.column("text_unit_ids").is_ok());
        assert!(entity_frame.column("source_id").is_err());
        assert!(relationship_frame.column("text_unit_ids").is_ok());
        assert!(relationship_frame.column("source_id").is_err());
    }
}
