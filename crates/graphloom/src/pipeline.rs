//! Pipeline factory and sequential runner.

use std::time::Instant;

use tracing::{error, info};

use crate::{
    GraphLoomError, GraphRagConfig, PipelineRunContext, Result, WorkflowFunctionOutput,
    WorkflowRegistry,
};

/// Executable pipeline.
#[derive(Debug, Clone)]
pub struct Pipeline {
    workflow_names: Vec<String>,
    registry: WorkflowRegistry,
}

impl Pipeline {
    /// Run this pipeline sequentially.
    ///
    /// # Errors
    ///
    /// Stops at the first failing workflow and returns a workflow-scoped error.
    pub async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut PipelineRunContext,
    ) -> Result<Vec<WorkflowFunctionOutput>> {
        let started_at = Instant::now();
        let mut outputs = Vec::with_capacity(self.workflow_names.len());

        for workflow_name in &self.workflow_names {
            let workflow = self.registry.get(workflow_name)?;
            context.callbacks.workflow_started(workflow_name);
            let workflow_started = Instant::now();
            info!(workflow_name = %workflow_name, event = "started");

            match workflow.run(config, context).await {
                Ok(output) => {
                    let elapsed = workflow_started.elapsed();
                    context
                        .stats
                        .record_workflow_elapsed(workflow_name, elapsed);
                    info!(
                        workflow_name = %workflow_name,
                        event = "completed",
                        elapsed = elapsed.as_millis(),
                        input_rows = output.input_rows,
                        output_rows = output.output_rows,
                    );
                    context
                        .callbacks
                        .workflow_completed(workflow_name, &context.stats);
                    let should_stop = output.stop;
                    outputs.push(output);
                    if should_stop {
                        break;
                    }
                }
                Err(source) => {
                    error!(
                        workflow_name = %workflow_name,
                        event = "failed",
                        error = %source,
                    );
                    context.callbacks.error(workflow_name, &source.to_string());
                    return Err(GraphLoomError::WorkflowFailed {
                        name: workflow_name.clone(),
                        source: Box::new(source),
                    });
                }
            }
        }

        context.stats.elapsed_ms = started_at.elapsed().as_millis();
        Ok(outputs)
    }
}

/// Factory for built-in pipelines.
#[derive(Debug, Clone)]
pub struct PipelineFactory {
    registry: WorkflowRegistry,
}

impl PipelineFactory {
    /// Create a factory from a workflow registry.
    #[must_use]
    pub fn new(registry: WorkflowRegistry) -> Self {
        Self { registry }
    }

    /// Build the standard indexing pipeline.
    ///
    /// # Errors
    ///
    /// Returns an error when the configured workflow list contains an unknown name.
    pub fn standard(&self, config: &GraphRagConfig) -> Result<Pipeline> {
        let workflow_names = config.workflow_order();
        self.registry.validate_names(&workflow_names)?;
        Ok(Pipeline {
            workflow_names,
            registry: self.registry.clone(),
        })
    }
}
