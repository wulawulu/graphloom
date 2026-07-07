//! Workflow abstraction and registry.

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use serde_json::Value;

use crate::{GraphLoomError, GraphRagConfig, PipelineRunContext, Result};

/// Workflow return envelope.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct WorkflowFunctionOutput {
    /// Up to five sample output rows.
    pub result: Vec<Value>,
    /// Number of input rows read.
    pub input_rows: usize,
    /// Number of output rows written.
    pub output_rows: usize,
}

/// Pipeline workflow.
#[async_trait]
pub trait Workflow: Send + Sync + std::fmt::Debug {
    /// Workflow name.
    fn name(&self) -> &'static str;

    /// Execute the workflow.
    ///
    /// # Errors
    ///
    /// Returns an error when the workflow cannot complete.
    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut PipelineRunContext,
    ) -> Result<WorkflowFunctionOutput>;
}

/// Registry of named workflow implementations.
#[derive(Debug, Clone, Default)]
pub struct WorkflowRegistry {
    workflows: BTreeMap<String, Arc<dyn Workflow>>,
}

impl WorkflowRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a workflow.
    pub fn register<W>(&mut self, workflow: W)
    where
        W: Workflow + 'static,
    {
        self.workflows
            .insert(workflow.name().to_owned(), Arc::new(workflow));
    }

    /// Return a workflow by name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is not registered.
    pub fn get(&self, name: &str) -> Result<Arc<dyn Workflow>> {
        self.workflows
            .get(name)
            .cloned()
            .ok_or_else(|| GraphLoomError::UnknownWorkflow {
                name: name.to_owned(),
            })
    }

    /// Validate all workflow names.
    ///
    /// # Errors
    ///
    /// Returns an error for the first unregistered workflow.
    pub fn validate_names(&self, names: &[String]) -> Result<()> {
        for name in names {
            self.get(name)?;
        }
        Ok(())
    }
}
