//! Provider-specific model construction for indexing runtime preparation.

use std::sync::Arc;

use graphloom_llm::{
    CompletionModel, EmbeddingModel, ModelConfig, OpenAiCompletionModel, OpenAiEmbeddingModel,
};

use crate::{
    GraphLoomError, GraphRagConfig, IndexWorkflowRequirements, Result, runtime::ModelRegistry,
};

/// Factory for provider-specific model clients.
pub trait ModelFactory: Send + Sync + std::fmt::Debug {
    /// Create a completion model instance.
    fn create_completion(
        &self,
        id: &str,
        config: &ModelConfig,
        concurrent_requests: usize,
    ) -> Result<Arc<dyn CompletionModel>>;

    /// Create an embedding model instance.
    fn create_embedding(
        &self,
        id: &str,
        config: &ModelConfig,
        concurrent_requests: usize,
    ) -> Result<Arc<dyn EmbeddingModel>>;
}

/// Default model factory using the currently supported OpenAI adapters.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultModelFactory;

impl ModelFactory for DefaultModelFactory {
    fn create_completion(
        &self,
        id: &str,
        config: &ModelConfig,
        concurrent_requests: usize,
    ) -> Result<Arc<dyn CompletionModel>> {
        Ok(Arc::new(OpenAiCompletionModel::new(
            id,
            config.clone(),
            concurrent_requests,
        )?))
    }

    fn create_embedding(
        &self,
        id: &str,
        config: &ModelConfig,
        concurrent_requests: usize,
    ) -> Result<Arc<dyn EmbeddingModel>> {
        Ok(Arc::new(OpenAiEmbeddingModel::new(
            id,
            config.clone(),
            concurrent_requests,
        )?))
    }
}

pub(crate) fn create_model_registry(
    config: &GraphRagConfig,
    requirements: &IndexWorkflowRequirements,
    factory: &dyn ModelFactory,
) -> Result<ModelRegistry> {
    let mut registry = ModelRegistry::default();
    for id in requirements.completion_models() {
        let model_config =
            config
                .completion_models
                .get(id)
                .ok_or_else(|| GraphLoomError::InvalidModel {
                    model_id: id.to_owned(),
                    message: "required completion model is not configured".to_owned(),
                })?;
        registry.insert_completion(
            id,
            factory.create_completion(id, model_config, config.concurrent_requests)?,
        )?;
    }
    for id in requirements.embedding_models() {
        let model_config =
            config
                .embedding_models
                .get(id)
                .ok_or_else(|| GraphLoomError::InvalidModel {
                    model_id: id.to_owned(),
                    message: "required embedding model is not configured".to_owned(),
                })?;
        registry.insert_embedding(
            id,
            factory.create_embedding(id, model_config, config.concurrent_requests)?,
        )?;
    }
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use graphloom_llm::{
        CompletionModel, EmbeddingModel, MockCompletionModel, MockEmbeddingModel, ModelConfig,
    };

    use super::{ModelFactory, create_model_registry};
    use crate::{GraphRagConfig, IndexWorkflowRequirements, Result};

    #[derive(Debug, Default)]
    struct CountingModelFactory {
        completion_calls: AtomicUsize,
        embedding_calls: AtomicUsize,
    }

    impl ModelFactory for CountingModelFactory {
        fn create_completion(
            &self,
            id: &str,
            _config: &ModelConfig,
            _concurrent_requests: usize,
        ) -> Result<Arc<dyn CompletionModel>> {
            self.completion_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(MockCompletionModel::new(
                id,
                vec!["ok".to_owned()],
            )))
        }

        fn create_embedding(
            &self,
            id: &str,
            _config: &ModelConfig,
            _concurrent_requests: usize,
        ) -> Result<Arc<dyn EmbeddingModel>> {
            self.embedding_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(MockEmbeddingModel::new(id, vec![1.0])))
        }
    }

    #[test]
    fn test_should_create_each_configured_model_once_and_reuse_arc() {
        let model_config = serde_json::from_value::<ModelConfig>(serde_json::json!({
            "model_provider": "openai",
            "model": "test-model",
        }))
        .expect("model config should deserialize");
        let mut config = GraphRagConfig::default();
        config
            .completion_models
            .insert("shared".to_owned(), model_config);
        let factory = CountingModelFactory::default();
        let mut requirements = IndexWorkflowRequirements::default();
        requirements.require_completion_model("shared");

        let registry =
            create_model_registry(&config, &requirements, &factory).expect("registry should build");
        let first = registry.completion("shared").expect("first lookup");
        let second = registry.completion("shared").expect("second lookup");

        assert_eq!(factory.completion_calls.load(Ordering::SeqCst), 1);
        assert_eq!(factory.embedding_calls.load(Ordering::SeqCst), 0);
        assert!(Arc::ptr_eq(&first, &second));
    }
}
