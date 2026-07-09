//! `create_final_documents` workflow.

use std::collections::BTreeMap;

use async_trait::async_trait;
use polars_core::prelude::DataFrame;

use super::input_documents::{DocumentRow, documents_dataframe};
use crate::{
    GraphRagConfig, PipelineRunContext, Result, Workflow, WorkflowFunctionOutput,
    dataframe::{optional_string_at, row_to_static, string_value},
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
        let document_ids = documents.column("id")?.str()?;
        let document_texts = documents.column("text")?.str()?;
        let mut rows = Vec::with_capacity(documents.height());
        let mut sample = Vec::new();

        for row_index in 0..documents.height() {
            let row = row_to_static(documents.get_row(row_index)?);
            let document_id = string_value(
                document_ids.get(row_index),
                "id",
                CREATE_FINAL_DOCUMENTS_WORKFLOW,
            )?;
            let document = DocumentRow {
                id: document_id.clone(),
                human_readable_id: row_index as i64,
                title: optional_string_at(&row, 2),
                text: string_value(
                    document_texts.get(row_index),
                    "text",
                    CREATE_FINAL_DOCUMENTS_WORKFLOW,
                )?,
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
    let text_unit_ids = dataframe.column("id")?.str()?;
    let document_ids = dataframe.column("document_id")?.str()?;
    let mut mapping: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row_index in 0..dataframe.height() {
        let text_unit_id = string_value(
            text_unit_ids.get(row_index),
            "id",
            CREATE_FINAL_DOCUMENTS_WORKFLOW,
        )?;
        let document_id = string_value(
            document_ids.get(row_index),
            "document_id",
            CREATE_FINAL_DOCUMENTS_WORKFLOW,
        )?;
        mapping.entry(document_id).or_default().push(text_unit_id);
    }
    Ok(mapping)
}
