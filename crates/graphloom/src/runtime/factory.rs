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
        IndexVectorStoreFactory, ModelFactory, PreparedIndexServices,
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
    ) -> Result<PreparedIndexServices>;

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
    ) -> Result<PreparedIndexServices> {
        create_default_services(
            project,
            project_root,
            cache_enabled,
            requirements,
            &DefaultModelFactory,
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
) -> Result<PreparedIndexServices> {
    let output_table_provider: Arc<dyn TableProvider> =
        Arc::new(ParquetTableProvider::new(&project.paths.output_dir).map_err(runtime_build)?);
    let input_reader: Arc<dyn InputReader> = Arc::new(
        FileInputReader::with_file_pattern(
            &project.paths.input_dir,
            &project.config.input.file_pattern,
        )
        .map_err(runtime_build)?,
    );
    let output_storage: Arc<dyn Storage> =
        Arc::new(FileStorage::new(&project.paths.output_dir).map_err(runtime_build)?);
    let cache = if cache_enabled && project.config.cache.cache_type.eq_ignore_ascii_case("json") {
        let storage: Arc<dyn Storage> =
            Arc::new(FileStorage::new(&project.paths.cache_dir).map_err(runtime_build)?);
        CacheService::Enabled(Arc::new(JsonCache::new(storage)) as Arc<dyn Cache>)
    } else {
        CacheService::Disabled
    };
    let models = create_model_registry(&project.config, requirements, model_factory)?;

    Ok(PreparedIndexServices {
        io: IndexRuntimeIo::new(input_reader, output_storage, output_table_provider),
        cache,
        models,
        project_root: project_root.to_path_buf(),
    })
}

fn runtime_build(source: impl std::error::Error + Send + Sync + 'static) -> GraphLoomError {
    GraphLoomError::RuntimeBuild {
        source: Box::new(source),
    }
}
