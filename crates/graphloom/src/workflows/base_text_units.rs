//! `create_base_text_units` workflow.

use std::sync::Arc;

use async_trait::async_trait;
use futures_util::StreamExt;
use graphloom_chunking::{MetadataTransform, TextTransform, add_metadata, create_chunker};
use graphloom_input::{TextDocument, gen_sha512_hash};
use graphloom_llm::{TiktokenTokenizer, Tokenizer};
use polars_core::{frame::row::Row, prelude::*};
use serde_json::{Map, Value, json};

use super::input_documents::{list_column, usize_to_i64};
use crate::{
    GraphLoomError, GraphRagConfig, PipelineRunContext, Result, Workflow, WorkflowFunctionOutput,
};

/// Workflow name.
pub const CREATE_BASE_TEXT_UNITS_WORKFLOW: &str = "create_base_text_units";

/// Create base text units from documents.
#[derive(Debug, Clone, Copy, Default)]
pub struct CreateBaseTextUnitsWorkflow;

#[async_trait]
impl Workflow for CreateBaseTextUnitsWorkflow {
    fn name(&self) -> &'static str {
        CREATE_BASE_TEXT_UNITS_WORKFLOW
    }

    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut PipelineRunContext,
    ) -> Result<WorkflowFunctionOutput> {
        let tokenizer = Arc::new(TiktokenTokenizer::new(&config.chunking.encoding_model)?);
        let chunker = create_chunker(&config.chunking)?;

        let mut documents = context
            .output_table_provider
            .open("documents", false)
            .await?;
        let input_rows = documents.length();
        let mut text_units = context
            .output_table_provider
            .open("text_units", true)
            .await?;
        let mut rows = Vec::new();
        let mut sample = Vec::new();

        let mut document_rows = documents.rows();
        while let Some(row) = document_rows.next().await {
            let row = row?;
            let document = document_from_row(&row)?;
            let transform = metadata_transform(&document, &config.chunking.prepend_metadata);
            let transform_fn: Option<Box<TextTransform>> = transform.map(|transform| {
                Box::new(move |text: &str| transform.transform(text)) as Box<TextTransform>
            });
            let transform_ref = transform_fn.as_deref();
            for chunk in chunker.chunk(&document.text, transform_ref)? {
                let n_tokens = tokenizer.count(&chunk.text)?;
                let row = TextUnitRow {
                    id: gen_sha512_hash([chunk.text.as_str()]),
                    human_readable_id: usize_to_i64(rows.len(), CREATE_BASE_TEXT_UNITS_WORKFLOW)?,
                    text: chunk.text,
                    n_tokens: usize_to_i64(n_tokens, CREATE_BASE_TEXT_UNITS_WORKFLOW)?,
                    document_id: document.id.clone(),
                    entity_ids: Vec::new(),
                    relationship_ids: Vec::new(),
                    covariate_ids: Vec::new(),
                };
                if sample.len() < 5 {
                    sample.push(row.to_value());
                }
                rows.push(row);
            }
            context.callbacks.progress(
                CREATE_BASE_TEXT_UNITS_WORKFLOW,
                rows.len(),
                Some(input_rows),
            );
        }

        if !rows.is_empty() {
            text_units.write(text_units_dataframe(&rows)?).await?;
        }
        text_units.close().await?;
        context.stats.text_unit_count = rows.len();
        Ok(WorkflowFunctionOutput {
            result: sample,
            stop: false,
            input_rows,
            output_rows: rows.len(),
        })
    }
}

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
    )?)?;
    dataframe.with_column(list_column(
        "relationship_ids",
        &rows
            .iter()
            .map(|row| row.relationship_ids.clone())
            .collect::<Vec<_>>(),
    )?)?;
    dataframe.with_column(list_column(
        "covariate_ids",
        &rows
            .iter()
            .map(|row| row.covariate_ids.clone())
            .collect::<Vec<_>>(),
    )?)?;
    Ok(dataframe)
}

fn metadata_transform(
    document: &TextDocument,
    prepend_metadata: &[String],
) -> Option<MetadataTransform> {
    if prepend_metadata.is_empty() {
        None
    } else {
        Some(add_metadata(
            &document.collect(prepend_metadata),
            ": ",
            ".\n",
            false,
        ))
    }
}

fn document_from_row(row: &Row<'static>) -> Result<TextDocument> {
    Ok(TextDocument::new(
        string_at(row, 0, "id")?,
        string_at(row, 3, "text")?,
        optional_string_at(row, 2).unwrap_or_default(),
        optional_string_at(row, 5),
        optional_string_at(row, 6)
            .map(|raw| serde_json::from_str(&raw))
            .transpose()?,
    ))
}

pub(crate) fn string_at(row: &Row<'static>, index: usize, column: &'static str) -> Result<String> {
    row.0
        .get(index)
        .and_then(any_value_to_string)
        .ok_or_else(|| GraphLoomError::InvalidData {
            workflow: CREATE_BASE_TEXT_UNITS_WORKFLOW,
            message: format!("missing string column {column}"),
        })
}

pub(crate) fn optional_string_at(row: &Row<'static>, index: usize) -> Option<String> {
    row.0.get(index).and_then(any_value_to_string)
}

fn any_value_to_string(value: &AnyValue<'_>) -> Option<String> {
    match value {
        AnyValue::String(value) => Some((*value).to_owned()),
        AnyValue::StringOwned(value) => Some(value.to_string()),
        AnyValue::Null => None,
        _ => None,
    }
}
