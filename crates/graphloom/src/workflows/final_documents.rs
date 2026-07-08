//! `create_final_documents` workflow.

use std::collections::BTreeMap;

use async_trait::async_trait;
use polars_core::{
    frame::row::Row,
    prelude::{AnyValue, DataFrame},
};

use super::{
    base_text_units::{optional_string_at, string_at},
    input_documents::{DocumentRow, documents_dataframe},
};
use crate::{
    GraphLoomError, GraphRagConfig, PipelineRunContext, Result, Workflow, WorkflowFunctionOutput,
};

/// Workflow name.
pub const CREATE_FINAL_DOCUMENTS_WORKFLOW: &str = "create_final_documents";

/// Populate final `documents.text_unit_ids`.
#[derive(Debug, Clone, Copy, Default)]
pub struct CreateFinalDocumentsWorkflow;

#[async_trait]
impl Workflow for CreateFinalDocumentsWorkflow {
    fn name(&self) -> &'static str {
        CREATE_FINAL_DOCUMENTS_WORKFLOW
    }

    async fn run(
        &self,
        _config: &GraphRagConfig,
        context: &mut PipelineRunContext,
    ) -> Result<WorkflowFunctionOutput> {
        let text_units = context
            .output_table_provider
            .read_dataframe("text_units")
            .await?;
        let mapping = text_unit_mapping(&text_units)?;
        let documents = context
            .output_table_provider
            .read_dataframe("documents")
            .await?;
        let mut rows = Vec::with_capacity(documents.height());
        let mut sample = Vec::new();

        for row_index in 0..documents.height() {
            let row = row_to_static(documents.get_row(row_index)?);
            let document_id = string_at(&row, 0, "id")?;
            let document = DocumentRow {
                id: document_id.clone(),
                human_readable_id: row_index,
                title: optional_string_at(&row, 2),
                text: string_at(&row, 3, "text")?,
                text_unit_ids: mapping.get(&document_id).cloned().unwrap_or_default(),
                creation_date: optional_string_at(&row, 5),
                raw_data: optional_string_at(&row, 6),
            };
            if sample.len() < 5 {
                sample.push(document.to_value());
            }
            rows.push(document);
        }

        context
            .output_table_provider
            .write_dataframe("documents", documents_dataframe(&rows)?)
            .await?;

        Ok(WorkflowFunctionOutput {
            result: sample,
            stop: false,
            input_rows: documents.height().saturating_add(text_units.height()),
            output_rows: rows.len(),
        })
    }
}

fn text_unit_mapping(dataframe: &DataFrame) -> Result<BTreeMap<String, Vec<String>>> {
    let mut mapping: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row_index in 0..dataframe.height() {
        let row = row_to_static(dataframe.get_row(row_index)?);
        let text_unit_id = string_at_for_workflow(&row, 0, "id")?;
        let document_id = string_at_for_workflow(&row, 4, "document_id")?;
        mapping.entry(document_id).or_default().push(text_unit_id);
    }
    Ok(mapping)
}

fn row_to_static(row: Row<'_>) -> Row<'static> {
    Row::new(row.0.into_iter().map(AnyValue::into_static).collect())
}

fn string_at_for_workflow(
    row: &Row<'static>,
    index: usize,
    column: &'static str,
) -> Result<String> {
    string_at(row, index, column).map_err(|source| GraphLoomError::InvalidData {
        workflow: CREATE_FINAL_DOCUMENTS_WORKFLOW,
        message: source.to_string(),
    })
}
