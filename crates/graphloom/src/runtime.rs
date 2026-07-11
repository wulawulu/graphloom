//! Runtime assembly for standard indexing.

mod factory;
mod generation;
mod model_factory;
mod model_registry;
mod services;

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) use factory::{DefaultIndexRuntimeFactory, IndexRuntimeFactory};
pub(crate) use generation::StagedIndexGeneration;
use graphloom_storage::{FileStorage, Storage};
use graphloom_vectors::{LanceDbVectorStore, VectorStore};
pub use model_factory::{DefaultModelFactory, ModelFactory};
pub use model_registry::ModelRegistry;
pub use services::{CacheService, IndexRuntimeIo, IndexRuntimeServices};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::{
    ALL_EMBEDDINGS, GraphLoomError, GraphRagConfig, Pipeline, PipelineFactory, PipelineRunContext,
    Result, WorkflowCallbacks, WorkflowRegistry,
    path_safety::{path_is_within_or_equal, resolve_path_rejecting_links},
    project::{LoadedProject, ProjectPaths},
    register_standard_workflows,
};

/// Runtime ready to execute standard indexing.
#[derive(Debug)]
pub struct IndexRuntime {
    /// Resolved config.
    pub config: GraphRagConfig,
    /// Pipeline context.
    pub context: PipelineRunContext,
    /// Built pipeline.
    pub pipeline: Pipeline,
}

/// Providers and pipeline that passed non-destructive runtime preflight.
#[derive(Debug)]
pub(crate) struct PreparedIndexRuntime {
    services: Option<IndexRuntimeServices>,
    callbacks: Arc<dyn WorkflowCallbacks>,
    pipeline: Pipeline,
}

/// Build standard-index providers and pipeline without clearing output or resetting vectors.
///
/// # Errors
///
/// Returns an error when provider construction, vector config, or pipeline build fails.
pub(crate) async fn preflight_index_runtime(
    project: &LoadedProject,
    cache_enabled: bool,
    callbacks: Vec<Arc<dyn WorkflowCallbacks>>,
) -> Result<PreparedIndexRuntime> {
    preflight_index_runtime_with_factory(
        project,
        &project.root,
        cache_enabled,
        callbacks,
        &DefaultIndexRuntimeFactory,
    )
    .await
}

pub(crate) async fn preflight_index_runtime_with_factory(
    project: &LoadedProject,
    project_root: &Path,
    cache_enabled: bool,
    callbacks: Vec<Arc<dyn WorkflowCallbacks>>,
    factory: &dyn IndexRuntimeFactory,
) -> Result<PreparedIndexRuntime> {
    validate_managed_vector_schemas(&project.config)?;
    project.paths.validate_vector_path_safety()?;
    preflight_writable_paths(&project.paths, cache_enabled).await?;
    let services = factory
        .create_services(project, project_root, cache_enabled)
        .await?;
    let callbacks = crate::callbacks::callback_chain(callbacks);

    let mut registry = WorkflowRegistry::new();
    register_standard_workflows(&mut registry);
    let pipeline = PipelineFactory::new(registry)
        .standard(&project.config)
        .map_err(|source| GraphLoomError::RuntimeBuild {
            source: Box::new(source),
        })?;

    Ok(PreparedIndexRuntime {
        services: Some(services),
        callbacks,
        pipeline,
    })
}

/// Clear output storage and reset managed vector indices.
///
/// # Errors
///
/// Returns an error when cleanup fails.
pub(crate) async fn prepare_full_index(
    project: &LoadedProject,
    runtime: &mut PreparedIndexRuntime,
) -> Result<()> {
    project.paths.validate_destructive_paths()?;
    project.paths.validate_vector_path_safety()?;
    match vector_location(&project.paths)? {
        VectorLocation::InsideOutput => {
            let services = runtime.take_services()?;
            let IndexRuntimeServices {
                input_reader,
                input_storage,
                output_storage,
                output_table_provider,
                cache,
                vector_store,
                models,
                project_root,
            } = services;
            drop(vector_store);
            clear_output_dir(&project.paths.output_dir).await?;
            let new_store = connect_vector_store(&project.config).await?;
            reset_managed_indices(new_store.as_ref(), &project.config).await?;
            runtime.services = Some(IndexRuntimeServices::new(
                IndexRuntimeIo::new(
                    input_reader,
                    input_storage,
                    output_storage,
                    output_table_provider,
                ),
                cache,
                new_store,
                models,
                project_root,
            ));
            Ok(())
        }
        VectorLocation::OutsideOutput => {
            let store = &runtime.services()?.vector_store;
            reset_managed_indices(store.as_ref(), &project.config).await?;
            clear_output_dir(&project.paths.output_dir).await
        }
    }
}

impl PreparedIndexRuntime {
    pub(crate) fn into_runtime(
        self,
        config: GraphRagConfig,
        project_root: &Path,
    ) -> Result<IndexRuntime> {
        let mut services = self.services.ok_or_else(missing_services)?;
        services.project_root = project_root.to_path_buf();
        let context = PipelineRunContext::new(services, self.callbacks);

        Ok(IndexRuntime {
            config,
            context,
            pipeline: self.pipeline,
        })
    }

    fn services(&self) -> Result<&IndexRuntimeServices> {
        self.services.as_ref().ok_or_else(missing_services)
    }

    fn take_services(&mut self) -> Result<IndexRuntimeServices> {
        self.services.take().ok_or_else(missing_services)
    }
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

fn validate_managed_vector_schemas(config: &GraphRagConfig) -> Result<()> {
    config
        .vector_store
        .validate()
        .map_err(|source| GraphLoomError::RuntimeBuild {
            source: Box::new(source),
        })?;
    for embedding_name in ALL_EMBEDDINGS {
        config
            .vector_store
            .schema_for(embedding_name)
            .validate()
            .map_err(|source| GraphLoomError::RuntimeBuild {
                source: Box::new(source),
            })?;
    }
    Ok(())
}

async fn connect_vector_store(config: &GraphRagConfig) -> Result<Arc<dyn VectorStore>> {
    Ok(Arc::new(
        LanceDbVectorStore::connect(&config.vector_store)
            .await
            .map_err(|source| GraphLoomError::RuntimeBuild {
                source: Box::new(source),
            })?,
    ))
}

async fn preflight_writable_paths(paths: &ProjectPaths, cache_enabled: bool) -> Result<()> {
    probe_directory_writable(&paths.output_dir, "output").await?;
    probe_directory_writable(&paths.reporting_dir, "logs").await?;
    if cache_enabled {
        probe_directory_writable(&paths.cache_dir, "cache").await?;
    }
    probe_directory_writable(&paths.vector_db_uri, "vector DB").await
}

async fn probe_directory_writable(directory: &Path, label: &'static str) -> Result<()> {
    let probe_root = writable_probe_root(directory, label).await?;
    let probe = probe_root.join(format!(".graphloom-write-probe-{}", Uuid::new_v4()));
    let write_result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .await
            .map_err(|source| io_error("create write probe", &probe, source))?;
        file.write_all(b"graphloom")
            .await
            .map_err(|source| io_error("write probe", &probe, source))?;
        file.flush()
            .await
            .map_err(|source| io_error("flush write probe", &probe, source))?;
        drop(file);
        tokio::fs::remove_file(&probe)
            .await
            .map_err(|source| io_error("remove write probe", &probe, source))?;
        Ok(())
    }
    .await;

    if write_result.is_err() {
        let _ = tokio::fs::remove_file(&probe).await;
    }
    write_result
}

async fn writable_probe_root(directory: &Path, label: &'static str) -> Result<PathBuf> {
    match tokio::fs::metadata(directory).await {
        Ok(metadata) if metadata.is_dir() => Ok(directory.to_path_buf()),
        Ok(_) => Err(GraphLoomError::RuntimeBuild {
            source: Box::new(std::io::Error::new(
                ErrorKind::AlreadyExists,
                format!("{label} path {} is not a directory", directory.display()),
            )),
        }),
        Err(source) if source.kind() == ErrorKind::NotFound => {
            existing_ancestor(directory, label).await
        }
        Err(source) => Err(io_error("inspect writable directory", directory, source)),
    }
}

async fn existing_ancestor(path: &Path, label: &'static str) -> Result<PathBuf> {
    let mut current = path.to_path_buf();
    while let Some(parent) = current.parent() {
        match tokio::fs::metadata(parent).await {
            Ok(metadata) if metadata.is_dir() => return Ok(parent.to_path_buf()),
            Ok(_) => {
                return Err(GraphLoomError::RuntimeBuild {
                    source: Box::new(std::io::Error::new(
                        ErrorKind::AlreadyExists,
                        format!("{label} ancestor {} is not a directory", parent.display()),
                    )),
                });
            }
            Err(source) if source.kind() == ErrorKind::NotFound => {
                current = parent.to_path_buf();
            }
            Err(source) => return Err(io_error("inspect writable ancestor", parent, source)),
        }
    }
    Err(GraphLoomError::RuntimeBuild {
        source: Box::new(std::io::Error::new(
            ErrorKind::NotFound,
            format!(
                "no writable ancestor found for {label} path {}",
                path.display()
            ),
        )),
    })
}

fn missing_services() -> GraphLoomError {
    GraphLoomError::RuntimeBuild {
        source: Box::new(std::io::Error::other(
            "preflight services are missing from prepared runtime",
        )),
    }
}

fn io_error(operation: &'static str, path: &Path, source: std::io::Error) -> GraphLoomError {
    GraphLoomError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

async fn clear_output_dir(path: &Path) -> Result<()> {
    let storage = FileStorage::new(path).map_err(|source| GraphLoomError::RuntimeBuild {
        source: Box::new(source),
    })?;
    storage
        .clear()
        .await
        .map_err(|source| GraphLoomError::RuntimeBuild {
            source: Box::new(source),
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VectorLocation {
    InsideOutput,
    OutsideOutput,
}

fn vector_location(paths: &ProjectPaths) -> Result<VectorLocation> {
    let output = resolve_path_rejecting_links(&paths.output_dir)?;
    let vector = resolve_path_rejecting_links(&paths.vector_db_uri)?;
    Ok(
        if path_is_within_or_equal(&vector.resolved, &output.resolved)? {
            VectorLocation::InsideOutput
        } else {
            VectorLocation::OutsideOutput
        },
    )
}

#[cfg(test)]
mod runtime_factory_tests {
    use std::{
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use async_trait::async_trait;
    use futures_util::{Stream, stream};
    use graphloom_input::{DocumentStream, InputReader};
    use graphloom_storage::{MemoryStorage, MemoryTableProvider};
    use graphloom_vectors::{
        Result as VectorResult, VectorDocument, VectorIndexSchema, VectorStore,
    };
    use tempfile::TempDir;

    use super::{IndexRuntimeFactory, preflight_index_runtime_with_factory};
    use crate::{
        CacheService, GraphRagConfig, IndexRuntimeIo, IndexRuntimeServices, ModelRegistry, Result,
        project::LoadedProject,
    };

    #[derive(Debug, Default)]
    struct EmptyInputReader;

    impl InputReader for EmptyInputReader {
        fn read_documents(&self) -> DocumentStream<'_> {
            Box::pin(stream::empty()) as Pin<Box<dyn Stream<Item = _> + Send + '_>>
        }
    }

    #[derive(Debug, Default)]
    struct EmptyVectorStore;

    #[async_trait]
    impl VectorStore for EmptyVectorStore {
        async fn ensure_index(&self, _schema: &VectorIndexSchema) -> VectorResult<()> {
            Ok(())
        }
        async fn reset_index(&self, _schema: &VectorIndexSchema) -> VectorResult<()> {
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
    }

    #[derive(Debug)]
    struct MemoryRuntimeFactory {
        calls: AtomicUsize,
        table_provider: Arc<MemoryTableProvider>,
    }

    #[async_trait]
    impl IndexRuntimeFactory for MemoryRuntimeFactory {
        async fn create_services(
            &self,
            _project: &LoadedProject,
            project_root: &std::path::Path,
            _cache_enabled: bool,
        ) -> Result<IndexRuntimeServices> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let storage = Arc::new(MemoryStorage::new());
            Ok(IndexRuntimeServices::new(
                IndexRuntimeIo::new(
                    Arc::new(EmptyInputReader),
                    storage.clone(),
                    storage,
                    self.table_provider.clone(),
                ),
                CacheService::Disabled,
                Arc::new(EmptyVectorStore),
                ModelRegistry::default(),
                project_root,
            ))
        }
    }

    #[tokio::test]
    async fn test_should_prepare_runtime_entirely_from_injected_factory() {
        let tempdir = TempDir::new().expect("tempdir");
        let project = LoadedProject::from_config(tempdir.path(), GraphRagConfig::default())
            .expect("project should load");
        let factory = MemoryRuntimeFactory {
            calls: AtomicUsize::new(0),
            table_provider: Arc::new(MemoryTableProvider::new()),
        };

        let prepared = preflight_index_runtime_with_factory(
            &project,
            tempdir.path(),
            false,
            Vec::new(),
            &factory,
        )
        .await
        .expect("runtime should prepare");
        let runtime = prepared
            .into_runtime(project.config.clone(), tempdir.path())
            .expect("prepared runtime should convert");

        assert_eq!(factory.calls.load(Ordering::SeqCst), 1);
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
}

#[cfg(all(test, windows))]
mod tests {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::{VectorLocation, vector_location};
    use crate::project::ProjectPaths;

    #[test]
    fn test_should_detect_vector_inside_output_case_insensitively() {
        let tempdir = TempDir::new().expect("tempdir");
        let paths = project_paths(
            tempdir.path().join("Output"),
            tempdir.path().join("output").join("lancedb"),
        );

        assert_eq!(
            vector_location(&paths).expect("vector location"),
            VectorLocation::InsideOutput,
        );
    }

    #[test]
    fn test_should_detect_vector_outside_output_case_insensitively() {
        let tempdir = TempDir::new().expect("tempdir");
        let paths = project_paths(tempdir.path().join("Output"), tempdir.path().join("Vector"));

        assert_eq!(
            vector_location(&paths).expect("vector location"),
            VectorLocation::OutsideOutput,
        );
    }

    fn project_paths(output_dir: PathBuf, vector_db_uri: PathBuf) -> ProjectPaths {
        let root = output_dir.parent().expect("project root").to_path_buf();
        ProjectPaths {
            input_dir: root.join("Input"),
            cache_dir: root.join("Cache"),
            reporting_dir: root.join("Logs"),
            root,
            output_dir,
            vector_db_uri,
        }
    }
}
