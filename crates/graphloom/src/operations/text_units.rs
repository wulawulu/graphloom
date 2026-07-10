//! Text-unit domain rows and table codecs.

use polars_core::prelude::*;
use serde_json::{Map, Value, json};

use crate::{Result, dataframe::list_column};

#[derive(Debug, Clone)]
pub(crate) struct TextUnitRow {
    pub(crate) id: String,
    pub(crate) human_readable_id: i64,
    pub(crate) text: String,
    pub(crate) n_tokens: i64,
    pub(crate) document_id: String,
    pub(crate) entity_ids: Vec<String>,
    pub(crate) relationship_ids: Vec<String>,
    pub(crate) covariate_ids: Vec<String>,
}

impl TextUnitRow {
    pub(crate) fn to_value(&self) -> Value {
        let mut object = Map::new();
        object.insert("id".to_owned(), Value::String(self.id.clone()));
        object.insert(
            "human_readable_id".to_owned(),
            json!(self.human_readable_id),
        );
        object.insert("text".to_owned(), Value::String(self.text.clone()));
        object.insert("n_tokens".to_owned(), json!(self.n_tokens));
        object.insert(
            "document_id".to_owned(),
            Value::String(self.document_id.clone()),
        );
        object.insert("entity_ids".to_owned(), json!(self.entity_ids));
        object.insert("relationship_ids".to_owned(), json!(self.relationship_ids));
        object.insert("covariate_ids".to_owned(), json!(self.covariate_ids));
        Value::Object(object)
    }
}

pub(crate) fn text_units_dataframe(rows: &[TextUnitRow]) -> Result<DataFrame> {
    let ids = rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>();
    let human_ids = rows
        .iter()
        .map(|row| row.human_readable_id)
        .collect::<Vec<_>>();
    let texts = rows.iter().map(|row| row.text.as_str()).collect::<Vec<_>>();
    let n_tokens = rows.iter().map(|row| row.n_tokens).collect::<Vec<_>>();
    let document_ids = rows
        .iter()
        .map(|row| row.document_id.as_str())
        .collect::<Vec<_>>();
    let mut dataframe = df!(
        "id" => ids,
        "human_readable_id" => human_ids,
        "text" => texts,
        "n_tokens" => n_tokens,
        "document_id" => document_ids,
    )?;
    dataframe.with_column(list_column(
        "entity_ids",
        &rows
            .iter()
            .map(|row| row.entity_ids.clone())
            .collect::<Vec<_>>(),
    ))?;
    dataframe.with_column(list_column(
        "relationship_ids",
        &rows
            .iter()
            .map(|row| row.relationship_ids.clone())
            .collect::<Vec<_>>(),
    ))?;
    dataframe.with_column(list_column(
        "covariate_ids",
        &rows
            .iter()
            .map(|row| row.covariate_ids.clone())
            .collect::<Vec<_>>(),
    ))?;
    Ok(dataframe)
}

#[cfg(test)]
mod tests {
    use polars_core::prelude::*;

    use super::*;

    #[test]
    fn test_should_write_text_unit_numeric_schema_as_signed_int64() {
        let rows = vec![TextUnitRow {
            id: "tu-1".to_owned(),
            human_readable_id: 0,
            text: "Alice reports Bob.".to_owned(),
            n_tokens: 4,
            document_id: "doc-1".to_owned(),
            entity_ids: Vec::new(),
            relationship_ids: Vec::new(),
            covariate_ids: Vec::new(),
        }];

        let dataframe = text_units_dataframe(&rows).expect("dataframe should build");

        assert_eq!(
            dataframe
                .column("human_readable_id")
                .expect("human_readable_id")
                .dtype(),
            &DataType::Int64
        );
        assert_eq!(
            dataframe.column("n_tokens").expect("n_tokens").dtype(),
            &DataType::Int64
        );
    }
}
