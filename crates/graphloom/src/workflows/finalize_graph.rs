//! Graph finalization workflow orchestration.

use async_trait::async_trait;

use crate::{
    GraphLoomError, GraphRagConfig, PipelineRunContext, Result, Workflow, WorkflowFunctionOutput,
    operations::graph::{
        degree_map, final_entities_dataframe, final_relationships_dataframe, finalize_entities,
        finalize_graph_sample, finalize_relationships, graphml_snapshot, read_entity_rows,
        read_relationship_rows,
    },
};

/// Workflow name.
pub const FINALIZE_GRAPH_WORKFLOW: &str = "finalize_graph";

/// Finalize extracted graph rows.
#[derive(Debug, Clone, Copy, Default)]
pub struct FinalizeGraphWorkflow;

#[async_trait]
impl Workflow for FinalizeGraphWorkflow {
    fn name(&self) -> &'static str {
        FINALIZE_GRAPH_WORKFLOW
    }

    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut PipelineRunContext,
    ) -> Result<WorkflowFunctionOutput> {
        let entities = read_entity_rows(
            &context
                .output_table_provider
                .read_dataframe("entities")
                .await?,
        )?;
        let relationships = read_relationship_rows(
            &context
                .output_table_provider
                .read_dataframe("relationships")
                .await?,
        )?;
        let degree_map = degree_map(&relationships);
        let final_entities = finalize_entities(&entities, &degree_map);
        let final_relationships = finalize_relationships(&relationships, &degree_map);

        context
            .output_table_provider
            .write_dataframe("entities", final_entities_dataframe(&final_entities)?)
            .await?;
        context
            .output_table_provider
            .write_dataframe(
                "relationships",
                final_relationships_dataframe(&final_relationships)?,
            )
            .await?;

        if config.snapshots.graphml {
            let storage =
                context
                    .output_storage
                    .as_ref()
                    .ok_or(GraphLoomError::MissingProvider {
                        name: "output_storage",
                    })?;
            storage
                .set_text("graph.graphml", &graphml_snapshot(&final_relationships))
                .await?;
        }

        context.stats.entity_count = final_entities.len();
        context.stats.relationship_count = final_relationships.len();
        Ok(WorkflowFunctionOutput {
            result: finalize_graph_sample(&final_entities, &final_relationships),
            stop: false,
            input_rows: entities.len().saturating_add(relationships.len()),
            output_rows: final_entities
                .len()
                .saturating_add(final_relationships.len()),
        })
    }
}
