//! Prepared model instances shared by indexing workflows.

use std::{collections::BTreeMap, sync::Arc};

use graphloom_llm::{CompletionModel, EmbeddingModel};

use crate::{GraphLoomError, Result};

/// Registry of prepared completion and embedding model instances.
#[derive(Debug, Clone, Default)]
pub struct ModelRegistry {
    completion: BTreeMap<String, Arc<dyn CompletionModel>>,
    embedding: BTreeMap<String, Arc<dyn EmbeddingModel>>,
}

impl ModelRegistry {
    /// Register a completion model without replacing an existing model.
    ///
    /// # Errors
    ///
    /// Returns an error when `id` is already registered as a completion model.
    pub fn insert_completion(
        &mut self,
        id: impl Into<String>,
        model: Arc<dyn CompletionModel>,
    ) -> Result<()> {
        insert_model(&mut self.completion, id.into(), model, "completion")
    }

    /// Register an embedding model without replacing an existing model.
    ///
    /// # Errors
    ///
    /// Returns an error when `id` is already registered as an embedding model.
    pub fn insert_embedding(
        &mut self,
        id: impl Into<String>,
        model: Arc<dyn EmbeddingModel>,
    ) -> Result<()> {
        insert_model(&mut self.embedding, id.into(), model, "embedding")
    }

    /// Return a prepared completion model.
    ///
    /// # Errors
    ///
    /// Returns an error when no completion model is registered for `id`.
    #[cfg(test)]
    pub(crate) fn completion(&self, id: &str) -> Result<Arc<dyn CompletionModel>> {
        self.completion_for_workflow(id, "model_registry")
    }

    /// Return a prepared embedding model.
    ///
    /// # Errors
    ///
    /// Returns an error when no embedding model is registered for `id`.
    #[cfg(test)]
    pub(crate) fn embedding(&self, id: &str) -> Result<Arc<dyn EmbeddingModel>> {
        self.embedding_for_workflow(id, "model_registry")
    }

    pub(crate) fn completion_for_workflow(
        &self,
        id: &str,
        workflow: &'static str,
    ) -> Result<Arc<dyn CompletionModel>> {
        self.completion
            .get(id)
            .cloned()
            .ok_or_else(|| missing_model(id, "completion", workflow))
    }

    pub(crate) fn embedding_for_workflow(
        &self,
        id: &str,
        workflow: &'static str,
    ) -> Result<Arc<dyn EmbeddingModel>> {
        self.embedding
            .get(id)
            .cloned()
            .ok_or_else(|| missing_model(id, "embedding", workflow))
    }
}

fn insert_model<T: ?Sized>(
    models: &mut BTreeMap<String, Arc<T>>,
    id: String,
    model: Arc<T>,
    kind: &'static str,
) -> Result<()> {
    if models.contains_key(&id) {
        return Err(GraphLoomError::DuplicateModelRegistration { kind, model_id: id });
    }
    models.insert(id, model);
    Ok(())
}

fn missing_model(id: &str, kind: &'static str, workflow: &'static str) -> GraphLoomError {
    GraphLoomError::MissingPreparedModel {
        kind,
        model_id: id.to_owned(),
        workflow,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use graphloom_llm::{MockCompletionModel, MockEmbeddingModel};

    use super::ModelRegistry;

    #[test]
    fn test_should_register_and_retrieve_models_by_kind() {
        let completion = Arc::new(MockCompletionModel::new("shared", vec!["ok".to_owned()]));
        let embedding = Arc::new(MockEmbeddingModel::new("shared", vec![1.0]));
        let mut registry = ModelRegistry::default();

        registry
            .insert_completion("shared", completion.clone())
            .expect("completion should register");
        registry
            .insert_embedding("shared", embedding.clone())
            .expect("embedding should register");

        let resolved_completion = registry.completion("shared").expect("completion");
        let resolved_embedding = registry.embedding("shared").expect("embedding");
        let completion_trait: Arc<dyn graphloom_llm::CompletionModel> = completion;
        let embedding_trait: Arc<dyn graphloom_llm::EmbeddingModel> = embedding;
        assert!(Arc::ptr_eq(&resolved_completion, &completion_trait));
        assert!(Arc::ptr_eq(&resolved_embedding, &embedding_trait));
    }

    #[test]
    fn test_should_report_missing_model_kind_id_and_workflow() {
        let registry = ModelRegistry::default();

        let completion = registry
            .completion_for_workflow("chat", "extract_graph")
            .expect_err("completion should be missing")
            .to_string();
        let embedding = registry
            .embedding_for_workflow("embed", "generate_text_embeddings")
            .expect_err("embedding should be missing")
            .to_string();

        assert!(completion.contains("completion model `chat`"));
        assert!(completion.contains("workflow `extract_graph`"));
        assert!(embedding.contains("embedding model `embed`"));
        assert!(embedding.contains("workflow `generate_text_embeddings`"));
    }

    #[test]
    fn test_should_reject_duplicate_registration_without_replacement() {
        let first = Arc::new(MockCompletionModel::new("first", vec!["first".to_owned()]));
        let second = Arc::new(MockCompletionModel::new(
            "second",
            vec!["second".to_owned()],
        ));
        let mut registry = ModelRegistry::default();
        registry
            .insert_completion("chat", first.clone())
            .expect("first should register");

        let error = registry
            .insert_completion("chat", second)
            .expect_err("duplicate should fail");
        let resolved = registry.completion("chat").expect("first should remain");
        let first_trait: Arc<dyn graphloom_llm::CompletionModel> = first;

        assert!(error.to_string().contains("already registered"));
        assert!(Arc::ptr_eq(&resolved, &first_trait));
    }
}
