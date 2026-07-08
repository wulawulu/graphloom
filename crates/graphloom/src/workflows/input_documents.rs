//! `load_input_documents` workflow.

use async_trait::async_trait;
use futures_util::TryStreamExt;
use polars_core::prelude::*;
use serde_json::{Map, Value, json};

use crate::{
    GraphLoomError, GraphRagConfig, PipelineRunContext, Result, Workflow, WorkflowFunctionOutput,
};

/// Workflow name.
pub const LOAD_INPUT_DOCUMENTS_WORKFLOW: &str = "load_input_documents";

/// Load input documents into the `documents` table.
#[derive(Debug, Clone, Copy, Default)]
pub struct LoadInputDocumentsWorkflow;

#[async_trait]
impl Workflow for LoadInputDocumentsWorkflow {
    fn name(&self) -> &'static str {
        LOAD_INPUT_DOCUMENTS_WORKFLOW
    }

    async fn run(
        &self,
        _config: &GraphRagConfig,
        context: &mut PipelineRunContext,
    ) -> Result<WorkflowFunctionOutput> {
        let reader = context
            .input_reader
            .as_ref()
            .ok_or(GraphLoomError::MissingProvider {
                name: "input_reader",
            })?;
        let mut stream = reader.read_documents();
        let mut table = context
            .output_table_provider
            .open("documents", true)
            .await?;
        let mut rows = Vec::new();
        let mut sample = Vec::new();

        while let Some(document) = stream.try_next().await? {
            let raw_data = document
                .raw_data
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            let row = DocumentRow {
                id: document.id,
                human_readable_id: rows.len(),
                title: Some(document.title),
                text: document.text,
                text_unit_ids: Vec::new(),
                creation_date: document.creation_date,
                raw_data,
            };
            if sample.len() < 5 {
                sample.push(row.to_value());
            }
            rows.push(row);
            context
                .callbacks
                .progress(LOAD_INPUT_DOCUMENTS_WORKFLOW, rows.len(), None);
        }

        if rows.is_empty() {
            table.abort().await?;
            return Err(GraphLoomError::InvalidData {
                workflow: LOAD_INPUT_DOCUMENTS_WORKFLOW,
                message: "no input documents were read".to_owned(),
            });
        }

        table.write(documents_dataframe(&rows)?).await?;
        table.close().await?;
        context.stats.document_count = rows.len();
        Ok(WorkflowFunctionOutput {
            result: sample,
            stop: false,
            input_rows: rows.len(),
            output_rows: rows.len(),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DocumentRow {
    pub(crate) id: String,
    pub(crate) human_readable_id: usize,
    pub(crate) title: Option<String>,
    pub(crate) text: String,
    pub(crate) text_unit_ids: Vec<String>,
    pub(crate) creation_date: Option<String>,
    pub(crate) raw_data: Option<String>,
}

impl DocumentRow {
    pub(crate) fn to_value(&self) -> Value {
        let mut object = Map::new();
        object.insert("id".to_owned(), Value::String(self.id.clone()));
        object.insert(
            "human_readable_id".to_owned(),
            json!(self.human_readable_id),
        );
        object.insert(
            "title".to_owned(),
            self.title.clone().map(Value::String).unwrap_or(Value::Null),
        );
        object.insert("text".to_owned(), Value::String(self.text.clone()));
        object.insert("text_unit_ids".to_owned(), json!(self.text_unit_ids));
        object.insert(
            "creation_date".to_owned(),
            self.creation_date
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        );
        object.insert(
            "raw_data".to_owned(),
            self.raw_data
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        );
        Value::Object(object)
    }
}

pub(crate) fn documents_dataframe(rows: &[DocumentRow]) -> Result<DataFrame> {
    let ids = rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>();
    let human_ids = rows
        .iter()
        .map(|row| row.human_readable_id as u64)
        .collect::<Vec<_>>();
    let titles = rows
        .iter()
        .map(|row| row.title.as_deref())
        .collect::<Vec<_>>();
    let texts = rows.iter().map(|row| row.text.as_str()).collect::<Vec<_>>();
    let creation_dates = rows
        .iter()
        .map(|row| row.creation_date.as_deref())
        .collect::<Vec<_>>();
    let raw_data = rows
        .iter()
        .map(|row| row.raw_data.as_deref())
        .collect::<Vec<_>>();

    let mut dataframe = df!(
        "id" => ids,
        "human_readable_id" => human_ids,
        "title" => titles,
        "text" => texts,
        "creation_date" => creation_dates,
        "raw_data" => raw_data,
    )?;
    dataframe.insert_column(
        4,
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

pub(crate) fn list_column(name: &str, rows: &[Vec<String>]) -> Result<Column> {
    let series_rows = rows
        .iter()
        .map(|values| {
            let refs = values.iter().map(String::as_str).collect::<Vec<_>>();
            Series::new("item".into(), refs)
        })
        .collect::<Vec<_>>();
    Ok(Series::new(name.into(), series_rows).into())
}
