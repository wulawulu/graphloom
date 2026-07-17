//! Graph finalization workflow orchestration.

use async_trait::async_trait;

use crate::{
    GraphRagConfig, IndexPipelineContext, IndexWorkflow, IndexWorkflowOutput, Result,
    operations::graph::{
        final_entities_dataframe, final_relationships_dataframe, finalize_graph,
        finalize_graph_sample, graphml_snapshot, read_entity_rows, read_relationship_rows,
    },
};

/// `IndexWorkflow` name.
pub const FINALIZE_GRAPH_WORKFLOW: &str = "finalize_graph";

/// Finalize extracted graph rows.
#[derive(Debug, Clone, Copy, Default)]
pub struct FinalizeGraphWorkflow;

#[async_trait]
impl IndexWorkflow for FinalizeGraphWorkflow {
    fn name(&self) -> &'static str {
        FINALIZE_GRAPH_WORKFLOW
    }

    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut IndexPipelineContext,
    ) -> Result<IndexWorkflowOutput> {
        let entities = read_entity_rows(
            &context
                .output_table_provider()
                .read_dataframe("entities")
                .await?,
        )?;
        let relationships = read_relationship_rows(
            &context
                .output_table_provider()
                .read_dataframe("relationships")
                .await?,
        )?;
        let finalized = finalize_graph(&entities, &relationships)?;

        context
            .output_table_provider()
            .write_dataframe("entities", final_entities_dataframe(&finalized.entities)?)
            .await?;
        context
            .output_table_provider()
            .write_dataframe(
                "relationships",
                final_relationships_dataframe(&finalized.relationships)?,
            )
            .await?;

        if config.snapshots.graphml {
            let storage = context.output_storage();
            storage
                .set_text("graph.graphml", &graphml_snapshot(&finalized.relationships))
                .await?;
        }

        context.stats.entity_count = finalized.entities.len();
        context.stats.relationship_count = finalized.relationships.len();
        Ok(IndexWorkflowOutput {
            result: finalize_graph_sample(&finalized.entities, &finalized.relationships),
            stop: false,
            input_rows: entities.len().saturating_add(relationships.len()),
            output_rows: finalized
                .entities
                .len()
                .saturating_add(finalized.relationships.len()),
        })
    }
}
