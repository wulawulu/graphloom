//! Runtime assembly for standard indexing.

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
};

use graphloom_cache::JsonCache;
use graphloom_input::{FileInputReader, InputReader};
use graphloom_storage::{FileStorage, ParquetTableProvider, Storage, TableProvider};
use graphloom_vectors::{LanceDbVectorStore, VectorStore};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::{
    ALL_EMBEDDINGS, GraphLoomError, GraphRagConfig, Pipeline, PipelineFactory, PipelineRunContext,
    Result, WorkflowCallbacks, WorkflowRegistry,
    project::{LoadedProject, ProjectPaths, resolve_path_rejecting_links},
    register_step9_workflows,
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
    input_reader: Arc<dyn InputReader>,
    input_storage: Arc<dyn Storage>,
    output_provider: Arc<dyn TableProvider>,
    output_storage: Arc<dyn Storage>,
    cache: Option<Arc<dyn graphloom_cache::Cache>>,
    vector_store: Option<Arc<dyn VectorStore>>,
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
    validate_managed_vector_schemas(&project.config)?;
    project.paths.validate_vector_path_safety()?;
    preflight_writable_paths(&project.paths, cache_enabled).await?;
    let output_provider = Arc::new(
        ParquetTableProvider::new(&project.paths.output_dir).map_err(|source| {
            GraphLoomError::RuntimeBuild {
                source: Box::new(source),
            }
        })?,
    );
    let input_reader = Arc::new(
        FileInputReader::with_file_pattern(
            &project.paths.input_dir,
            &project.config.input.file_pattern,
        )
        .map_err(|source| GraphLoomError::RuntimeBuild {
            source: Box::new(source),
        })?,
    );
    let input_storage = Arc::new(
        FileStorage::new(&project.paths.input_dir).map_err(|source| {
            GraphLoomError::RuntimeBuild {
                source: Box::new(source),
            }
        })?,
    );
    let output_storage = Arc::new(FileStorage::new(&project.paths.output_dir).map_err(
        |source| GraphLoomError::RuntimeBuild {
            source: Box::new(source),
        },
    )?);
    let cache =
        if cache_enabled && project.config.cache.cache_type.eq_ignore_ascii_case("json") {
            let storage = Arc::new(FileStorage::new(&project.paths.cache_dir).map_err(
                |source| GraphLoomError::RuntimeBuild {
                    source: Box::new(source),
                },
            )?);
            Some(Arc::new(JsonCache::new(storage)) as Arc<dyn graphloom_cache::Cache>)
        } else {
            None
        };
    let vector_store = Arc::new(
        LanceDbVectorStore::connect(&project.config.vector_store)
            .await
            .map_err(|source| GraphLoomError::RuntimeBuild {
                source: Box::new(source),
            })?,
    );
    let callbacks = crate::callbacks::callback_chain(callbacks);

    let mut registry = WorkflowRegistry::new();
    register_step9_workflows(&mut registry);
    let pipeline = PipelineFactory::new(registry)
        .standard(&project.config)
        .map_err(|source| GraphLoomError::RuntimeBuild {
            source: Box::new(source),
        })?;

    Ok(PreparedIndexRuntime {
        input_reader,
        input_storage,
        output_provider,
        output_storage,
        cache,
        vector_store: Some(vector_store),
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
            let old_store = runtime.take_vector_store()?;
            drop(old_store);
            clear_output_dir(&project.paths.output_dir).await?;
            let new_store = connect_vector_store(&project.config).await?;
            reset_managed_indices(new_store.as_ref(), &project.config).await?;
            runtime.vector_store = Some(new_store);
            Ok(())
        }
        VectorLocation::OutsideOutput => {
            let store = runtime.vector_store()?;
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
        let vector_store = self.vector_store.ok_or_else(missing_vector_store)?;
        let mut context = PipelineRunContext::new(self.output_provider)
            .with_input_reader(self.input_reader)
            .with_vector_store(vector_store)
            .with_callbacks(self.callbacks)
            .with_project_root(project_root);
        context.input_storage = Some(self.input_storage);
        context.output_storage = Some(self.output_storage);
        context.cache = self.cache;

        Ok(IndexRuntime {
            config,
            context,
            pipeline: self.pipeline,
        })
    }

    fn vector_store(&self) -> Result<&Arc<dyn VectorStore>> {
        self.vector_store.as_ref().ok_or_else(missing_vector_store)
    }

    fn take_vector_store(&mut self) -> Result<Arc<dyn VectorStore>> {
        self.vector_store.take().ok_or_else(missing_vector_store)
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

fn missing_vector_store() -> GraphLoomError {
    GraphLoomError::RuntimeBuild {
        source: Box::new(std::io::Error::other(
            "preflight vector store is missing from prepared runtime",
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
    Ok(if vector.resolved.starts_with(&output.resolved) {
        VectorLocation::InsideOutput
    } else {
        VectorLocation::OutsideOutput
    })
}
