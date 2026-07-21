//! Reusable, snapshot-oriented Query engine.

use std::{
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum QueryDataRootKey {
    Canonical(PathBuf),
    Unresolved(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct QueryResourceKey {
    data_root: QueryDataRootKey,
    community_level: i64,
    dynamic_community_selection: bool,
}

impl QueryResourceKey {
    fn is_cacheable(&self) -> bool {
        matches!(self.data_root, QueryDataRootKey::Canonical(_))
    }
}

/// Long-lived, concurrency-safe Query resource cache.
///
/// Each method/data-key becomes a snapshot when that key is prepared on its first query. Global
/// static and dynamic selection use distinct data snapshots because their report adaptation
/// differs. Models, tokenizers, prompts, adapted Parquet data, vector connections, and validated
/// vector schemas are then reused for that key. Switching Global selection mode can prepare a
/// second snapshot. Later changes to prepared index files are not observed; construct a new engine
/// to reload them.
///
/// Data overrides that resolve to the same directory share a snapshot. An override that cannot be
/// resolved bypasses the long-lived resource cache and proceeds through normal runtime loading,
/// where the existing typed loading error is returned. If that loading happens to succeed, its
/// resources remain request-local; the next request resolves the path again and only a canonical
/// path can establish a snapshot.
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
    /// `options.project_root` must resolve to the same existing directory passed to
    /// [`Self::load`]. This engine's root owns relative data and prompt resolution.
    ///
    /// # Errors
    ///
    /// Returns a typed Query error for invalid request options, missing snapshot resources, or
    /// model/provider failures.
    pub async fn query(&self, options: QueryOptions) -> crate::Result<QueryResult> {
        self.validate_project_root(&options).await?;
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
        self.validate_project_root(&options).await?;
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
        let key = self.key(options, false, false).await;
        self.basic_runtime_for_key(options, key).await
    }

    async fn basic_runtime_for_key(
        &self,
        options: &QueryOptions,
        key: QueryResourceKey,
    ) -> Result<BasicQueryRuntime> {
        let mut runtime = if key.is_cacheable() {
            let cell = cache_cell(&self.basic, key);
            cell.get_or_try_init(|| async {
                let resource_options = resource_options(options);
                QueryRuntimeFactory::build_basic_with_factory(
                    &self.project,
                    &resource_options,
                    self.model_factory.as_ref(),
                )
                .await
            })
            .await?
            .clone()
        } else {
            let resource_options = resource_options(options);
            QueryRuntimeFactory::build_basic_with_factory(
                &self.project,
                &resource_options,
                self.model_factory.as_ref(),
            )
            .await?
        };
        runtime.callbacks = request_callbacks(options);
        Ok(runtime)
    }

    async fn local_runtime(&self, options: &QueryOptions) -> Result<LocalQueryRuntime> {
        validate_local_requirements(&self.project, options)?;
        let key = self.key(options, true, false).await;
        let mut runtime = if key.is_cacheable() {
            let cell = cache_cell(&self.local, key);
            cell.get_or_try_init(|| async {
                let resource_options = resource_options(options);
                QueryRuntimeFactory::build_local_with_factory(
                    &self.project,
                    &resource_options,
                    self.model_factory.as_ref(),
                )
                .await
            })
            .await?
            .clone()
        } else {
            let resource_options = resource_options(options);
            QueryRuntimeFactory::build_local_with_factory(
                &self.project,
                &resource_options,
                self.model_factory.as_ref(),
            )
            .await?
        };
        runtime.callbacks = request_callbacks(options);
        Ok(runtime)
    }

    async fn global_runtime(&self, options: &QueryOptions) -> Result<GlobalQueryRuntime> {
        validate_global_requirements(&self.project, options)?;
        let key = self
            .key(options, true, options.dynamic_community_selection)
            .await;
        let mut runtime = if key.is_cacheable() {
            let cell = cache_cell(&self.global, key);
            cell.get_or_try_init(|| async {
                let resource_options = resource_options(options);
                QueryRuntimeFactory::build_global_with_factory(
                    &self.project,
                    &resource_options,
                    self.model_factory.as_ref(),
                )
                .await
            })
            .await?
            .clone()
        } else {
            let resource_options = resource_options(options);
            QueryRuntimeFactory::build_global_with_factory(
                &self.project,
                &resource_options,
                self.model_factory.as_ref(),
            )
            .await?
        };
        runtime.callbacks = request_callbacks(options);
        Ok(runtime)
    }

    async fn drift_runtime(&self, options: &QueryOptions) -> Result<DriftQueryRuntime> {
        validate_drift_requirements(&self.project, options)?;
        let key = self.key(options, true, false).await;
        let mut runtime = if key.is_cacheable() {
            let cell = cache_cell(&self.drift, key);
            cell.get_or_try_init(|| async {
                let resource_options = resource_options(options);
                QueryRuntimeFactory::build_drift_with_factory(
                    &self.project,
                    &resource_options,
                    self.model_factory.as_ref(),
                )
                .await
            })
            .await?
            .clone()
        } else {
            let resource_options = resource_options(options);
            QueryRuntimeFactory::build_drift_with_factory(
                &self.project,
                &resource_options,
                self.model_factory.as_ref(),
            )
            .await?
        };
        runtime.callbacks = request_callbacks(options);
        Ok(runtime)
    }

    async fn validate_project_root(&self, options: &QueryOptions) -> Result<()> {
        let engine_root = canonical_query_root(
            &self.project.root,
            options.method,
            "resolve QueryEngine project root",
        )
        .await?;
        let options_root = canonical_query_root(
            &options.project_root,
            options.method,
            "resolve QueryOptions project root",
        )
        .await?;
        if engine_root != options_root {
            return Err(super::QueryError::InvalidQueryConfig {
                method: options.method,
                operation: "validate QueryEngine project root",
                message: "QueryOptions project_root does not match the QueryEngine root".to_owned(),
            });
        }
        Ok(())
    }

    async fn key(
        &self,
        options: &QueryOptions,
        include_community_level: bool,
        dynamic_community_selection: bool,
    ) -> QueryResourceKey {
        let unresolved_data_root = options.data_dir.as_ref().map_or_else(
            || self.project.paths.output_dir.clone(),
            |path| {
                // LoadedProject roots are absolute; joining here preserves `..` components and
                // their filesystem lookup semantics for the unresolved-key fallback below.
                if path.is_absolute() {
                    path.clone()
                } else {
                    self.project.root.join(path)
                }
            },
        );
        let canonicalized = tokio::fs::canonicalize(&unresolved_data_root).await;
        let data_root = classify_data_root(unresolved_data_root, canonicalized);
        QueryResourceKey {
            data_root,
            community_level: if include_community_level {
                options.community_level
            } else {
                0
            },
            dynamic_community_selection,
        }
    }
}

fn classify_data_root(
    unresolved: PathBuf,
    canonicalized: std::io::Result<PathBuf>,
) -> QueryDataRootKey {
    match canonicalized {
        Ok(path) => QueryDataRootKey::Canonical(path),
        Err(_) => QueryDataRootKey::Unresolved(unresolved),
    }
}

async fn canonical_query_root(
    root: &Path,
    method: SearchMethod,
    operation: &'static str,
) -> Result<PathBuf> {
    let canonical = tokio::fs::canonicalize(root).await.map_err(|source| {
        super::QueryError::InvalidQueryConfig {
            method,
            operation,
            message: format!("project root must be an existing directory: {source}"),
        }
    })?;
    let metadata = tokio::fs::metadata(&canonical).await.map_err(|source| {
        super::QueryError::InvalidQueryConfig {
            method,
            operation,
            message: format!("cannot inspect project root: {source}"),
        }
    })?;
    if !metadata.is_dir() {
        return Err(super::QueryError::InvalidQueryConfig {
            method,
            operation,
            message: "project root must be a directory".to_owned(),
        });
    }
    Ok(canonical)
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
    use std::{
        path::{Path, PathBuf},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use graphloom_llm::{
        CompletionModel, EmbeddingModel, MockCompletionModel, MockEmbeddingModel, ModelConfig,
    };
    use graphloom_storage::{ParquetTableProvider, TableProvider};
    use graphloom_vectors::{LanceDbVectorStore, VectorDocument, VectorStore, VectorStoreConfig};
    use polars_core::prelude::{DataFrame, NamedFrom, Series};

    use super::{QueryDataRootKey, QueryEngine, classify_data_root};
    use crate::{
        GraphLoomError, GraphRagConfig, Result, TEXT_UNIT_TEXT_EMBEDDING,
        query::{QueryCallbacks, QueryContext, QueryError, QueryOptions, SearchMethod},
        runtime::ModelFactory,
        test_support::CanonicalTempDir,
    };

    #[derive(Debug, Default)]
    struct CountingModelFactory {
        completions: AtomicUsize,
        embeddings: AtomicUsize,
    }

    #[derive(Debug, Default)]
    struct CountingCallbacks {
        calls: AtomicUsize,
    }

    impl QueryCallbacks for CountingCallbacks {
        fn on_context(&self, _context: &QueryContext) {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
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
                vec!["answer".to_owned(), "answer".to_owned()],
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

    async fn seeded_basic_project() -> (CanonicalTempDir, GraphRagConfig) {
        let project = CanonicalTempDir::new();
        let output = project.path().join("output");
        write_basic_text_units(&output).await;

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
        (project, config)
    }

    async fn write_basic_text_units(root: &Path) {
        let provider = ParquetTableProvider::new(root).expect("Parquet provider");
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
    }

    async fn engine_with_factory(
        config: GraphRagConfig,
        project: &CanonicalTempDir,
    ) -> (QueryEngine, Arc<CountingModelFactory>) {
        let factory = Arc::new(CountingModelFactory::default());
        let factory_trait: Arc<dyn ModelFactory> = factory.clone();
        let engine = QueryEngine::load_with_factory(config, project.path(), factory_trait)
            .await
            .expect("Query engine");
        (engine, factory)
    }

    fn basic_options(project: &CanonicalTempDir, data_dir: &str) -> QueryOptions {
        let mut options = QueryOptions::new(
            project.path().to_path_buf(),
            "question".to_owned(),
            SearchMethod::Basic,
        );
        options.data_dir = Some(PathBuf::from(data_dir));
        options
    }

    fn missing_table_fields(error: &GraphLoomError) -> (SearchMethod, &'static str, &'static str) {
        match error {
            GraphLoomError::Query(source) => match source.as_ref() {
                QueryError::MissingQueryTable {
                    method,
                    operation,
                    table,
                } => (*method, operation, table),
                other => panic!("expected MissingQueryTable, got {other:?}"),
            },
            other => panic!("expected Query error, got {other:?}"),
        }
    }

    #[test]
    fn test_should_preserve_unresolved_path_after_canonicalization_failure() {
        let original = PathBuf::from("missing/../output");
        let key = classify_data_root(
            original.clone(),
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "synthetic failure",
            )),
        );

        assert_eq!(key, QueryDataRootKey::Unresolved(original));
        assert_ne!(
            QueryDataRootKey::Canonical(PathBuf::from("output")),
            QueryDataRootKey::Unresolved(PathBuf::from("output")),
        );
    }

    #[tokio::test]
    async fn test_should_prepare_basic_models_tokenizer_tables_and_vector_connection_once() {
        let (project, config) = seeded_basic_project().await;
        let (engine, factory) = engine_with_factory(config, &project).await;
        let options = QueryOptions::new(
            project.path().to_path_buf(),
            "question".to_owned(),
            SearchMethod::Basic,
        );

        let (first, second, third, fourth) = tokio::join!(
            engine.basic_runtime(&options),
            engine.basic_runtime(&options),
            engine.basic_runtime(&options),
            engine.basic_runtime(&options),
        );
        let first = first.expect("first concurrent runtime");
        let second = second.expect("second concurrent runtime");
        let third = third.expect("third concurrent runtime");
        let fourth = fourth.expect("fourth concurrent runtime");

        assert_eq!(factory.completions.load(Ordering::SeqCst), 1);
        assert_eq!(factory.embeddings.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&first.basic_context, &second.basic_context));
        assert!(Arc::ptr_eq(&first.basic_context, &third.basic_context));
        assert!(Arc::ptr_eq(&first.basic_context, &fourth.basic_context));
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
        let static_key = engine.key(&static_global, true, false).await;
        let dynamic_key = engine.key(&dynamic_global, true, true).await;
        assert!(static_key.is_cacheable());
        assert!(dynamic_key.is_cacheable());
        assert_ne!(
            static_key, dynamic_key,
            "Global static and dynamic selection require distinct data snapshots"
        );
        let mut other_level = static_global.clone();
        other_level.community_level += 1;
        let other_level_key = engine.key(&other_level, true, false).await;
        assert!(other_level_key.is_cacheable());
        assert_ne!(
            engine.key(&static_global, true, false).await,
            other_level_key,
            "Global community levels require distinct data snapshots"
        );
        engine
            .query(options.clone())
            .await
            .expect("canonical engine root");
        let mut dotted_options = options;
        dotted_options.project_root = project.path().join(".");
        engine
            .query(dotted_options)
            .await
            .expect("equivalent dotted engine root");
        assert_eq!(factory.completions.load(Ordering::SeqCst), 1);
        assert_eq!(factory.embeddings.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_should_share_snapshot_for_canonical_data_root_aliases() {
        let (project, config) = seeded_basic_project().await;
        let (engine, factory) = engine_with_factory(config, &project).await;
        let direct = basic_options(&project, "output");
        let dotted = basic_options(&project, "./output");
        let mut absolute = direct.clone();
        absolute.data_dir = Some(project.path().join("output"));

        let direct_key = engine.key(&direct, false, false).await;
        assert_eq!(direct_key, engine.key(&dotted, false, false).await);
        assert_eq!(direct_key, engine.key(&absolute, false, false).await);

        let direct_runtime = engine
            .basic_runtime(&direct)
            .await
            .expect("direct data runtime");
        let dotted_runtime = engine
            .basic_runtime(&dotted)
            .await
            .expect("dotted data runtime");
        let absolute_runtime = engine
            .basic_runtime(&absolute)
            .await
            .expect("absolute data runtime");

        assert!(Arc::ptr_eq(
            &direct_runtime.basic_context,
            &dotted_runtime.basic_context,
        ));
        assert!(Arc::ptr_eq(
            &direct_runtime.basic_context,
            &absolute_runtime.basic_context,
        ));
        assert_eq!(factory.completions.load(Ordering::SeqCst), 1);
        assert_eq!(factory.embeddings.load(Ordering::SeqCst), 1);
        assert_eq!(engine.basic.len(), 1);
    }

    #[tokio::test]
    async fn test_should_isolate_missing_nested_data_root_from_warm_snapshot() {
        let (project, config) = seeded_basic_project().await;
        let (engine, factory) = engine_with_factory(config.clone(), &project).await;
        engine
            .query(basic_options(&project, "output"))
            .await
            .expect("warm valid output snapshot");

        let callbacks = Arc::new(CountingCallbacks::default());
        let mut invalid = basic_options(&project, "missing-sub/output");
        invalid.callbacks.push(callbacks.clone());
        let warm_error = engine
            .query(invalid.clone())
            .await
            .expect_err("missing nested directory must not reuse snapshot");

        let (fresh_engine, fresh_factory) = engine_with_factory(config, &project).await;
        let fresh_error = fresh_engine
            .query(invalid)
            .await
            .expect_err("fresh engine must reject missing nested directory");

        assert_eq!(
            missing_table_fields(&warm_error),
            (
                SearchMethod::Basic,
                "open Query table directory",
                "text_units"
            )
        );
        assert_eq!(
            missing_table_fields(&warm_error),
            missing_table_fields(&fresh_error)
        );
        assert_eq!(callbacks.calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.completions.load(Ordering::SeqCst), 1);
        assert_eq!(factory.embeddings.load(Ordering::SeqCst), 1);
        assert_eq!(fresh_factory.completions.load(Ordering::SeqCst), 0);
        assert_eq!(fresh_factory.embeddings.load(Ordering::SeqCst), 0);
        let reused = engine
            .basic_runtime(&basic_options(&project, "output"))
            .await
            .expect("valid snapshot remains reusable");
        assert_eq!(reused.basic_context.text_units.len(), 1);
        assert_eq!(factory.completions.load(Ordering::SeqCst), 1);
        assert_eq!(factory.embeddings.load(Ordering::SeqCst), 1);
        assert_eq!(engine.basic.len(), 1);
    }

    #[tokio::test]
    async fn test_should_isolate_non_directory_ancestor_from_warm_snapshot() {
        let (project, config) = seeded_basic_project().await;
        tokio::fs::write(project.path().join("not-a-directory"), b"file")
            .await
            .expect("create regular file");
        let (engine, factory) = engine_with_factory(config.clone(), &project).await;
        engine
            .query(basic_options(&project, "output"))
            .await
            .expect("warm valid output snapshot");

        let invalid = basic_options(&project, "not-a-directory/child");
        let warm_error = engine
            .query(invalid.clone())
            .await
            .expect_err("non-directory ancestor must not reuse snapshot");
        let (fresh_engine, fresh_factory) = engine_with_factory(config, &project).await;
        let fresh_error = fresh_engine
            .query(invalid)
            .await
            .expect_err("fresh engine must reject non-directory ancestor");

        assert_eq!(
            missing_table_fields(&warm_error),
            (
                SearchMethod::Basic,
                "open Query table directory",
                "text_units"
            )
        );
        assert_eq!(
            missing_table_fields(&warm_error),
            missing_table_fields(&fresh_error)
        );
        assert_eq!(factory.completions.load(Ordering::SeqCst), 1);
        assert_eq!(factory.embeddings.load(Ordering::SeqCst), 1);
        assert_eq!(fresh_factory.completions.load(Ordering::SeqCst), 0);
        assert_eq!(fresh_factory.embeddings.load(Ordering::SeqCst), 0);
        assert_eq!(engine.basic.len(), 1);
        assert!(fresh_engine.basic.is_empty());
    }

    #[tokio::test]
    async fn test_should_resolve_data_root_after_directory_is_created() {
        let (project, config) = seeded_basic_project().await;
        let (engine, factory) = engine_with_factory(config, &project).await;
        let future = basic_options(&project, "future-output");

        let unresolved_key = engine.key(&future, false, false).await;
        let expected_unresolved = project.path().join("future-output");
        assert!(expected_unresolved.is_absolute());
        assert!(matches!(
            &unresolved_key.data_root,
            QueryDataRootKey::Unresolved(path) if path == &expected_unresolved
        ));
        assert!(!unresolved_key.is_cacheable());
        let error = engine
            .query(future.clone())
            .await
            .expect_err("future data root does not exist yet");
        assert_eq!(
            missing_table_fields(&error),
            (
                SearchMethod::Basic,
                "open Query table directory",
                "text_units"
            )
        );
        assert!(engine.basic.is_empty());

        write_basic_text_units(&project.path().join("future-output")).await;
        let second = engine
            .basic_runtime(&future)
            .await
            .expect("new data root becomes canonically valid");
        let third = engine
            .basic_runtime(&future)
            .await
            .expect("canonical runtime is reused");
        assert!(Arc::ptr_eq(&second.basic_context, &third.basic_context));
        assert_eq!(factory.completions.load(Ordering::SeqCst), 1);
        assert_eq!(factory.embeddings.load(Ordering::SeqCst), 1);
        assert_eq!(engine.basic.len(), 1);
    }

    #[tokio::test]
    async fn test_should_not_grow_cache_for_many_unresolved_data_roots() {
        let (project, config) = seeded_basic_project().await;
        let (engine, factory) = engine_with_factory(config, &project).await;
        engine
            .query(basic_options(&project, "output"))
            .await
            .expect("warm valid output snapshot");
        let callbacks = Arc::new(CountingCallbacks::default());

        for index in 0..32 {
            let mut invalid = basic_options(&project, &format!("missing-{index}/output"));
            invalid.callbacks.push(callbacks.clone());
            let error = engine
                .query(invalid)
                .await
                .expect_err("unresolved data root must fail");
            assert_eq!(
                missing_table_fields(&error),
                (
                    SearchMethod::Basic,
                    "open Query table directory",
                    "text_units"
                )
            );
        }

        assert_eq!(callbacks.calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.completions.load(Ordering::SeqCst), 1);
        assert_eq!(factory.embeddings.load(Ordering::SeqCst), 1);
        assert_eq!(engine.basic.len(), 1);
        engine
            .basic_runtime(&basic_options(&project, "output"))
            .await
            .expect("valid snapshot remains reusable");
        assert_eq!(factory.completions.load(Ordering::SeqCst), 1);
        assert_eq!(factory.embeddings.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_should_not_cache_repeated_unresolved_data_root_failure() {
        let (project, config) = seeded_basic_project().await;
        let (engine, factory) = engine_with_factory(config, &project).await;
        let invalid = basic_options(&project, "repeated-missing/output");

        let first_error = engine
            .query(invalid.clone())
            .await
            .expect_err("first unresolved request");
        let second_error = engine
            .query(invalid)
            .await
            .expect_err("second unresolved request");

        assert_eq!(
            missing_table_fields(&first_error),
            missing_table_fields(&second_error)
        );
        assert_eq!(
            missing_table_fields(&first_error),
            (
                SearchMethod::Basic,
                "open Query table directory",
                "text_units"
            )
        );
        assert!(engine.basic.is_empty());
        assert_eq!(factory.completions.load(Ordering::SeqCst), 0);
        assert_eq!(factory.embeddings.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_should_not_cache_successful_runtime_built_from_precomputed_unresolved_key() {
        let (project, config) = seeded_basic_project().await;
        let (engine, factory) = engine_with_factory(config, &project).await;
        let options = basic_options(&project, "appears-output");
        let unresolved_key = engine.key(&options, false, false).await;
        assert!(!unresolved_key.is_cacheable());

        write_basic_text_units(&project.path().join("appears-output")).await;
        let ephemeral = engine
            .basic_runtime_for_key(&options, unresolved_key)
            .await
            .expect("unresolved-key runtime can load after path appears");
        assert!(engine.basic.is_empty());
        assert_eq!(factory.completions.load(Ordering::SeqCst), 1);
        assert_eq!(factory.embeddings.load(Ordering::SeqCst), 1);

        let canonical = engine
            .basic_runtime(&options)
            .await
            .expect("next request establishes canonical snapshot");
        assert!(!Arc::ptr_eq(
            &ephemeral.basic_context,
            &canonical.basic_context
        ));
        assert_eq!(factory.completions.load(Ordering::SeqCst), 2);
        assert_eq!(factory.embeddings.load(Ordering::SeqCst), 2);
        assert_eq!(engine.basic.len(), 1);

        let third = engine
            .basic_runtime(&options)
            .await
            .expect("canonical runtime is reused");
        assert!(Arc::ptr_eq(&canonical.basic_context, &third.basic_context));
        assert_eq!(factory.completions.load(Ordering::SeqCst), 2);
        assert_eq!(factory.embeddings.load(Ordering::SeqCst), 2);
        assert_eq!(engine.basic.len(), 1);
    }

    #[tokio::test]
    async fn test_should_reject_mismatched_or_missing_root_before_preparing_resources() {
        let project = CanonicalTempDir::new();
        let other_project = CanonicalTempDir::new();
        let factory = Arc::new(CountingModelFactory::default());
        let factory_trait: Arc<dyn ModelFactory> = factory.clone();
        let engine = QueryEngine::load_with_factory(
            GraphRagConfig::default(),
            project.path(),
            factory_trait,
        )
        .await
        .expect("Query engine");

        let callbacks = Arc::new(CountingCallbacks::default());
        let mut options = QueryOptions::new(
            other_project.path().to_path_buf(),
            "question".to_owned(),
            SearchMethod::Basic,
        );
        options.callbacks.push(callbacks.clone());
        let error = engine
            .query(options.clone())
            .await
            .expect_err("cross-project query");
        assert!(matches!(
            error,
            crate::GraphLoomError::Query(source)
                if matches!(
                    *source,
                    crate::query::QueryError::InvalidQueryConfig {
                        method: SearchMethod::Basic,
                        operation: "validate QueryEngine project root",
                        ..
                    }
                )
        ));
        assert!(
            engine
                .query_stream(options)
                .await
                .is_err_and(|error| matches!(error, crate::GraphLoomError::Query(_)))
        );

        let missing = QueryOptions::new(
            project.path().join("missing"),
            "question".to_owned(),
            SearchMethod::Basic,
        );
        assert!(
            engine
                .query(missing)
                .await
                .is_err_and(|error| matches!(error, crate::GraphLoomError::Query(_)))
        );
        assert_eq!(factory.completions.load(Ordering::SeqCst), 0);
        assert_eq!(factory.embeddings.load(Ordering::SeqCst), 0);
        assert_eq!(callbacks.calls.load(Ordering::SeqCst), 0);
        assert!(engine.basic.is_empty());
        assert!(engine.local.is_empty());
        assert!(engine.global.is_empty());
        assert!(engine.drift.is_empty());
    }
}
