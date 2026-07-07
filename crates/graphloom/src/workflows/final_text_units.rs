//! Final text-unit reference materialization workflow.

use std::collections::BTreeMap;

use async_trait::async_trait;
use polars_core::prelude::*;

use super::{
    base_text_units::{TextUnitRow, text_units_dataframe},
    common::{invalid_data, string_value},
    graph::{list_at, row_to_static},
    input_documents::usize_to_i64,
};
use crate::{GraphRagConfig, PipelineRunContext, Result, Workflow, WorkflowFunctionOutput};

/// Workflow name.
pub const CREATE_FINAL_TEXT_UNITS_WORKFLOW: &str = "create_final_text_units";

/// Fill final text-unit entity, relationship, and covariate references.
#[derive(Debug, Clone, Copy, Default)]
pub struct CreateFinalTextUnitsWorkflow;

#[async_trait]
impl Workflow for CreateFinalTextUnitsWorkflow {
    fn name(&self) -> &'static str {
        CREATE_FINAL_TEXT_UNITS_WORKFLOW
    }

    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut PipelineRunContext,
    ) -> Result<WorkflowFunctionOutput> {
        let text_units = read_text_units(
            &context
                .output_table_provider
                .read_dataframe("text_units")
                .await?,
        )?;
        let entity_map = build_multi_ref_map(
            &context
                .output_table_provider
                .read_dataframe("entities")
                .await?,
            "entity",
        )?;
        let relationship_map = build_multi_ref_map(
            &context
                .output_table_provider
                .read_dataframe("relationships")
                .await?,
            "relationship",
        )?;
        let covariate_map = if config.extract_claims.enabled
            && context.output_table_provider.has("covariates").await?
        {
            build_covariate_map(
                &context
                    .output_table_provider
                    .read_dataframe("covariates")
                    .await?,
            )?
        } else {
            BTreeMap::new()
        };

        let mut rows = Vec::with_capacity(text_units.len());
        for (index, text_unit) in text_units.into_iter().enumerate() {
            rows.push(TextUnitRow {
                id: text_unit.id.clone(),
                human_readable_id: usize_to_i64(index, CREATE_FINAL_TEXT_UNITS_WORKFLOW)?,
                text: text_unit.text,
                n_tokens: text_unit.n_tokens,
                document_id: text_unit.document_id,
                entity_ids: cloned_vec_or_empty(entity_map.get(&text_unit.id)),
                relationship_ids: cloned_vec_or_empty(relationship_map.get(&text_unit.id)),
                covariate_ids: cloned_vec_or_empty(covariate_map.get(&text_unit.id)),
            });
        }

        context
            .output_table_provider
            .write_dataframe("text_units", text_units_dataframe(&rows)?)
            .await?;

        Ok(WorkflowFunctionOutput {
            result: rows.iter().take(5).map(TextUnitRow::to_value).collect(),
            stop: false,
            input_rows: rows.len(),
            output_rows: rows.len(),
        })
    }
}

#[derive(Debug, Clone)]
struct TextUnitInput {
    id: String,
    text: String,
    n_tokens: i64,
    document_id: String,
}

fn cloned_vec_or_empty(values: Option<&Vec<String>>) -> Vec<String> {
    match values {
        Some(values) => values.clone(),
        None => Vec::new(),
    }
}

fn read_text_units(dataframe: &DataFrame) -> Result<Vec<TextUnitInput>> {
    let ids = dataframe.column("id")?.str()?;
    let texts = dataframe.column("text")?.str()?;
    let n_tokens = dataframe.column("n_tokens")?.i64()?;
    let document_ids = dataframe.column("document_id")?.str()?;
    let mut rows = Vec::with_capacity(dataframe.height());
    for index in 0..dataframe.height() {
        rows.push(TextUnitInput {
            id: string_value(ids.get(index), "id", CREATE_FINAL_TEXT_UNITS_WORKFLOW)?,
            text: string_value(texts.get(index), "text", CREATE_FINAL_TEXT_UNITS_WORKFLOW)?,
            n_tokens: n_tokens.get(index).ok_or_else(|| {
                invalid_data(CREATE_FINAL_TEXT_UNITS_WORKFLOW, "missing n_tokens")
            })?,
            document_id: string_value(
                document_ids.get(index),
                "document_id",
                CREATE_FINAL_TEXT_UNITS_WORKFLOW,
            )?,
        });
    }
    Ok(rows)
}

fn build_multi_ref_map(
    dataframe: &DataFrame,
    kind: &'static str,
) -> Result<BTreeMap<String, Vec<String>>> {
    let ids = dataframe.column("id")?.str()?;
    let text_unit_ids_index = dataframe
        .get_column_names()
        .iter()
        .position(|name| name.as_str() == "text_unit_ids")
        .ok_or_else(|| {
            invalid_data(
                CREATE_FINAL_TEXT_UNITS_WORKFLOW,
                &format!("missing {kind} text_unit_ids"),
            )
        })?;
    let mut mapping: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for index in 0..dataframe.height() {
        let row_id = string_value(ids.get(index), "id", CREATE_FINAL_TEXT_UNITS_WORKFLOW)?;
        let row = row_to_static(dataframe.get_row(index)?);
        for text_unit_id in list_at(&row, text_unit_ids_index)? {
            mapping
                .entry(text_unit_id)
                .or_default()
                .push(row_id.clone());
        }
    }
    Ok(mapping)
}

fn build_covariate_map(dataframe: &DataFrame) -> Result<BTreeMap<String, Vec<String>>> {
    let ids = dataframe.column("id")?.str()?;
    let text_unit_ids = dataframe.column("text_unit_id")?.str()?;
    let mut mapping: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for index in 0..dataframe.height() {
        let id = string_value(ids.get(index), "id", CREATE_FINAL_TEXT_UNITS_WORKFLOW)?;
        let text_unit_id = string_value(
            text_unit_ids.get(index),
            "text_unit_id",
            CREATE_FINAL_TEXT_UNITS_WORKFLOW,
        )?;
        mapping.entry(text_unit_id).or_default().push(id);
    }
    Ok(mapping)
}
