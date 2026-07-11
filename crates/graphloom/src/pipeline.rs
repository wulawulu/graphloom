//! Compiled GraphRAG indexing pipeline and factory.

use std::{sync::Arc, time::Instant};

use tracing::{error, info};

use crate::{
    GraphLoomError, GraphRagConfig, IndexPipelineContext, IndexWorkflow, IndexWorkflowOutput,
    IndexWorkflowRegistry, IndexWorkflowRequirements, Result,
};

/// A resolved workflow step in an indexing pipeline.
#[derive(Debug, Clone)]
pub struct IndexPipelineStep {
    name: &'static str,
    workflow: Arc<dyn IndexWorkflow>,
}

/// Compiled GraphRAG indexing pipeline.
#[derive(Debug, Clone)]
pub struct IndexPipeline {
    workflows: Vec<IndexPipelineStep>,
}

impl IndexPipeline {
    /// Iterate over workflow names in execution order.
    #[cfg(test)]
    pub fn workflow_names(&self) -> impl Iterator<Item = &str> {
        self.workflows.iter().map(|step| step.name)
    }

    /// Aggregate requirements declared by all active workflows.
    ///
    /// # Errors
    ///
    /// Returns an error when a workflow cannot derive its requirements.
    pub fn requirements(&self, config: &GraphRagConfig) -> Result<IndexWorkflowRequirements> {
        let mut requirements = IndexWorkflowRequirements::default();
        for step in &self.workflows {
            requirements.merge(step.workflow.requirements(config)?);
        }
        Ok(requirements)
    }

    /// Execute all resolved indexing steps sequentially.
    ///
    /// # Errors
    ///
    /// Stops at the first failing workflow and returns a workflow-scoped error.
    pub async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut IndexPipelineContext,
    ) -> Result<Vec<IndexWorkflowOutput>> {
        let started_at = Instant::now();
        let mut outputs = Vec::with_capacity(self.workflows.len());
        for step in &self.workflows {
            let name = step.name;
            context.callbacks.workflow_started(name);
            let workflow_started = Instant::now();
            info!(workflow_name = name, event = "started");
            match step.workflow.run(config, context).await {
                Ok(output) => {
                    let elapsed = workflow_started.elapsed();
                    context.stats.record_workflow_elapsed(name, elapsed);
                    info!(
                        workflow_name = name,
                        event = "completed",
                        elapsed = elapsed.as_millis(),
                        input_rows = output.input_rows,
                        output_rows = output.output_rows
                    );
                    context.callbacks.workflow_completed(name, &context.stats);
                    let should_stop = output.stop;
                    outputs.push(output);
                    if should_stop {
                        break;
                    }
                }
                Err(source) => {
                    error!(workflow_name = name, event = "failed", error = %source);
                    context.callbacks.error(name, &source.to_string());
                    return Err(GraphLoomError::IndexWorkflowFailed {
                        name: name.to_owned(),
                        source: Box::new(source),
                    });
                }
            }
        }
        context.stats.elapsed_ms = started_at.elapsed().as_millis();
        Ok(outputs)
    }
}

/// Factory that compiles configured workflow names into indexing steps.
#[derive(Debug, Clone)]
pub struct IndexPipelineFactory {
    registry: IndexWorkflowRegistry,
}

impl IndexPipelineFactory {
    /// Create an indexing pipeline factory.
    #[must_use]
    pub fn new(registry: IndexWorkflowRegistry) -> Self {
        Self { registry }
    }

    /// Compile the configured standard indexing pipeline.
    ///
    /// # Errors
    ///
    /// Returns an error when a configured workflow is unknown.
    pub fn standard(&self, config: &GraphRagConfig) -> Result<IndexPipeline> {
        let workflows = config
            .workflow_order()
            .into_iter()
            .map(|name| {
                let workflow = self.registry.resolve(&name)?;
                Ok(IndexPipelineStep {
                    name: workflow.name(),
                    workflow,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(IndexPipeline { workflows })
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::IndexPipelineFactory;
    use crate::{
        GraphRagConfig, IndexPipelineContext, IndexWorkflow, IndexWorkflowOutput,
        IndexWorkflowRegistry, IndexWorkflowRequirements, Result,
    };

    #[derive(Debug, Clone, Copy)]
    struct RequiredWorkflow;

    #[async_trait]
    impl IndexWorkflow for RequiredWorkflow {
        fn name(&self) -> &'static str {
            "required"
        }

        fn requirements(&self, _config: &GraphRagConfig) -> Result<IndexWorkflowRequirements> {
            let mut requirements = IndexWorkflowRequirements::default();
            requirements.require_completion_model("shared");
            Ok(requirements)
        }

        async fn run(
            &self,
            _config: &GraphRagConfig,
            _context: &mut IndexPipelineContext,
        ) -> Result<IndexWorkflowOutput> {
            Ok(IndexWorkflowOutput::default())
        }
    }

    #[test]
    fn test_should_compile_resolved_workflows_independent_of_registry() {
        let mut registry = IndexWorkflowRegistry::new();
        registry
            .register(RequiredWorkflow)
            .expect("workflow registration");
        let factory = IndexPipelineFactory::new(registry);
        let config = GraphRagConfig {
            workflows: vec!["required".to_owned()],
            ..Default::default()
        };
        let pipeline = factory.standard(&config).expect("pipeline should compile");
        drop(factory);

        assert_eq!(
            pipeline.workflow_names().collect::<Vec<_>>(),
            vec!["required"]
        );
        assert_eq!(
            pipeline
                .requirements(&config)
                .expect("requirements")
                .completion_models()
                .collect::<Vec<_>>(),
            vec!["shared"]
        );
    }

    #[test]
    fn test_should_reject_unknown_workflow_while_compiling_pipeline() {
        let config = GraphRagConfig {
            workflows: vec!["missing".to_owned()],
            ..Default::default()
        };
        let error = IndexPipelineFactory::new(IndexWorkflowRegistry::new())
            .standard(&config)
            .expect_err("unknown workflow must fail");
        assert!(error.to_string().contains("index workflow `missing`"));
    }
}
