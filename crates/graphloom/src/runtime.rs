//! Runtime assembly for standard indexing.

mod factory;
mod model_factory;
mod model_registry;
mod services;
mod vector_store_factory;

use std::{path::Path, sync::Arc};

pub(crate) use factory::{DefaultIndexRuntimeFactory, IndexRuntimeFactory};
use graphloom_vectors::VectorStore;
pub(crate) use model_factory::validate_model_connectivity;
pub use model_factory::{DefaultModelFactory, ModelFactory};
pub use model_registry::ModelRegistry;
pub use services::{CacheService, IndexRuntimeIo, IndexRuntimeServices};
pub(crate) use services::{PreparedIndexServices, VectorStoreService};
pub(crate) use vector_store_factory::{DefaultIndexVectorStoreFactory, IndexVectorStoreFactory};

use crate::{
    ALL_EMBEDDINGS, GraphLoomError, GraphRagConfig, IndexPipeline, IndexPipelineContext,
    IndexPipelineFactory, IndexWorkflowCallbacks, IndexWorkflowRegistry, Result,
    project::LoadedProject, register_standard_index_workflows,
};

/// Runtime ready to execute standard indexing.
#[derive(Debug)]
pub struct IndexRuntime {
    /// Resolved config.
    pub config: GraphRagConfig,
    /// `IndexPipeline` context.
    pub context: IndexPipelineContext,
    /// Built pipeline.
    pub pipeline: IndexPipeline,
}

/// Providers and pipeline prepared for a validated index run.
#[derive(Debug)]
pub(crate) struct PreparedIndexRuntime {
    services: PreparedIndexServices,
    vector_store: Option<Arc<dyn VectorStore>>,
    callbacks: Arc<dyn IndexWorkflowCallbacks>,
    pipeline: IndexPipeline,
}

/// Build standard-index providers and pipeline without clearing output or resetting vectors.
///
/// # Errors
///
/// Returns an error when provider construction or pipeline build fails.
pub(crate) async fn prepare_index_runtime(
    project: &LoadedProject,
    cache_enabled: bool,
    callbacks: Vec<Arc<dyn IndexWorkflowCallbacks>>,
) -> Result<PreparedIndexRuntime> {
    prepare_index_runtime_with_factory(
        project,
        &project.root,
        cache_enabled,
        callbacks,
        &DefaultIndexRuntimeFactory,
    )
    .await
}

pub(crate) async fn prepare_index_runtime_with_factory(
    project: &LoadedProject,
    project_root: &Path,
    cache_enabled: bool,
    callbacks: Vec<Arc<dyn IndexWorkflowCallbacks>>,
    factory: &dyn IndexRuntimeFactory,
) -> Result<PreparedIndexRuntime> {
    let mut registry = IndexWorkflowRegistry::new();
    register_standard_index_workflows(&mut registry)?;
    let pipeline = IndexPipelineFactory::new(registry)
        .standard(&project.config)
        .map_err(|source| GraphLoomError::RuntimeBuild {
            source: Box::new(source),
        })?;
    let requirements = pipeline.requirements(&project.config)?;
    let requires_vector_store = requirements.requires_vector_store();
    let services = factory
        .create_services(project, project_root, cache_enabled, &requirements)
        .await?;
    let vector_store = if requires_vector_store {
        project.paths.validate_vector_path_safety()?;
        let store = factory
            .vector_store_factory()
            .create(&project.config.vector_store)
            .await?;
        reset_managed_indices(store.as_ref(), &project.config).await?;
        Some(store)
    } else {
        None
    };
    let callbacks = crate::callbacks::callback_chain(callbacks);
    callbacks.runtime_prepared();

    Ok(PreparedIndexRuntime {
        services,
        vector_store,
        callbacks,
        pipeline,
    })
}

async fn reset_managed_indices(store: &dyn VectorStore, config: &GraphRagConfig) -> Result<()> {
    for embedding_name in ALL_EMBEDDINGS {
        let schema = config.vector_store.schema_for(embedding_name);
        store
            .reset_index(&schema)
            .await
            .map_err(|source| GraphLoomError::RuntimeBuild {
                source: Box::new(source),
            })?;
    }
    Ok(())
}

impl PreparedIndexRuntime {
    pub(crate) fn into_runtime(self, config: GraphRagConfig, project_root: &Path) -> IndexRuntime {
        let mut services = self.services;
        services.project_root = project_root.to_path_buf();
        let vector_store = match self.vector_store {
            Some(store) => VectorStoreService::Enabled(store),
            None => VectorStoreService::Disabled,
        };
        let context =
            IndexPipelineContext::new(services.into_runtime_services(vector_store), self.callbacks);

        IndexRuntime {
            config,
            context,
            pipeline: self.pipeline,
        }
    }
}

#[cfg(test)]
mod runtime_factory_tests {
    use std::{
        pin::Pin,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use async_trait::async_trait;
    use futures_util::{Stream, stream};
    use graphloom_input::{DocumentStream, InputReader, TextDocument};
    use graphloom_storage::{
        MemoryStorage, MemoryTableProvider, Result as StorageResult, Storage, TableProvider,
    };
    use graphloom_vectors::{
        Result as VectorResult, VectorDocument, VectorIndexSchema, VectorSearchResult, VectorStore,
        VectorStoreConfig,
    };
    use tempfile::TempDir;

    use super::{IndexRuntimeFactory, IndexVectorStoreFactory, prepare_index_runtime_with_factory};
    use crate::{
        GraphLoomError, GraphRagConfig, IndexWorkflowRequirements, ModelRegistry, Result,
        project::LoadedProject,
        runtime::{CacheService, IndexRuntimeIo, PreparedIndexServices},
    };

    #[derive(Debug, Default)]
    struct EmptyInputReader;

    impl InputReader for EmptyInputReader {
        fn read_documents(&self) -> DocumentStream<'_> {
            Box::pin(stream::empty()) as Pin<Box<dyn Stream<Item = _> + Send + '_>>
        }
    }

    #[derive(Debug)]
    struct StaticInputReader {
        documents: Vec<TextDocument>,
    }

    impl InputReader for StaticInputReader {
        fn read_documents(&self) -> DocumentStream<'_> {
            Box::pin(stream::iter(self.documents.clone().into_iter().map(Ok)))
        }
    }

    #[derive(Debug, Default)]
    struct EmptyVectorStore {
        resets: Arc<AtomicUsize>,
        drops: Arc<AtomicUsize>,
    }

    impl Drop for EmptyVectorStore {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl VectorStore for EmptyVectorStore {
        async fn ensure_index(&self, _schema: &VectorIndexSchema) -> VectorResult<()> {
            Ok(())
        }
        async fn reset_index(&self, _schema: &VectorIndexSchema) -> VectorResult<()> {
            self.resets.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn upsert_documents(
            &self,
            _schema: &VectorIndexSchema,
            _documents: &[VectorDocument],
        ) -> VectorResult<()> {
            Ok(())
        }
        async fn count(&self, _schema: &VectorIndexSchema) -> VectorResult<usize> {
            Ok(0)
        }
        async fn ids(&self, _schema: &VectorIndexSchema) -> VectorResult<Vec<String>> {
            Ok(Vec::new())
        }
        async fn get_by_id(
            &self,
            _schema: &VectorIndexSchema,
            _id: &str,
        ) -> VectorResult<Option<VectorDocument>> {
            Ok(None)
        }
        async fn similarity_search_by_vector(
            &self,
            _schema: &VectorIndexSchema,
            _query_vector: &[f32],
            _k: usize,
            _include_vectors: bool,
        ) -> VectorResult<Vec<VectorSearchResult>> {
            Ok(Vec::new())
        }
    }

    #[derive(Debug)]
    struct MemoryRuntimeFactory {
        calls: AtomicUsize,
        table_provider: Arc<MemoryTableProvider>,
        vector_factory: Arc<MemoryVectorStoreFactory>,
        output_storage: Arc<CountingStorage>,
        input_reader: Arc<dyn InputReader>,
    }

    #[derive(Debug, Default)]
    struct CountingStorage {
        inner: MemoryStorage,
        clear_calls: AtomicUsize,
    }

    #[async_trait]
    impl Storage for CountingStorage {
        async fn get(&self, name: &str) -> StorageResult<Option<Vec<u8>>> {
            self.inner.get(name).await
        }
        async fn set(&self, name: &str, bytes: &[u8]) -> StorageResult<()> {
            self.inner.set(name, bytes).await
        }
        async fn delete(&self, name: &str) -> StorageResult<()> {
            self.inner.delete(name).await
        }
        async fn clear(&self) -> StorageResult<()> {
            self.clear_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.clear().await
        }
        async fn has(&self, name: &str) -> StorageResult<bool> {
            self.inner.has(name).await
        }
        async fn list(&self, prefix: &str) -> StorageResult<Vec<String>> {
            self.inner.list(prefix).await
        }
        async fn keys(&self) -> StorageResult<Vec<String>> {
            self.inner.keys().await
        }
        async fn find(&self, pattern: &str) -> StorageResult<Vec<String>> {
            self.inner.find(pattern).await
        }
        async fn get_creation_date(&self, name: &str) -> StorageResult<Option<String>> {
            self.inner.get_creation_date(name).await
        }
        fn child(&self, namespace: Option<&str>) -> StorageResult<Arc<dyn Storage>> {
            self.inner.child(namespace)
        }
    }

    #[derive(Debug, Default)]
    struct MemoryVectorStoreFactory {
        factory_id: usize,
        call_factory_ids: Mutex<Vec<usize>>,
        calls: AtomicUsize,
        resets: Arc<AtomicUsize>,
        drops: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl IndexVectorStoreFactory for MemoryVectorStoreFactory {
        async fn create(&self, _config: &VectorStoreConfig) -> Result<Arc<dyn VectorStore>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.call_factory_ids
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(self.factory_id);
            Ok(Arc::new(EmptyVectorStore {
                resets: self.resets.clone(),
                drops: self.drops.clone(),
            }))
        }
    }

    #[async_trait]
    impl IndexRuntimeFactory for MemoryRuntimeFactory {
        async fn create_services(
            &self,
            _project: &LoadedProject,
            project_root: &std::path::Path,
            _cache_enabled: bool,
            _requirements: &IndexWorkflowRequirements,
        ) -> Result<PreparedIndexServices> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(PreparedIndexServices {
                io: IndexRuntimeIo::new(
                    self.input_reader.clone(),
                    self.output_storage.clone(),
                    self.table_provider.clone(),
                ),
                cache: CacheService::Disabled,
                models: ModelRegistry::default(),
                project_root: project_root.to_path_buf(),
            })
        }

        fn vector_store_factory(&self) -> Arc<dyn IndexVectorStoreFactory> {
            self.vector_factory.clone()
        }
    }

    #[tokio::test]
    async fn test_should_prepare_runtime_entirely_from_injected_factory() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut config = GraphRagConfig {
            workflows: vec![crate::GENERATE_TEXT_EMBEDDINGS_WORKFLOW.to_owned()],
            ..Default::default()
        };
        config.embedding_models.insert(
            config.embed_text.embedding_model_id.clone(),
            serde_json::from_value(serde_json::json!({
                "model_provider": "openai",
                "model": "embedding-test",
                "api_key": "test-key"
            }))
            .expect("embedding model config"),
        );
        config.vector_store.db_uri = tempdir
            .path()
            .join("output")
            .join("lancedb")
            .to_string_lossy()
            .into_owned();
        let project =
            LoadedProject::from_config(tempdir.path(), config).expect("project should load");
        let factory = MemoryRuntimeFactory {
            calls: AtomicUsize::new(0),
            table_provider: Arc::new(MemoryTableProvider::new()),
            vector_factory: Arc::new(MemoryVectorStoreFactory {
                factory_id: 41,
                call_factory_ids: Mutex::new(Vec::new()),
                calls: AtomicUsize::new(0),
                resets: Arc::new(AtomicUsize::new(0)),
                drops: Arc::new(AtomicUsize::new(0)),
            }),
            output_storage: Arc::new(CountingStorage::default()),
            input_reader: Arc::new(EmptyInputReader),
        };

        let prepared = prepare_index_runtime_with_factory(
            &project,
            tempdir.path(),
            false,
            Vec::new(),
            &factory,
        )
        .await
        .expect("runtime should prepare");
        let runtime = prepared.into_runtime(project.config.clone(), tempdir.path());

        assert_eq!(factory.calls.load(Ordering::SeqCst), 1);
        assert_eq!(factory.vector_factory.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            *factory
                .vector_factory
                .call_factory_ids
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec![41]
        );
        assert_eq!(factory.output_storage.clear_calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.vector_factory.drops.load(Ordering::SeqCst), 0);
        assert_eq!(
            factory.vector_factory.resets.load(Ordering::SeqCst),
            crate::ALL_EMBEDDINGS.len()
        );
        assert_eq!(runtime.context.project_root(), tempdir.path());
        assert!(
            !runtime
                .context
                .output_table_provider()
                .has("documents")
                .await
                .expect("table lookup")
        );
    }

    #[tokio::test]
    async fn test_should_reuse_factory_store_when_vector_path_is_outside_output() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut config = GraphRagConfig {
            workflows: vec![crate::GENERATE_TEXT_EMBEDDINGS_WORKFLOW.to_owned()],
            ..Default::default()
        };
        config.embedding_models.insert(
            config.embed_text.embedding_model_id.clone(),
            serde_json::from_value(serde_json::json!({
                "model_provider": "openai",
                "model": "embedding-test",
                "api_key": "test-key"
            }))
            .expect("embedding model config"),
        );
        config.vector_store.db_uri = tempdir
            .path()
            .join("vectors")
            .to_string_lossy()
            .into_owned();
        let project =
            LoadedProject::from_config(tempdir.path(), config).expect("project should load");
        let vector_factory = Arc::new(MemoryVectorStoreFactory {
            factory_id: 42,
            call_factory_ids: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
            resets: Arc::new(AtomicUsize::new(0)),
            drops: Arc::new(AtomicUsize::new(0)),
        });
        let factory = MemoryRuntimeFactory {
            calls: AtomicUsize::new(0),
            table_provider: Arc::new(MemoryTableProvider::new()),
            vector_factory: vector_factory.clone(),
            output_storage: Arc::new(CountingStorage::default()),
            input_reader: Arc::new(EmptyInputReader),
        };
        let prepared = prepare_index_runtime_with_factory(
            &project,
            tempdir.path(),
            false,
            Vec::new(),
            &factory,
        )
        .await
        .expect("runtime should prepare");

        let _runtime = prepared.into_runtime(project.config.clone(), tempdir.path());

        assert_eq!(vector_factory.calls.load(Ordering::SeqCst), 1);
        assert_eq!(factory.output_storage.clear_calls.load(Ordering::SeqCst), 0);
        assert_eq!(vector_factory.drops.load(Ordering::SeqCst), 0);
        assert_eq!(
            vector_factory.resets.load(Ordering::SeqCst),
            crate::ALL_EMBEDDINGS.len()
        );
    }

    #[tokio::test]
    async fn test_should_run_chunk_only_without_vector_capability() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut config = serde_yaml::from_str::<GraphRagConfig>(
            "workflows:\n  - load_input_documents\n  - create_base_text_units\n",
        )
        .expect("chunk-only YAML should deserialize");
        config.vector_store.vector_size = 0;
        config.vector_store.db_uri = "/".to_owned();
        let project =
            LoadedProject::from_config(tempdir.path(), config).expect("project should load");
        let vector_factory = Arc::new(MemoryVectorStoreFactory {
            factory_id: 43,
            call_factory_ids: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
            resets: Arc::new(AtomicUsize::new(0)),
            drops: Arc::new(AtomicUsize::new(0)),
        });
        let output_storage = Arc::new(CountingStorage::default());
        output_storage
            .set("sentinel", b"old")
            .await
            .expect("sentinel should write");
        let table_provider = Arc::new(MemoryTableProvider::new());
        let factory = MemoryRuntimeFactory {
            calls: AtomicUsize::new(0),
            table_provider: table_provider.clone(),
            vector_factory: vector_factory.clone(),
            output_storage: output_storage.clone(),
            input_reader: Arc::new(StaticInputReader {
                documents: vec![TextDocument::new(
                    "doc-1".to_owned(),
                    "alpha beta gamma".to_owned(),
                    "doc.txt".to_owned(),
                    None,
                    None,
                )],
            }),
        };
        let prepared = prepare_index_runtime_with_factory(
            &project,
            tempdir.path(),
            false,
            Vec::new(),
            &factory,
        )
        .await
        .expect("chunk-only runtime should prepare");
        let mut runtime = prepared.into_runtime(project.config.clone(), tempdir.path());
        runtime
            .pipeline
            .run(&runtime.config, &mut runtime.context)
            .await
            .expect("chunk-only pipeline should run");

        assert_eq!(vector_factory.calls.load(Ordering::SeqCst), 0);
        assert_eq!(vector_factory.resets.load(Ordering::SeqCst), 0);
        assert_eq!(vector_factory.drops.load(Ordering::SeqCst), 0);
        assert_eq!(output_storage.clear_calls.load(Ordering::SeqCst), 0);
        assert!(
            output_storage
                .has("sentinel")
                .await
                .expect("sentinel lookup")
        );
        assert!(
            table_provider
                .has("documents")
                .await
                .expect("documents lookup")
        );
        assert!(
            table_provider
                .has("text_units")
                .await
                .expect("text units lookup")
        );
        assert!(matches!(
            runtime.context.vector_store(),
            Err(GraphLoomError::MissingRuntimeCapability {
                capability: "vector_store"
            })
        ));
    }
}
