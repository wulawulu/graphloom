//! Runtime assembly for standard indexing.

use std::{path::Path, sync::Arc};

use graphloom_cache::JsonCache;
use graphloom_input::FileInputReader;
use graphloom_storage::{FileStorage, ParquetTableProvider, Storage};
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

/// Build a runtime for standard indexing.
///
/// # Errors
///
/// Returns an error when providers cannot be created.
pub async fn build_runtime(
    project: &LoadedProject,
    cache_enabled: bool,
    callbacks: Vec<Arc<dyn WorkflowCallbacks>>,
) -> Result<IndexRuntime> {
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

    let mut context = PipelineRunContext::new(output_provider)
        .with_input_reader(input_reader)
        .with_vector_store(vector_store)
        .with_callbacks(callbacks)
        .with_project_root(&project.root);
    context.input_storage = Some(input_storage);
    context.output_storage = Some(output_storage);
    context.cache = cache;

    let mut registry = WorkflowRegistry::new();
    register_step9_workflows(&mut registry);
    let pipeline = PipelineFactory::new(registry)
        .standard(&project.config)
        .map_err(|source| GraphLoomError::RuntimeBuild {
            source: Box::new(source),
        })?;

    Ok(IndexRuntime {
        config: project.config.clone(),
        context,
        pipeline,
    })
}

/// Clear output storage and reset managed vector indices.
///
/// # Errors
///
/// Returns an error when cleanup fails.
pub async fn prepare_full_index(project: &LoadedProject) -> Result<()> {
    project.paths.validate_destructive_paths()?;
    clear_output_dir(&project.paths.output_dir).await?;
    let store = LanceDbVectorStore::connect(&project.config.vector_store)
        .await
        .map_err(|source| GraphLoomError::RuntimeBuild {
            source: Box::new(source),
        })?;
    reset_managed_indices(&store, &project.config).await
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
