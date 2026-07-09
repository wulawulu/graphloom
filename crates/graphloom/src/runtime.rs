//! Runtime assembly for standard indexing.

use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use graphloom_cache::JsonCache;
use graphloom_input::{FileInputReader, InputReader};
use graphloom_storage::{FileStorage, ParquetTableProvider, Storage, TableProvider};
use graphloom_vectors::{LanceDbVectorStore, VectorStore};

use crate::{
    ALL_EMBEDDINGS, GraphLoomError, GraphRagConfig, Pipeline, PipelineFactory, PipelineRunContext,
    Result, WorkflowCallbacks, WorkflowRegistry, project::LoadedProject, register_step9_workflows,
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
    vector_store: Arc<dyn VectorStore>,
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
        vector_store,
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
    match vector_location(&project.paths.output_dir, &project.paths.vector_db_uri) {
        VectorLocation::InsideOutput => {
            clear_output_dir(&project.paths.output_dir).await?;
            runtime.vector_store = connect_vector_store(&project.config).await?;
            reset_managed_indices(runtime.vector_store.as_ref(), &project.config).await
        }
        VectorLocation::OutsideOutput => {
            reset_managed_indices(runtime.vector_store.as_ref(), &project.config).await?;
            clear_output_dir(&project.paths.output_dir).await
        }
    }
}

impl PreparedIndexRuntime {
    pub(crate) fn into_runtime(self, config: GraphRagConfig, project_root: &Path) -> IndexRuntime {
        let mut context = PipelineRunContext::new(self.output_provider)
            .with_input_reader(self.input_reader)
            .with_vector_store(self.vector_store)
            .with_callbacks(self.callbacks)
            .with_project_root(project_root);
        context.input_storage = Some(self.input_storage);
        context.output_storage = Some(self.output_storage);
        context.cache = self.cache;

        IndexRuntime {
            config,
            context,
            pipeline: self.pipeline,
        }
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

fn vector_location(output_dir: &Path, vector_db_uri: &Path) -> VectorLocation {
    if normalize_lexical(vector_db_uri).starts_with(normalize_lexical(output_dir)) {
        VectorLocation::InsideOutput
    } else {
        VectorLocation::OutsideOutput
    }
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}
