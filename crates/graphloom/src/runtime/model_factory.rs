//! Provider-specific model construction for indexing runtime preparation.

use std::{error::Error, fmt, sync::Arc};

use graphloom_llm::{
    ChatMessage, CompletionModel, CompletionRequest, EmbeddingModel, EmbeddingRequest, ModelConfig,
    OpenAiCompletionModel, OpenAiEmbeddingModel,
};
use secrecy::ExposeSecret;

use crate::{
    GraphLoomError, GraphRagConfig, IndexWorkflowRequirements, Result,
    error::ModelConnectivityError, runtime::ModelRegistry,
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

const COMPLETION_CONNECTIVITY_TEXT: &str = "This is an LLM connectivity test. Say Hello World";
const EMBEDDING_CONNECTIVITY_TEXT: &str = "This is an LLM Embedding Test String";

/// Validate required raw provider models without involving model cache middleware.
pub(crate) async fn validate_model_connectivity(
    config: &GraphRagConfig,
    requirements: &IndexWorkflowRequirements,
    factory: &dyn ModelFactory,
) -> Result<()> {
    for id in requirements.completion_models() {
        let model_config = required_model_config(&config.completion_models, id, "completion")?;
        let model = factory
            .create_completion(id, model_config, config.concurrent_requests)
            .map_err(|source| connectivity_error("completion", id, model_config, source))?;
        let response = model
            .complete(CompletionRequest::new(vec![ChatMessage::user(
                COMPLETION_CONNECTIVITY_TEXT.to_owned(),
            )]))
            .await
            .map_err(|source| connectivity_error("completion", id, model_config, source))?;
        let content = response
            .content()
            .map_err(|source| connectivity_error("completion", id, model_config, source))?;
        if content.trim().is_empty() {
            return Err(connectivity_error(
                "completion",
                id,
                model_config,
                std::io::Error::other("completion returned empty content"),
            ));
        }
    }

    for id in requirements.embedding_models() {
        let model_config = required_model_config(&config.embedding_models, id, "embedding")?;
        let model = factory
            .create_embedding(id, model_config, config.concurrent_requests)
            .map_err(|source| connectivity_error("embedding", id, model_config, source))?;
        let response = model
            .embed(EmbeddingRequest::new(vec![
                EMBEDDING_CONNECTIVITY_TEXT.to_owned(),
            ]))
            .await
            .map_err(|source| connectivity_error("embedding", id, model_config, source))?;
        let first = response.embeddings().next().ok_or_else(|| {
            connectivity_error(
                "embedding",
                id,
                model_config,
                std::io::Error::other("embedding returned no vectors"),
            )
        })?;
        if first.is_empty() {
            return Err(connectivity_error(
                "embedding",
                id,
                model_config,
                std::io::Error::other("embedding returned an empty embedding"),
            ));
        }
        if first.iter().any(|value| !value.is_finite()) {
            return Err(connectivity_error(
                "embedding",
                id,
                model_config,
                std::io::Error::other("embedding returned a non-finite value"),
            ));
        }
        validate_embedding_dimension(config, requirements, id, first.len())?;
    }
    Ok(())
}

fn required_model_config<'a>(
    models: &'a std::collections::BTreeMap<String, ModelConfig>,
    id: &str,
    kind: &'static str,
) -> Result<&'a ModelConfig> {
    models.get(id).ok_or_else(|| GraphLoomError::InvalidModel {
        model_id: id.to_owned(),
        message: format!("required {kind} model is not configured"),
    })
}

fn validate_embedding_dimension(
    config: &GraphRagConfig,
    requirements: &IndexWorkflowRequirements,
    model_id: &str,
    detected: usize,
) -> Result<()> {
    if !requirements.requires_vector_store() || model_id != config.embed_text.embedding_model_id {
        return Ok(());
    }
    let mut embedding_names = config.embed_text.names.iter().collect::<Vec<_>>();
    embedding_names.sort_unstable();
    for embedding_name in embedding_names {
        let configured = config.vector_store.schema_for(embedding_name).vector_size;
        if detected != configured {
            return Err(GraphLoomError::EmbeddingDimensionMismatch {
                model_id: model_id.to_owned(),
                embedding_name: embedding_name.clone(),
                configured,
                detected,
            });
        }
    }
    Ok(())
}

fn connectivity_error(
    kind: &'static str,
    id: &str,
    config: &ModelConfig,
    source: impl Error + Send + Sync + 'static,
) -> GraphLoomError {
    let secret = config.api_key.as_ref().map(ExposeSecret::expose_secret);
    let source = RedactedModelError::new(source, secret);
    GraphLoomError::ModelConnectivity {
        source: Box::new(ModelConnectivityError {
            model_kind: kind,
            model_id: id.to_owned(),
            provider_model: config.model.clone(),
            operation: if kind == "completion" {
                "completion connectivity check"
            } else {
                "embedding connectivity check"
            },
            provider: config.provider_type.clone(),
            base_url: config.effective_api_base(),
            source: Box::new(source),
        }),
    }
}

fn redact_value(value: &str, secret: Option<&str>) -> String {
    secret.filter(|secret| !secret.is_empty()).map_or_else(
        || value.to_owned(),
        |secret| value.replace(secret, "<redacted>"),
    )
}

struct RedactedModelError {
    message: String,
    source: Option<Box<Self>>,
}

impl RedactedModelError {
    fn new(source: impl Error + Send + Sync + 'static, secret: Option<&str>) -> Self {
        Self::from_error(&source, secret)
    }

    fn from_error(source: &(dyn Error + 'static), secret: Option<&str>) -> Self {
        let message = redact_value(&source.to_string(), secret);
        Self {
            message,
            source: source
                .source()
                .map(|source| Box::new(Self::from_error(source, secret))),
        }
    }
}

impl fmt::Debug for RedactedModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedactedModelError")
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for RedactedModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for RedactedModelError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use async_trait::async_trait;
    use graphloom_llm::{
        CompletionModel, CompletionRequest, CompletionResponse, EmbeddingModel, EmbeddingRequest,
        EmbeddingResponse, LlmError, MockCompletionModel, MockEmbeddingModel, ModelConfig,
    };
    use secrecy::ExposeSecret;

    use super::{
        ModelFactory, RedactedModelError, create_model_registry, validate_model_connectivity,
    };
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

    #[tokio::test]
    async fn test_should_validate_only_required_models_once_in_stable_order() {
        let mut config = connectivity_config();
        config
            .completion_models
            .insert("unused_bad".to_owned(), model_config("unused"));
        config
            .embedding_models
            .insert("unused_bad".to_owned(), model_config("unused"));
        let mut requirements = IndexWorkflowRequirements::default();
        requirements.require_completion_model("z_completion");
        requirements.require_completion_model("a_completion");
        requirements.require_completion_model("a_completion");
        requirements.require_embedding_model("z_embedding");
        requirements.require_embedding_model("a_embedding");
        requirements.require_embedding_model("a_embedding");
        requirements.require_vector_store();
        for id in ["z_completion", "a_completion"] {
            config
                .completion_models
                .insert(id.to_owned(), model_config(id));
        }
        for id in ["z_embedding", "a_embedding"] {
            config
                .embedding_models
                .insert(id.to_owned(), model_config(id));
        }
        config.embed_text.embedding_model_id = "a_embedding".to_owned();
        let factory = RecordingModelFactory::successful(3);

        validate_model_connectivity(&config, &requirements, &factory)
            .await
            .expect("required models should validate");

        assert_eq!(
            factory.events(),
            vec![
                "create completion a_completion",
                "completion a_completion: This is an LLM connectivity test. Say Hello World",
                "create completion z_completion",
                "completion z_completion: This is an LLM connectivity test. Say Hello World",
                "create embedding a_embedding",
                "embedding a_embedding: This is an LLM Embedding Test String",
                "create embedding z_embedding",
                "embedding z_embedding: This is an LLM Embedding Test String",
            ]
        );
    }

    #[tokio::test]
    async fn test_should_accept_any_non_empty_completion_and_matching_embedding() {
        let config = connectivity_config();
        let requirements = connectivity_requirements();
        let factory = RecordingModelFactory::successful(3);

        validate_model_connectivity(&config, &requirements, &factory)
            .await
            .expect("arbitrary non-empty completion should pass");
    }

    #[tokio::test]
    async fn test_should_reject_empty_completion_and_embedding_responses() {
        let config = connectivity_config();
        let mut completion_requirements = IndexWorkflowRequirements::default();
        completion_requirements.require_completion_model("completion");
        let empty_completion = RecordingModelFactory::new(
            CompletionBehavior::Text("  "),
            EmbeddingBehavior::Vectors(vec![vec![1.0; 3]]),
        );
        let error =
            validate_model_connectivity(&config, &completion_requirements, &empty_completion)
                .await
                .expect_err("blank completion should fail");
        assert!(error.to_string().contains("completion"));
        assert!(error.to_string().contains("empty content"));

        let mut embedding_requirements = IndexWorkflowRequirements::default();
        embedding_requirements.require_embedding_model("embedding");
        let no_vectors = RecordingModelFactory::new(
            CompletionBehavior::Text("hello"),
            EmbeddingBehavior::Vectors(Vec::new()),
        );
        let error = validate_model_connectivity(&config, &embedding_requirements, &no_vectors)
            .await
            .expect_err("missing vectors should fail");
        assert!(error.to_string().contains("embedding"));
        assert!(error.to_string().contains("no vectors"));

        let empty_vector = RecordingModelFactory::new(
            CompletionBehavior::Text("hello"),
            EmbeddingBehavior::Vectors(vec![Vec::new()]),
        );
        let error = validate_model_connectivity(&config, &embedding_requirements, &empty_vector)
            .await
            .expect_err("empty vector should fail");
        assert!(error.to_string().contains("empty embedding"));

        let failed_embedding = RecordingModelFactory::new(
            CompletionBehavior::Text("hello"),
            EmbeddingBehavior::Fail("embedding provider rejected secret-key"),
        );
        let error =
            validate_model_connectivity(&config, &embedding_requirements, &failed_embedding)
                .await
                .expect_err("embedding provider failure should fail");
        assert!(error.to_string().contains("embedding"));
        assert!(error.to_string().contains("embedding connectivity check"));
        assert!(!error.to_string().contains("secret-key"));
    }

    #[tokio::test]
    async fn test_should_report_model_ids_and_redact_secrets_on_provider_failure() {
        let config = connectivity_config();
        let secret = config.completion_models["completion"]
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret)
            .expect("api key");
        let mut requirements = IndexWorkflowRequirements::default();
        requirements.require_completion_model("completion");
        let factory = RecordingModelFactory::new(
            CompletionBehavior::Fail("authentication rejected secret-key"),
            EmbeddingBehavior::Vectors(vec![vec![1.0; 3]]),
        );

        let error = validate_model_connectivity(&config, &requirements, &factory)
            .await
            .expect_err("provider failure should fail validation");
        let message = error.to_string();

        assert!(message.contains("completion"));
        assert!(message.contains("completion connectivity check"));
        assert!(message.contains("provider model chat"));
        assert!(message.contains("provider openai"));
        assert!(message.contains("https://models.example/v1"));
        assert!(message.contains("<redacted>"));
        assert!(!message.contains(secret));
        let mut source = error.source();
        while let Some(cause) = source {
            assert!(!cause.to_string().contains(secret));
            source = cause.source();
        }
    }

    #[test]
    fn test_should_redact_api_key_from_every_provider_error_source() {
        let error = NestedProviderError {
            message: "outer provider error contains secret-key",
            source: LeafProviderError("inner provider error contains secret-key"),
        };
        let redacted = RedactedModelError::new(error, Some("secret-key"));

        assert!(!redacted.to_string().contains("secret-key"));
        assert!(redacted.to_string().contains("<redacted>"));
        let mut source = redacted.source();
        while let Some(cause) = source {
            assert!(!cause.to_string().contains("secret-key"));
            source = cause.source();
        }
    }

    #[tokio::test]
    async fn test_should_reject_dimension_mismatch_without_mutating_config() {
        let config = connectivity_config();
        let original_vector_store = config.vector_store.clone();
        let original_embedding_names = config.embed_text.names.clone();
        let requirements = connectivity_requirements();
        let factory = RecordingModelFactory::successful(2);

        let error = validate_model_connectivity(&config, &requirements, &factory)
            .await
            .expect_err("dimension mismatch should fail");

        assert!(error.to_string().contains("returned 2 dimensions"));
        assert!(error.to_string().contains("configured as 3"));
        assert_eq!(config.vector_store, original_vector_store);
        assert_eq!(config.embed_text.names, original_embedding_names);
    }

    fn connectivity_config() -> GraphRagConfig {
        let mut config = GraphRagConfig::default();
        config
            .completion_models
            .insert("completion".to_owned(), model_config("chat"));
        config
            .embedding_models
            .insert("embedding".to_owned(), model_config("embed"));
        config.embed_text.embedding_model_id = "embedding".to_owned();
        config.embed_text.names = vec!["entity_description".to_owned()];
        config.vector_store.vector_size = 3;
        config
    }

    fn connectivity_requirements() -> IndexWorkflowRequirements {
        let mut requirements = IndexWorkflowRequirements::default();
        requirements.require_completion_model("completion");
        requirements.require_embedding_model("embedding");
        requirements.require_vector_store();
        requirements
    }

    fn model_config(model: &str) -> ModelConfig {
        serde_json::from_value(serde_json::json!({
            "model_provider": "openai",
            "model": model,
            "api_key": "secret-key",
            "api_base": "https://models.example/v1",
        }))
        .expect("model config")
    }

    #[derive(Debug, Clone, Copy)]
    enum CompletionBehavior {
        Text(&'static str),
        Fail(&'static str),
    }

    #[derive(Debug, Clone)]
    enum EmbeddingBehavior {
        Vectors(Vec<Vec<f32>>),
        Fail(&'static str),
    }

    #[derive(Debug)]
    struct NestedProviderError {
        message: &'static str,
        source: LeafProviderError,
    }

    impl std::fmt::Display for NestedProviderError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str(self.message)
        }
    }

    impl Error for NestedProviderError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.source)
        }
    }

    #[derive(Debug)]
    struct LeafProviderError(&'static str);

    impl std::fmt::Display for LeafProviderError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str(self.0)
        }
    }

    impl Error for LeafProviderError {}

    #[derive(Debug)]
    struct RecordingModelFactory {
        events: Arc<Mutex<Vec<String>>>,
        completion: CompletionBehavior,
        embedding: EmbeddingBehavior,
    }

    impl RecordingModelFactory {
        fn new(completion: CompletionBehavior, embedding: EmbeddingBehavior) -> Self {
            Self {
                events: Arc::new(Mutex::new(Vec::new())),
                completion,
                embedding,
            }
        }

        fn successful(dimension: usize) -> Self {
            Self::new(
                CompletionBehavior::Text("A friendly response, not an exact phrase"),
                EmbeddingBehavior::Vectors(vec![vec![1.0; dimension]]),
            )
        }

        fn events(&self) -> Vec<String> {
            self.events.lock().expect("events lock").clone()
        }
    }

    impl ModelFactory for RecordingModelFactory {
        fn create_completion(
            &self,
            id: &str,
            _config: &ModelConfig,
            _concurrent_requests: usize,
        ) -> Result<Arc<dyn CompletionModel>> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("create completion {id}"));
            Ok(Arc::new(RecordingCompletionModel {
                id: id.to_owned(),
                events: Arc::clone(&self.events),
                behavior: self.completion,
            }))
        }

        fn create_embedding(
            &self,
            id: &str,
            _config: &ModelConfig,
            _concurrent_requests: usize,
        ) -> Result<Arc<dyn EmbeddingModel>> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("create embedding {id}"));
            Ok(Arc::new(RecordingEmbeddingModel {
                id: id.to_owned(),
                events: Arc::clone(&self.events),
                behavior: self.embedding.clone(),
            }))
        }
    }

    #[derive(Debug)]
    struct RecordingCompletionModel {
        id: String,
        events: Arc<Mutex<Vec<String>>>,
        behavior: CompletionBehavior,
    }

    #[async_trait]
    impl CompletionModel for RecordingCompletionModel {
        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> graphloom_llm::Result<CompletionResponse> {
            let text = request
                .messages
                .first()
                .map(|message| message.content.as_str())
                .unwrap_or_default();
            self.events
                .lock()
                .expect("events lock")
                .push(format!("completion {}: {text}", self.id));
            match self.behavior {
                CompletionBehavior::Text(content) => {
                    Ok(CompletionResponse::text_for_test(&self.id, content))
                }
                CompletionBehavior::Fail(message) => Err(LlmError::InvalidResponse {
                    model_instance: self.id.clone(),
                    operation: "completion",
                    message: message.to_owned(),
                }),
            }
        }
    }

    #[derive(Debug)]
    struct RecordingEmbeddingModel {
        id: String,
        events: Arc<Mutex<Vec<String>>>,
        behavior: EmbeddingBehavior,
    }

    #[async_trait]
    impl EmbeddingModel for RecordingEmbeddingModel {
        async fn embed(
            &self,
            request: EmbeddingRequest,
        ) -> graphloom_llm::Result<EmbeddingResponse> {
            let text = request
                .input
                .first()
                .map(String::as_str)
                .unwrap_or_default();
            self.events
                .lock()
                .expect("events lock")
                .push(format!("embedding {}: {text}", self.id));
            match &self.behavior {
                EmbeddingBehavior::Vectors(vectors) => Ok(EmbeddingResponse::vectors_for_test(
                    &self.id,
                    vectors.clone(),
                )),
                EmbeddingBehavior::Fail(message) => Err(LlmError::InvalidResponse {
                    model_instance: self.id.clone(),
                    operation: "embedding",
                    message: (*message).to_owned(),
                }),
            }
        }
    }
}
