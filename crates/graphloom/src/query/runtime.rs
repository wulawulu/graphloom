//! Read-only Query runtime assembly.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use graphloom_llm::{CompletionModel, EmbeddingModel, ModelConfig, TiktokenTokenizer, Tokenizer};
use graphloom_storage::{FileStorage, ParquetTableProvider, TableProvider};
use graphloom_vectors::{VectorError, VectorIndexSchema, VectorStore, create_vector_store};

use super::{
    QueryCallbackChain, QueryCallbacks, QueryError, QueryOptions, Result, SearchMethod, TextUnit,
    basic::BasicContextBuilder, data_loader::QueryDataLoader, requirements::QueryRequirements,
};
use crate::{
    project::LoadedProject,
    prompts::{PromptKind, PromptRepository, PromptTemplate},
    runtime::{DefaultModelFactory, ModelFactory},
};

/// Prepared resources for one Basic Search invocation.
#[derive(Debug)]
pub(crate) struct QueryRuntime {
    pub(crate) basic_context: BasicContextBuilder,
    pub(crate) completion_model: Arc<dyn CompletionModel>,
    pub(crate) completion_model_id: String,
    pub(crate) completion_config: ModelConfig,
    pub(crate) prompt: PromptTemplate,
    pub(crate) callbacks: Arc<dyn QueryCallbacks>,
}

struct BasicModelResources {
    completion: Arc<dyn CompletionModel>,
    completion_id: String,
    completion_config: ModelConfig,
    embedding: Arc<dyn EmbeddingModel>,
    embedding_id: String,
    tokenizer: Arc<dyn Tokenizer>,
}

struct BasicVectorResources {
    store: Arc<dyn VectorStore>,
    schema: VectorIndexSchema,
}

/// Factory that assembles only resources required by the active Query method.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct QueryRuntimeFactory;

impl QueryRuntimeFactory {
    pub(crate) async fn build_basic(
        project: &LoadedProject,
        options: &QueryOptions,
    ) -> Result<QueryRuntime> {
        Self::build_basic_with_factory(project, options, &DefaultModelFactory).await
    }

    async fn build_basic_with_factory(
        project: &LoadedProject,
        options: &QueryOptions,
        model_factory: &dyn ModelFactory,
    ) -> Result<QueryRuntime> {
        let method = SearchMethod::Basic;
        validate_basic_requirements(project, options)?;
        let text_units = load_basic_text_units(project, options).await?;
        let models = create_basic_models(project, model_factory)?;
        let vectors = open_basic_vectors(project).await?;
        let prompt = PromptRepository::new(&project.root)
            .load_configured(
                PromptKind::BasicSearch,
                project.config.basic_search.prompt.as_deref(),
            )
            .await
            .map_err(|source| QueryError::QueryPrompt {
                method,
                operation: "load Basic Search prompt",
                prompt: "basic_search_system_prompt.txt",
                source: Box::new(source),
            })?;
        let callbacks: Arc<dyn QueryCallbacks> =
            Arc::new(QueryCallbackChain::new(options.callbacks.clone()));
        Ok(QueryRuntime {
            basic_context: BasicContextBuilder {
                config: project.config.basic_search.clone(),
                text_units,
                embedding_model: models.embedding,
                embedding_model_id: models.embedding_id,
                vector_store: vectors.store,
                vector_schema: vectors.schema,
                tokenizer: models.tokenizer,
            },
            completion_model: models.completion,
            completion_model_id: models.completion_id,
            completion_config: models.completion_config,
            prompt,
            callbacks,
        })
    }
}

fn validate_basic_requirements(project: &LoadedProject, options: &QueryOptions) -> Result<()> {
    let method = SearchMethod::Basic;
    if options.community_level < 0 {
        return Err(QueryError::InvalidQueryConfig {
            method,
            operation: "validate query options",
            message: "community_level must be non-negative".to_owned(),
        });
    }
    project
        .config
        .basic_search
        .validate()
        .map_err(|message| QueryError::InvalidQueryConfig {
            method,
            operation: "validate Basic Search config",
            message,
        })?;
    let requirements = QueryRequirements::for_method(method, &project.config);
    if requirements.tables.len() == 1 && requirements.embeddings.len() == 1 {
        return Ok(());
    }
    Err(QueryError::QueryRuntime {
        method,
        operation: "resolve Basic Search requirements",
        source: Box::new(std::io::Error::other(
            "Basic Search requirements are internally inconsistent",
        )),
    })
}

async fn load_basic_text_units(
    project: &LoadedProject,
    options: &QueryOptions,
) -> Result<Vec<TextUnit>> {
    let method = SearchMethod::Basic;
    let table_root = resolve_data_root(
        &project.root,
        options.data_dir.as_deref(),
        &project.paths.output_dir,
    );
    if !table_root.is_dir() {
        return Err(QueryError::MissingQueryTable {
            method,
            operation: "open Query table directory",
            table: "text_units",
        });
    }
    let table_storage = Arc::new(FileStorage::existing(&table_root).map_err(|source| {
        QueryError::QueryRuntime {
            method,
            operation: "create Query table provider",
            source: Box::new(source),
        }
    })?);
    let table_provider: Arc<dyn TableProvider> =
        Arc::new(ParquetTableProvider::from_storage(table_storage));
    Ok(QueryDataLoader::new(table_provider)
        .load_basic()
        .await?
        .text_units)
}

fn create_basic_models(
    project: &LoadedProject,
    model_factory: &dyn ModelFactory,
) -> Result<BasicModelResources> {
    let method = SearchMethod::Basic;
    let completion_id = project.config.basic_search.completion_model_id.clone();
    let embedding_id = project.config.basic_search.embedding_model_id.clone();
    let completion_config = required_model(
        &project.config.completion_models,
        &completion_id,
        method,
        "completion",
    )?
    .clone();
    let embedding_config = required_model(
        &project.config.embedding_models,
        &embedding_id,
        method,
        "embedding",
    )?;
    let completion = model_factory
        .create_completion(
            &completion_id,
            &completion_config,
            project.config.concurrent_requests,
        )
        .map_err(|source| QueryError::QueryRuntime {
            method,
            operation: "create Basic completion model",
            source: Box::new(source),
        })?;
    let embedding = model_factory
        .create_embedding(
            &embedding_id,
            embedding_config,
            project.config.concurrent_requests,
        )
        .map_err(|source| QueryError::QueryRuntime {
            method,
            operation: "create Basic embedding model",
            source: Box::new(source),
        })?;
    let tokenizer: Arc<dyn Tokenizer> = Arc::new(
        TiktokenTokenizer::new(completion_config.effective_tokenizer_encoding()).map_err(
            |source| QueryError::QueryRuntime {
                method,
                operation: "create Basic tokenizer",
                source: Box::new(source),
            },
        )?,
    );
    Ok(BasicModelResources {
        completion,
        completion_id,
        completion_config,
        embedding,
        embedding_id,
        tokenizer,
    })
}

async fn open_basic_vectors(project: &LoadedProject) -> Result<BasicVectorResources> {
    let method = SearchMethod::Basic;
    let schema = project
        .config
        .vector_store
        .schema_for(crate::TEXT_UNIT_TEXT_EMBEDDING);
    if !project.paths.vector_db_uri.is_dir() {
        return Err(QueryError::MissingVectorIndex {
            method,
            operation: "open Basic Search vector database",
            index: schema.index_name.clone(),
            source: Box::new(VectorError::MissingIndex {
                index_name: schema.index_name.clone(),
            }),
        });
    }
    let store = create_vector_store(&project.config.vector_store)
        .await
        .map_err(|source| QueryError::QueryRuntime {
            method,
            operation: "connect Query vector store",
            source: Box::new(source),
        })?;
    store.count(&schema).await.map_err(|source| match source {
        source @ VectorError::MissingIndex { .. } => QueryError::MissingVectorIndex {
            method,
            operation: "validate Basic Search vector index",
            index: schema.index_name.clone(),
            source: Box::new(source),
        },
        source => QueryError::InvalidVectorIndex {
            method,
            operation: "validate Basic Search vector index",
            index: schema.index_name.clone(),
            source: Box::new(source),
        },
    })?;
    Ok(BasicVectorResources { store, schema })
}

fn resolve_data_root(root: &Path, override_path: Option<&Path>, configured: &Path) -> PathBuf {
    override_path.map_or_else(
        || configured.to_path_buf(),
        |path| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            }
        },
    )
}

fn required_model<'a>(
    models: &'a std::collections::BTreeMap<String, ModelConfig>,
    id: &str,
    method: SearchMethod,
    kind: &'static str,
) -> Result<&'a ModelConfig> {
    models
        .get(id)
        .ok_or_else(|| QueryError::InvalidQueryConfig {
            method,
            operation: "resolve Query model",
            message: format!("required {kind} model {id:?} is not configured"),
        })
}
