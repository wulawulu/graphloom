//! Fully prepared services consumed by indexing workflows.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use graphloom_cache::Cache;
use graphloom_input::InputReader;
use graphloom_storage::{Storage, TableProvider};
use graphloom_vectors::VectorStore;

use crate::runtime::ModelRegistry;

/// Input and output providers used by the standard indexing pipeline.
#[derive(Debug, Clone)]
pub struct IndexRuntimeIo {
    pub(crate) input_reader: Arc<dyn InputReader>,
    pub(crate) output_storage: Arc<dyn Storage>,
    pub(crate) output_table_provider: Arc<dyn TableProvider>,
}

impl IndexRuntimeIo {
    /// Create the complete indexing input/output provider set.
    #[must_use]
    pub fn new(
        input_reader: Arc<dyn InputReader>,
        output_storage: Arc<dyn Storage>,
        output_table_provider: Arc<dyn TableProvider>,
    ) -> Self {
        Self {
            input_reader,
            output_storage,
            output_table_provider,
        }
    }
}

/// Explicit cache availability for an indexing run.
#[derive(Debug, Clone)]
pub enum CacheService {
    /// Cache operations use the prepared provider.
    Enabled(Arc<dyn Cache>),
    /// Cache operations are disabled for this run.
    Disabled,
}

impl CacheService {
    pub(crate) fn provider(&self) -> Option<&Arc<dyn Cache>> {
        match self {
            Self::Enabled(cache) => Some(cache),
            Self::Disabled => None,
        }
    }
}

/// Complete set of services required by the standard indexing pipeline.
#[derive(Debug, Clone)]
pub struct IndexRuntimeServices {
    pub(crate) input_reader: Arc<dyn InputReader>,
    pub(crate) output_storage: Arc<dyn Storage>,
    pub(crate) output_table_provider: Arc<dyn TableProvider>,
    pub(crate) cache: CacheService,
    pub(crate) vector_store: VectorStoreService,
    pub(crate) models: ModelRegistry,
    pub(crate) project_root: PathBuf,
}

impl IndexRuntimeServices {
    /// Create a complete standard-index service set.
    #[must_use]
    pub fn new(
        io: IndexRuntimeIo,
        cache: CacheService,
        vector_store: VectorStoreService,
        models: ModelRegistry,
        project_root: impl Into<PathBuf>,
    ) -> Self {
        let IndexRuntimeIo {
            input_reader,
            output_storage,
            output_table_provider,
        } = io;
        Self {
            input_reader,
            output_storage,
            output_table_provider,
            cache,
            vector_store,
            models,
            project_root: project_root.into(),
        }
    }

    /// Return the project root used for prompt resolution.
    #[must_use]
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }
}

/// Explicit vector storage availability for the active indexing pipeline.
#[derive(Debug, Clone)]
pub(crate) enum VectorStoreService {
    Enabled(Arc<dyn VectorStore>),
    Disabled,
}

impl VectorStoreService {
    pub(crate) fn provider(&self) -> crate::Result<Arc<dyn VectorStore>> {
        match self {
            Self::Enabled(store) => Ok(Arc::clone(store)),
            Self::Disabled => Err(crate::GraphLoomError::MissingRuntimeCapability {
                capability: "vector_store",
            }),
        }
    }
}

/// Services prepared before optional vector storage is attached.
#[derive(Debug)]
pub(crate) struct PreparedIndexServices {
    pub(crate) io: IndexRuntimeIo,
    pub(crate) cache: CacheService,
    pub(crate) models: ModelRegistry,
    pub(crate) project_root: PathBuf,
}

impl PreparedIndexServices {
    pub(crate) fn into_runtime_services(
        self,
        vector_store: VectorStoreService,
    ) -> IndexRuntimeServices {
        IndexRuntimeServices::new(
            self.io,
            self.cache,
            vector_store,
            self.models,
            self.project_root,
        )
    }
}
