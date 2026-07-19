//! Reusable, snapshot-oriented Query engine.

use std::{
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
};

use dashmap::DashMap;
use tokio::sync::OnceCell;

use super::{
    BasicQueryRuntime, DriftQueryRuntime, GlobalQueryRuntime, LocalQueryRuntime,
    QueryCallbackChain, QueryCallbacks, QueryEventStream, QueryOptions, QueryResult,
    QueryRuntimeFactory, Result, SearchMethod,
    basic::{basic_search, basic_search_streaming},
    drift::{drift_search, drift_search_streaming},
    global::{global_search, global_search_streaming},
    local::{local_search, local_search_streaming},
    runtime::{
        validate_basic_requirements, validate_drift_requirements, validate_global_requirements,
        validate_local_requirements,
    },
};
use crate::{
    GraphRagConfig,
    project::LoadedProject,
    runtime::{DefaultModelFactory, ModelFactory},
};

#[derive(Debug, Clone, Eq)]
struct QueryResourceKey {
    data_root: PathBuf,
    community_level: i64,
}

impl PartialEq for QueryResourceKey {
    fn eq(&self, other: &Self) -> bool {
        self.data_root == other.data_root && self.community_level == other.community_level
    }
}

impl Hash for QueryResourceKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.data_root.hash(state);
        self.community_level.hash(state);
    }
}

/// Long-lived, concurrency-safe Query resource cache.
///
/// Each method/data-key becomes a snapshot when that key is prepared on its first query. Models,
/// tokenizers, prompts, adapted Parquet data, vector connections, and validated vector schemas are
/// then reused for that key. Later changes to its index files are not observed; construct a new
/// engine to reload them.
///
/// Request-specific query text, response type, callbacks, conversation history, usage counters,
/// streaming state, and traversal state are never cached. The engine can therefore be shared as
/// an [`Arc`] and queried concurrently.
#[derive(Debug)]
pub struct QueryEngine {
    project: LoadedProject,
    model_factory: Arc<dyn ModelFactory>,
    basic: DashMap<QueryResourceKey, Arc<OnceCell<BasicQueryRuntime>>>,
    local: DashMap<QueryResourceKey, Arc<OnceCell<LocalQueryRuntime>>>,
    global: DashMap<QueryResourceKey, Arc<OnceCell<GlobalQueryRuntime>>>,
    drift: DashMap<QueryResourceKey, Arc<OnceCell<DriftQueryRuntime>>>,
}

impl QueryEngine {
    /// Create a lazy Query engine for `project_root`.
    ///
    /// The method-specific index is loaded on its first query, so creating an engine intended only
    /// for Basic Search does not read Local, Global, or DRIFT tables and prompts.
    ///
    /// # Errors
    ///
    /// Returns the existing typed configuration/path error when the project cannot be loaded.
    #[allow(
        clippy::unused_async,
        reason = "the public lifecycle is async so future eager preparation and callers share one \
                  stable load contract"
    )]
    pub async fn load(
        config: GraphRagConfig,
        project_root: impl AsRef<Path>,
    ) -> crate::Result<Self> {
        Self::load_with_factory(config, project_root, Arc::new(DefaultModelFactory)).await
    }

    #[allow(
        clippy::unused_async,
        reason = "test and production constructors intentionally share the async load contract"
    )]
    pub(crate) async fn load_with_factory(
        config: GraphRagConfig,
        project_root: impl AsRef<Path>,
        model_factory: Arc<dyn ModelFactory>,
    ) -> crate::Result<Self> {
        let project = LoadedProject::from_config(project_root.as_ref(), config)?;
        Ok(Self {
            project,
            model_factory,
            basic: DashMap::new(),
            local: DashMap::new(),
            global: DashMap::new(),
            drift: DashMap::new(),
        })
    }

    /// Execute one request against this engine's index snapshot.
    ///
    /// `options.project_root` is retained for compatibility with the one-shot API; this engine's
    /// root, selected at [`Self::load`], owns relative data and prompt resolution.
    ///
    /// # Errors
    ///
    /// Returns a typed Query error for invalid request options, missing snapshot resources, or
    /// model/provider failures.
    pub async fn query(&self, options: QueryOptions) -> crate::Result<QueryResult> {
        match options.method {
            SearchMethod::Basic => {
                let runtime = self.basic_runtime(&options).await?;
                Ok(basic_search(runtime, &options.query, &options.response_type).await?)
            }
            SearchMethod::Local => {
                let runtime = self.local_runtime(&options).await?;
                Ok(local_search(
                    runtime,
                    &options.query,
                    &options.response_type,
                    options.conversation_history.as_ref(),
                )
                .await?)
            }
            SearchMethod::Global => {
                let runtime = self.global_runtime(&options).await?;
                Ok(global_search(runtime, &options.query, &options.response_type).await?)
            }
            SearchMethod::Drift => {
                let runtime = self.drift_runtime(&options).await?;
                Ok(drift_search(runtime, &options.query, &options.response_type).await?)
            }
        }
    }

    /// Start one streaming request against this engine's index snapshot.
    ///
    /// # Errors
    ///
    /// Returns a typed Query error when resource preparation or the provider stream handshake
    /// fails.
    pub async fn query_stream(&self, options: QueryOptions) -> crate::Result<QueryEventStream> {
        match options.method {
            SearchMethod::Basic => {
                let runtime = self.basic_runtime(&options).await?;
                Ok(basic_search_streaming(runtime, &options.query, &options.response_type).await?)
            }
            SearchMethod::Local => {
                let runtime = self.local_runtime(&options).await?;
                Ok(local_search_streaming(
                    runtime,
                    &options.query,
                    &options.response_type,
                    options.conversation_history.as_ref(),
                )
                .await?)
            }
            SearchMethod::Global => {
                let runtime = self.global_runtime(&options).await?;
                Ok(
                    global_search_streaming(runtime, &options.query, &options.response_type)
                        .await?,
                )
            }
            SearchMethod::Drift => {
                let runtime = self.drift_runtime(&options).await?;
                Ok(drift_search_streaming(runtime, &options.query, &options.response_type).await?)
            }
        }
    }

    async fn basic_runtime(&self, options: &QueryOptions) -> Result<BasicQueryRuntime> {
        validate_basic_requirements(&self.project, options)?;
        let key = self.key(options, false);
        let cell = cache_cell(&self.basic, key);
        let prepared = cell
            .get_or_try_init(|| async {
                let resource_options = resource_options(options);
                QueryRuntimeFactory::build_basic_with_factory(
                    &self.project,
                    &resource_options,
                    self.model_factory.as_ref(),
                )
                .await
            })
            .await?;
        let mut runtime = prepared.clone();
        runtime.callbacks = request_callbacks(options);
        Ok(runtime)
    }

    async fn local_runtime(&self, options: &QueryOptions) -> Result<LocalQueryRuntime> {
        validate_local_requirements(&self.project, options)?;
        let key = self.key(options, true);
        let cell = cache_cell(&self.local, key);
        let prepared = cell
            .get_or_try_init(|| async {
                let resource_options = resource_options(options);
                QueryRuntimeFactory::build_local_with_factory(
                    &self.project,
                    &resource_options,
                    self.model_factory.as_ref(),
                )
                .await
            })
            .await?;
        let mut runtime = prepared.clone();
        runtime.callbacks = request_callbacks(options);
        Ok(runtime)
    }

    async fn global_runtime(&self, options: &QueryOptions) -> Result<GlobalQueryRuntime> {
        validate_global_requirements(&self.project, options)?;
        let key = self.key(options, true);
        let cell = cache_cell(&self.global, key);
        let prepared = cell
            .get_or_try_init(|| async {
                let resource_options = resource_options(options);
                QueryRuntimeFactory::build_global_with_factory(
                    &self.project,
                    &resource_options,
                    self.model_factory.as_ref(),
                )
                .await
            })
            .await?;
        let mut runtime = prepared.clone();
        runtime.callbacks = request_callbacks(options);
        runtime.dynamic_community_selection = options.dynamic_community_selection;
        Ok(runtime)
    }

    async fn drift_runtime(&self, options: &QueryOptions) -> Result<DriftQueryRuntime> {
        validate_drift_requirements(&self.project, options)?;
        let key = self.key(options, true);
        let cell = cache_cell(&self.drift, key);
        let prepared = cell
            .get_or_try_init(|| async {
                let resource_options = resource_options(options);
                QueryRuntimeFactory::build_drift_with_factory(
                    &self.project,
                    &resource_options,
                    self.model_factory.as_ref(),
                )
                .await
            })
            .await?;
        let mut runtime = prepared.clone();
        runtime.callbacks = request_callbacks(options);
        Ok(runtime)
    }

    fn key(&self, options: &QueryOptions, include_community_level: bool) -> QueryResourceKey {
        let data_root = options.data_dir.as_ref().map_or_else(
            || self.project.paths.output_dir.clone(),
            |path| {
                if path.is_absolute() {
                    path.clone()
                } else {
                    self.project.root.join(path)
                }
            },
        );
        QueryResourceKey {
            data_root,
            community_level: if include_community_level {
                options.community_level
            } else {
                0
            },
        }
    }
}

fn cache_cell<T>(
    cache: &DashMap<QueryResourceKey, Arc<OnceCell<T>>>,
    key: QueryResourceKey,
) -> Arc<OnceCell<T>> {
    Arc::clone(
        cache
            .entry(key)
            .or_insert_with(|| Arc::new(OnceCell::new()))
            .value(),
    )
}

fn resource_options(options: &QueryOptions) -> QueryOptions {
    let mut resource_options = options.clone();
    resource_options.callbacks.clear();
    resource_options.conversation_history = None;
    resource_options
}

fn request_callbacks(options: &QueryOptions) -> Arc<dyn QueryCallbacks> {
    Arc::new(QueryCallbackChain::new(options.callbacks.clone()))
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
    use graphloom_storage::{ParquetTableProvider, TableProvider};
    use graphloom_vectors::{LanceDbVectorStore, VectorDocument, VectorStore, VectorStoreConfig};
    use polars_core::prelude::{DataFrame, NamedFrom, Series};

    use super::QueryEngine;
    use crate::{
        GraphRagConfig, Result, TEXT_UNIT_TEXT_EMBEDDING,
        query::{QueryOptions, SearchMethod},
        runtime::ModelFactory,
        test_support::CanonicalTempDir,
    };

    #[derive(Debug, Default)]
    struct CountingModelFactory {
        completions: AtomicUsize,
        embeddings: AtomicUsize,
    }

    impl ModelFactory for CountingModelFactory {
        fn create_completion(
            &self,
            id: &str,
            _config: &ModelConfig,
            _concurrent_requests: usize,
        ) -> Result<Arc<dyn CompletionModel>> {
            self.completions.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(MockCompletionModel::new(
                id,
                vec!["answer".to_owned()],
            )))
        }

        fn create_embedding(
            &self,
            id: &str,
            _config: &ModelConfig,
            _concurrent_requests: usize,
        ) -> Result<Arc<dyn EmbeddingModel>> {
            self.embeddings.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(MockEmbeddingModel::new(id, vec![1.0, 0.0])))
        }
    }

    #[tokio::test]
    async fn test_should_prepare_basic_models_tokenizer_tables_and_vector_connection_once() {
        let project = CanonicalTempDir::new();
        let output = project.path().join("output");
        let provider = ParquetTableProvider::new(&output).expect("Parquet provider");
        provider
            .write_dataframe(
                "text_units",
                DataFrame::new(
                    1,
                    vec![
                        Series::new("id".into(), ["unit"]).into(),
                        Series::new("text".into(), ["source"]).into(),
                    ],
                )
                .expect("text units"),
            )
            .await
            .expect("write text units");

        let mut vector_config = VectorStoreConfig::default();
        vector_config.db_uri = output.join("lancedb").to_string_lossy().into_owned();
        vector_config.vector_size = 2;
        let store = LanceDbVectorStore::connect(&vector_config)
            .await
            .expect("vector store");
        let schema = vector_config.schema_for(TEXT_UNIT_TEXT_EMBEDDING);
        store
            .upsert_documents(
                &schema,
                &[VectorDocument {
                    id: "unit".to_owned(),
                    vector: vec![1.0, 0.0],
                }],
            )
            .await
            .expect("seed vector");

        let mut config = GraphRagConfig::default();
        config.output_storage.base_dir = output.to_string_lossy().into_owned();
        config.vector_store = vector_config;
        let model_config: ModelConfig = serde_json::from_value(serde_json::json!({
            "model_provider": "openai",
            "model": "unused-by-counting-factory",
        }))
        .expect("model config");
        config.completion_models.insert(
            config.basic_search.completion_model_id.clone(),
            model_config.clone(),
        );
        config
            .embedding_models
            .insert(config.basic_search.embedding_model_id.clone(), model_config);
        let factory = Arc::new(CountingModelFactory::default());
        let factory_trait: Arc<dyn ModelFactory> = factory.clone();
        let engine = QueryEngine::load_with_factory(config, project.path(), factory_trait)
            .await
            .expect("Query engine");
        let options = QueryOptions::new(
            project.path().to_path_buf(),
            "question".to_owned(),
            SearchMethod::Basic,
        );

        let first = engine.basic_runtime(&options).await.expect("first runtime");
        let second = engine
            .basic_runtime(&options)
            .await
            .expect("second runtime");

        assert_eq!(factory.completions.load(Ordering::SeqCst), 1);
        assert_eq!(factory.embeddings.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&first.basic_context, &second.basic_context));
        assert!(Arc::ptr_eq(
            &first.completion_model,
            &second.completion_model
        ));
        assert!(Arc::ptr_eq(
            &first.basic_context.tokenizer,
            &second.basic_context.tokenizer
        ));
        assert!(Arc::ptr_eq(
            &first.basic_context.vector_store,
            &second.basic_context.vector_store
        ));
        assert!(!Arc::ptr_eq(&first.callbacks, &second.callbacks));

        let mut static_global = options.clone();
        static_global.method = SearchMethod::Global;
        let mut dynamic_global = static_global.clone();
        dynamic_global.dynamic_community_selection = true;
        assert_eq!(
            engine.key(&static_global, true),
            engine.key(&dynamic_global, true),
            "request-local dynamic selection must not create a second resource cache"
        );
    }
}
