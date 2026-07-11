//! Replaceable indexing service assembly.

use std::sync::Arc;

use async_trait::async_trait;
use graphloom_cache::{Cache, JsonCache};
use graphloom_input::{FileInputReader, InputReader};
use graphloom_storage::{FileStorage, ParquetTableProvider, Storage, TableProvider};

use crate::{
    GraphLoomError, IndexWorkflowRequirements, Result,
    project::LoadedProject,
    runtime::{
        CacheService, DefaultIndexVectorStoreFactory, DefaultModelFactory, IndexRuntimeIo,
        IndexRuntimeServices, IndexVectorStoreFactory, ModelFactory,
        model_factory::create_model_registry,
    },
};

/// Factory seam for constructing all services required by standard indexing.
#[async_trait]
pub(crate) trait IndexRuntimeFactory: Send + Sync + std::fmt::Debug {
    async fn create_services(
        &self,
        project: &LoadedProject,
        project_root: &std::path::Path,
        cache_enabled: bool,
        requirements: &IndexWorkflowRequirements,
    ) -> Result<IndexRuntimeServices>;

    fn vector_store_factory(&self) -> Arc<dyn IndexVectorStoreFactory>;
}

/// Default factory preserving the existing file, Parquet, JSON, LanceDB, and OpenAI providers.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DefaultIndexRuntimeFactory;

#[async_trait]
impl IndexRuntimeFactory for DefaultIndexRuntimeFactory {
    async fn create_services(
        &self,
        project: &LoadedProject,
        project_root: &std::path::Path,
        cache_enabled: bool,
        requirements: &IndexWorkflowRequirements,
    ) -> Result<IndexRuntimeServices> {
        create_default_services(
            project,
            project_root,
            cache_enabled,
            requirements,
            &DefaultModelFactory,
            &DefaultIndexVectorStoreFactory,
        )
        .await
    }

    fn vector_store_factory(&self) -> Arc<dyn IndexVectorStoreFactory> {
        Arc::new(DefaultIndexVectorStoreFactory)
    }
}

async fn create_default_services(
    project: &LoadedProject,
    project_root: &std::path::Path,
    cache_enabled: bool,
    requirements: &IndexWorkflowRequirements,
    model_factory: &dyn ModelFactory,
    vector_store_factory: &dyn IndexVectorStoreFactory,
) -> Result<IndexRuntimeServices> {
    let output_table_provider: Arc<dyn TableProvider> =
        Arc::new(ParquetTableProvider::new(&project.paths.output_dir).map_err(runtime_build)?);
    let input_reader: Arc<dyn InputReader> = Arc::new(
        FileInputReader::with_file_pattern(
            &project.paths.input_dir,
            &project.config.input.file_pattern,
        )
        .map_err(runtime_build)?,
    );
    let input_storage: Arc<dyn Storage> =
        Arc::new(FileStorage::new(&project.paths.input_dir).map_err(runtime_build)?);
    let output_storage: Arc<dyn Storage> =
        Arc::new(FileStorage::new(&project.paths.output_dir).map_err(runtime_build)?);
    let cache = if cache_enabled && project.config.cache.cache_type.eq_ignore_ascii_case("json") {
        let storage: Arc<dyn Storage> =
            Arc::new(FileStorage::new(&project.paths.cache_dir).map_err(runtime_build)?);
        CacheService::Enabled(Arc::new(JsonCache::new(storage)) as Arc<dyn Cache>)
    } else {
        CacheService::Disabled
    };
    let vector_store = vector_store_factory
        .create(&project.config.vector_store)
        .await?;
    let models = create_model_registry(&project.config, requirements, model_factory)?;

    Ok(IndexRuntimeServices::new(
        IndexRuntimeIo::new(
            input_reader,
            input_storage,
            output_storage,
            output_table_provider,
        ),
        cache,
        vector_store,
        models,
        project_root,
    ))
}

fn runtime_build(source: impl std::error::Error + Send + Sync + 'static) -> GraphLoomError {
    GraphLoomError::RuntimeBuild {
        source: Box::new(source),
    }
}
