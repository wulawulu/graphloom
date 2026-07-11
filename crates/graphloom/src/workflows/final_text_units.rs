//! Final text-unit reference materialization workflow.

use std::collections::BTreeMap;

use async_trait::async_trait;
use polars_core::prelude::*;

use crate::{
    GraphRagConfig, IndexPipelineContext, IndexWorkflow, IndexWorkflowOutput, Result,
    dataframe::{
        i64_column_value, invalid_data, list_at, row_to_static, string_value, usize_to_i64,
    },
    operations::text_units::{TextUnitRow, text_units_dataframe},
};

/// IndexWorkflow name.
pub const CREATE_FINAL_TEXT_UNITS_WORKFLOW: &str = "create_final_text_units";

/// Fill final text-unit entity, relationship, and covariate references.
#[derive(Debug, Clone, Copy, Default)]
pub struct CreateFinalTextUnitsWorkflow;

#[async_trait]
impl IndexWorkflow for CreateFinalTextUnitsWorkflow {
    fn name(&self) -> &'static str {
        CREATE_FINAL_TEXT_UNITS_WORKFLOW
    }

    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut IndexPipelineContext,
    ) -> Result<IndexWorkflowOutput> {
        let text_units = read_text_units(
            &context
                .output_table_provider()
                .read_dataframe("text_units")
                .await?,
        )?;
        let entity_map = build_multi_ref_map(
            &context
                .output_table_provider()
                .read_dataframe("entities")
                .await?,
            "entity",
        )?;
        let relationship_map = build_multi_ref_map(
            &context
                .output_table_provider()
                .read_dataframe("relationships")
                .await?,
            "relationship",
        )?;
        let covariate_map = if config.extract_claims.enabled
            && context.output_table_provider().has("covariates").await?
        {
            build_covariate_map(
                &context
                    .output_table_provider()
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
                human_readable_id: usize_to_i64(
                    index,
                    CREATE_FINAL_TEXT_UNITS_WORKFLOW,
                    "human_readable_id",
                )?,
                text: text_unit.text,
                n_tokens: text_unit.n_tokens,
                document_id: text_unit.document_id,
                entity_ids: cloned_vec_or_empty(entity_map.get(&text_unit.id)),
                relationship_ids: cloned_vec_or_empty(relationship_map.get(&text_unit.id)),
                covariate_ids: cloned_vec_or_empty(covariate_map.get(&text_unit.id)),
            });
        }

        context
            .output_table_provider()
            .write_dataframe("text_units", text_units_dataframe(&rows)?)
            .await?;

        Ok(IndexWorkflowOutput {
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
    let document_ids = dataframe.column("document_id")?.str()?;
    let mut rows = Vec::with_capacity(dataframe.height());
    for index in 0..dataframe.height() {
        rows.push(TextUnitInput {
            id: string_value(ids.get(index), "id", CREATE_FINAL_TEXT_UNITS_WORKFLOW)?,
            text: string_value(texts.get(index), "text", CREATE_FINAL_TEXT_UNITS_WORKFLOW)?,
            n_tokens: i64_column_value(
                dataframe,
                index,
                "n_tokens",
                CREATE_FINAL_TEXT_UNITS_WORKFLOW,
            )?,
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
        for text_unit_id in list_at(&row, text_unit_ids_index, CREATE_FINAL_TEXT_UNITS_WORKFLOW)? {
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

#[cfg(test)]
mod tests {
    use polars_core::prelude::*;

    use super::*;
    use crate::dataframe::list_column;

    #[test]
    fn test_should_read_text_units_by_column_name() {
        let dataframe = df!(
            "n_tokens" => [7i64],
            "text" => ["Alice reports Bob."],
            "id" => ["tu-1"],
            "document_id" => ["doc-1"],
        )
        .expect("dataframe should build");

        let rows = read_text_units(&dataframe).expect("text units should decode");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "tu-1");
        assert_eq!(rows[0].n_tokens, 7);
        assert_eq!(rows[0].document_id, "doc-1");
    }

    #[test]
    fn test_should_error_on_wrong_text_unit_token_type() {
        let dataframe = df!(
            "id" => ["tu-1"],
            "text" => ["Alice reports Bob."],
            "n_tokens" => ["seven"],
            "document_id" => ["doc-1"],
        )
        .expect("dataframe should build");

        let error = read_text_units(&dataframe).expect_err("n_tokens type should fail");

        assert!(error.to_string().contains("n_tokens"));
    }

    #[test]
    fn test_should_build_text_unit_reference_maps() {
        let mut entities =
            df!("id" => ["entity-1", "entity-2"]).expect("entities dataframe should build");
        entities
            .with_column(list_column(
                "text_unit_ids",
                &[
                    vec!["tu-1".to_owned(), "tu-2".to_owned()],
                    vec!["tu-1".to_owned()],
                ],
            ))
            .expect("entity text unit ids should append");
        let mut relationships =
            df!("id" => ["rel-1"]).expect("relationships dataframe should build");
        relationships
            .with_column(list_column("text_unit_ids", &[vec!["tu-1".to_owned()]]))
            .expect("relationship text unit ids should append");
        let covariates = df!(
            "id" => ["claim-1", "claim-2"],
            "text_unit_id" => ["tu-1", "tu-1"],
        )
        .expect("covariates dataframe should build");

        let entity_map = build_multi_ref_map(&entities, "entity").expect("entity map");
        let relationship_map =
            build_multi_ref_map(&relationships, "relationship").expect("relationship map");
        let covariate_map = build_covariate_map(&covariates).expect("covariate map");

        assert_eq!(
            entity_map.get("tu-1").expect("tu-1 entities"),
            &vec!["entity-1".to_owned(), "entity-2".to_owned()]
        );
        assert_eq!(
            relationship_map.get("tu-1").expect("tu-1 relationships"),
            &vec!["rel-1".to_owned()]
        );
        assert_eq!(
            covariate_map.get("tu-1").expect("tu-1 covariates"),
            &vec!["claim-1".to_owned(), "claim-2".to_owned()]
        );
        assert_eq!(
            cloned_vec_or_empty(entity_map.get("missing")),
            Vec::<String>::new()
        );
    }

    #[test]
    fn test_should_error_on_wrong_reference_list_type() {
        let dataframe = df!(
            "id" => ["entity-1"],
            "text_unit_ids" => [42i64],
        )
        .expect("dataframe should build");

        let error =
            build_multi_ref_map(&dataframe, "entity").expect_err("text_unit_ids type should fail");

        assert!(error.to_string().contains("string list"));
    }
}
