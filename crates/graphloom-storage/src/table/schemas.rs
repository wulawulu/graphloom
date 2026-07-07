use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Schema, SchemaRef};

fn string_list_field(name: &str) -> Field {
    Field::new(
        name,
        DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
        false,
    )
}

/// Final `documents` table schema.
#[must_use]
pub fn documents() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("human_readable_id", DataType::Int64, false),
        Field::new("title", DataType::Utf8, true),
        Field::new("text", DataType::Utf8, false),
        string_list_field("text_unit_ids"),
        Field::new("creation_date", DataType::Utf8, true),
        Field::new("raw_data", DataType::Utf8, true),
    ]))
}

/// Final `text_units` table schema.
#[must_use]
pub fn text_units() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("human_readable_id", DataType::Int64, false),
        Field::new("text", DataType::Utf8, false),
        Field::new("n_tokens", DataType::Int64, false),
        Field::new("document_id", DataType::Utf8, false),
        string_list_field("entity_ids"),
        string_list_field("relationship_ids"),
        string_list_field("covariate_ids"),
    ]))
}

/// Final `entities` table schema.
#[must_use]
pub fn entities() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("human_readable_id", DataType::Int64, false),
        Field::new("title", DataType::Utf8, false),
        Field::new("type", DataType::Utf8, false),
        Field::new("description", DataType::Utf8, false),
        string_list_field("text_unit_ids"),
        Field::new("frequency", DataType::Int64, false),
        Field::new("degree", DataType::Int64, false),
    ]))
}

/// Final `relationships` table schema.
#[must_use]
pub fn relationships() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("human_readable_id", DataType::Int64, false),
        Field::new("source", DataType::Utf8, false),
        Field::new("target", DataType::Utf8, false),
        Field::new("description", DataType::Utf8, false),
        Field::new("weight", DataType::Float64, false),
        Field::new("combined_degree", DataType::Int64, false),
        string_list_field("text_unit_ids"),
    ]))
}
