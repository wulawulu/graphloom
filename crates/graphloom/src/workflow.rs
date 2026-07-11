//! GraphRAG indexing workflow contracts and registry.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use async_trait::async_trait;
use serde_json::Value;

use crate::{GraphLoomError, GraphRagConfig, IndexPipelineContext, Result};

/// Model dependencies declared by one or more indexing workflows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexWorkflowRequirements {
    completion_models: BTreeSet<String>,
    embedding_models: BTreeSet<String>,
    vector_store: bool,
    chunking_config: bool,
    tokenizer_encodings: BTreeSet<TokenizerRequirement>,
}

/// One tokenizer encoding required by an active indexing workflow.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TokenizerRequirement {
    pub(crate) source: String,
    pub(crate) encoding: String,
}

impl IndexWorkflowRequirements {
    /// Require a completion model by configured identifier.
    pub fn require_completion_model(&mut self, model_id: impl Into<String>) {
        self.completion_models.insert(model_id.into());
    }

    /// Require an embedding model by configured identifier.
    pub fn require_embedding_model(&mut self, model_id: impl Into<String>) {
        self.embedding_models.insert(model_id.into());
    }

    /// Iterate over required completion model identifiers.
    pub fn completion_models(&self) -> impl Iterator<Item = &str> {
        self.completion_models.iter().map(String::as_str)
    }

    /// Iterate over required embedding model identifiers.
    pub fn embedding_models(&self) -> impl Iterator<Item = &str> {
        self.embedding_models.iter().map(String::as_str)
    }

    /// Merge another workflow's requirements into this set.
    pub fn merge(&mut self, other: Self) {
        self.completion_models.extend(other.completion_models);
        self.embedding_models.extend(other.embedding_models);
        self.vector_store |= other.vector_store;
        self.chunking_config |= other.chunking_config;
        self.tokenizer_encodings.extend(other.tokenizer_encodings);
    }

    /// Require vector storage for this workflow.
    pub fn require_vector_store(&mut self) {
        self.vector_store = true;
    }

    /// Return whether the active indexing pipeline requires vector storage.
    #[must_use]
    pub fn requires_vector_store(&self) -> bool {
        self.vector_store
    }

    pub(crate) fn require_chunking_config(&mut self) {
        self.chunking_config = true;
    }

    pub(crate) fn requires_chunking_config(&self) -> bool {
        self.chunking_config
    }

    pub(crate) fn require_tokenizer(
        &mut self,
        source: impl Into<String>,
        encoding: impl Into<String>,
    ) {
        self.tokenizer_encodings.insert(TokenizerRequirement {
            source: source.into(),
            encoding: encoding.into(),
        });
    }

    pub(crate) fn tokenizer_requirements(&self) -> impl Iterator<Item = &TokenizerRequirement> {
        self.tokenizer_encodings.iter()
    }
}

/// Result returned by a GraphRAG indexing workflow.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct IndexWorkflowOutput {
    /// Up to five sample output rows.
    pub result: Vec<Value>,
    /// Stop the indexing pipeline after this workflow.
    pub stop: bool,
    /// Number of input rows read.
    pub input_rows: usize,
    /// Number of output rows written.
    pub output_rows: usize,
}

/// One executable step in the GraphRAG indexing pipeline.
#[async_trait]
pub trait IndexWorkflow: Send + Sync + std::fmt::Debug {
    /// Stable indexing workflow name.
    fn name(&self) -> &'static str;

    /// Declare model dependencies for the resolved indexing configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when requirements cannot be derived from configuration.
    fn requirements(&self, _config: &GraphRagConfig) -> Result<IndexWorkflowRequirements> {
        Ok(IndexWorkflowRequirements::default())
    }

    /// Execute the indexing workflow.
    ///
    /// # Errors
    ///
    /// Returns an error when the workflow cannot complete.
    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut IndexPipelineContext,
    ) -> Result<IndexWorkflowOutput>;
}

/// Registry used while compiling a GraphRAG indexing pipeline.
#[derive(Debug, Clone, Default)]
pub struct IndexWorkflowRegistry {
    workflows: BTreeMap<String, Arc<dyn IndexWorkflow>>,
}

impl IndexWorkflowRegistry {
    /// Create an empty indexing workflow registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an indexing workflow without replacing an existing entry.
    ///
    /// # Errors
    ///
    /// Returns an error when the workflow name is already registered.
    pub fn register<W>(&mut self, workflow: W) -> Result<()>
    where
        W: IndexWorkflow + 'static,
    {
        let name = workflow.name();
        if self.workflows.contains_key(name) {
            return Err(GraphLoomError::DuplicateIndexWorkflow {
                name: name.to_owned(),
            });
        }
        self.workflows.insert(name.to_owned(), Arc::new(workflow));
        Ok(())
    }

    pub(crate) fn resolve(&self, name: &str) -> Result<Arc<dyn IndexWorkflow>> {
        self.workflows
            .get(name)
            .cloned()
            .ok_or_else(|| GraphLoomError::UnknownIndexWorkflow {
                name: name.to_owned(),
            })
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::{
        IndexWorkflow, IndexWorkflowOutput, IndexWorkflowRegistry, IndexWorkflowRequirements,
    };
    use crate::{GraphRagConfig, IndexPipelineContext, Result};

    #[derive(Debug, Clone, Copy)]
    struct NamedWorkflow;

    #[async_trait]
    impl IndexWorkflow for NamedWorkflow {
        fn name(&self) -> &'static str {
            "named"
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
    fn test_should_reject_duplicate_index_workflow_without_replacement() {
        let mut registry = IndexWorkflowRegistry::new();
        registry
            .register(NamedWorkflow)
            .expect("first registration");

        let error = registry
            .register(NamedWorkflow)
            .expect_err("duplicate must fail");

        assert!(error.to_string().contains("index workflow `named`"));
        assert!(registry.resolve("named").is_ok());
    }

    #[test]
    fn test_should_deduplicate_identical_tokenizer_requirements_when_merging() {
        let mut requirements = IndexWorkflowRequirements::default();
        requirements.require_tokenizer("chunking.encoding_model", "cl100k_base");

        let mut duplicate = IndexWorkflowRequirements::default();
        duplicate.require_tokenizer("chunking.encoding_model", "cl100k_base");
        requirements.merge(duplicate);

        assert_eq!(requirements.tokenizer_requirements().count(), 1);
    }
}
