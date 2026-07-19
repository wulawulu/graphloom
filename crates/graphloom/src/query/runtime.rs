//! Read-only Query runtime assembly.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use graphloom_llm::{CompletionModel, EmbeddingModel, ModelConfig, TiktokenTokenizer, Tokenizer};
use graphloom_storage::{FileStorage, ParquetTableProvider, TableProvider};
use graphloom_vectors::{VectorError, VectorIndexSchema, VectorStore, create_vector_store};

use super::{
    QueryCallbackChain, QueryCallbacks, QueryDataIndex, QueryError, QueryOptions, Result,
    SearchMethod, TextUnit,
    basic::BasicContextBuilder,
    data_loader::{DriftQueryData, GlobalQueryData, LocalQueryData, QueryDataLoader},
    drift::DriftContextBuilder,
    global::GlobalContextBuilder,
    local::LocalContextBuilder,
};
use crate::{
    project::LoadedProject,
    prompts::{PromptKind, PromptRepository, PromptTemplate},
    runtime::{DefaultModelFactory, ModelFactory},
};

/// Prepared resources for one Basic Search invocation.
#[derive(Debug, Clone)]
pub(crate) struct BasicQueryRuntime {
    pub(crate) basic_context: Arc<BasicContextBuilder>,
    pub(crate) completion_model: Arc<dyn CompletionModel>,
    pub(crate) completion_model_id: String,
    pub(crate) completion_config: ModelConfig,
    pub(crate) prompt: PromptTemplate,
    pub(crate) callbacks: Arc<dyn QueryCallbacks>,
}

/// Prepared resources for one Local Search invocation.
#[derive(Debug, Clone)]
pub(crate) struct LocalQueryRuntime {
    pub(crate) local_context: Arc<LocalContextBuilder>,
    pub(crate) completion_model: Arc<dyn CompletionModel>,
    pub(crate) completion_model_id: String,
    pub(crate) completion_config: ModelConfig,
    pub(crate) prompt: PromptTemplate,
    pub(crate) callbacks: Arc<dyn QueryCallbacks>,
}

/// Prepared resources for one Global Search invocation.
#[derive(Debug, Clone)]
pub(crate) struct GlobalQueryRuntime {
    pub(crate) global_context: Arc<GlobalContextBuilder>,
    pub(crate) completion_model: Arc<dyn CompletionModel>,
    pub(crate) completion_model_id: String,
    pub(crate) completion_config: ModelConfig,
    pub(crate) map_prompt: PromptTemplate,
    pub(crate) reduce_prompt: PromptTemplate,
    // GraphRAG 3.1.0 validates this configured prompt during runtime assembly even though the
    // CLI keeps allow_general_knowledge disabled and does not render it.
    pub(crate) _knowledge_prompt: PromptTemplate,
    pub(crate) callbacks: Arc<dyn QueryCallbacks>,
    pub(crate) concurrent_requests: usize,
    pub(crate) dynamic_community_selection: bool,
}

/// Prepared resources for one DRIFT Search invocation.
#[derive(Debug, Clone)]
pub(crate) struct DriftQueryRuntime {
    pub(crate) context: Arc<DriftContextBuilder>,
    pub(crate) local_prompt: PromptTemplate,
    pub(crate) reduce_prompt: PromptTemplate,
    pub(crate) callbacks: Arc<dyn QueryCallbacks>,
}

struct QueryCompletionResources {
    completion: Arc<dyn CompletionModel>,
    completion_id: String,
    completion_config: ModelConfig,
    tokenizer: Arc<dyn Tokenizer>,
}

struct QueryModelResources {
    completion: Arc<dyn CompletionModel>,
    completion_id: String,
    completion_config: ModelConfig,
    embedding: Arc<dyn EmbeddingModel>,
    embedding_id: String,
    tokenizer: Arc<dyn Tokenizer>,
}

struct QueryVectorResources {
    store: Arc<dyn VectorStore>,
    schema: VectorIndexSchema,
}

struct DriftVectorResources {
    store: Arc<dyn VectorStore>,
    entity_schema: VectorIndexSchema,
    community_schema: VectorIndexSchema,
}

/// Factory that assembles only resources required by the active Query method.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct QueryRuntimeFactory;

impl QueryRuntimeFactory {
    pub(crate) async fn build_basic(
        project: &LoadedProject,
        options: &QueryOptions,
    ) -> Result<BasicQueryRuntime> {
        Self::build_basic_with_factory(project, options, &DefaultModelFactory).await
    }

    pub(crate) async fn build_basic_with_factory(
        project: &LoadedProject,
        options: &QueryOptions,
        model_factory: &dyn ModelFactory,
    ) -> Result<BasicQueryRuntime> {
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
        Ok(BasicQueryRuntime {
            basic_context: Arc::new(BasicContextBuilder {
                config: project.config.basic_search.clone(),
                text_units,
                embedding_model: models.embedding,
                embedding_model_id: models.embedding_id,
                vector_store: vectors.store,
                vector_schema: vectors.schema,
                tokenizer: models.tokenizer,
            }),
            completion_model: models.completion,
            completion_model_id: models.completion_id,
            completion_config: models.completion_config,
            prompt,
            callbacks,
        })
    }

    pub(crate) async fn build_local(
        project: &LoadedProject,
        options: &QueryOptions,
    ) -> Result<LocalQueryRuntime> {
        Self::build_local_with_factory(project, options, &DefaultModelFactory).await
    }

    pub(crate) async fn build_local_with_factory(
        project: &LoadedProject,
        options: &QueryOptions,
        model_factory: &dyn ModelFactory,
    ) -> Result<LocalQueryRuntime> {
        let method = SearchMethod::Local;
        validate_local_requirements(project, options)?;
        let data = load_local_data(project, options).await?;
        let index = Arc::new(QueryDataIndex::new(
            &data.entities,
            &data.reports,
            &data.text_units,
            &data.relationships,
            &data.covariates,
        ));
        let models = create_local_models(project, model_factory)?;
        let vectors = open_local_vectors(project).await?;
        let prompt = PromptRepository::new(&project.root)
            .load_configured(
                PromptKind::LocalSearch,
                project.config.local_search.prompt.as_deref(),
            )
            .await
            .map_err(|source| QueryError::QueryPrompt {
                method,
                operation: "load Local Search prompt",
                prompt: "local_search_system_prompt.txt",
                source: Box::new(source),
            })?;
        let callbacks: Arc<dyn QueryCallbacks> =
            Arc::new(QueryCallbackChain::new(options.callbacks.clone()));
        Ok(LocalQueryRuntime {
            local_context: Arc::new(LocalContextBuilder {
                method,
                config: project.config.local_search.clone(),
                entities: data.entities,
                reports: data.reports,
                text_units: data.text_units,
                relationships: data.relationships,
                covariates: data.covariates,
                index,
                embedding_model: models.embedding,
                embedding_model_id: models.embedding_id,
                vector_store: vectors.store,
                vector_schema: vectors.schema,
                tokenizer: models.tokenizer,
            }),
            completion_model: models.completion,
            completion_model_id: models.completion_id,
            completion_config: models.completion_config,
            prompt,
            callbacks,
        })
    }

    pub(crate) async fn build_global(
        project: &LoadedProject,
        options: &QueryOptions,
    ) -> Result<GlobalQueryRuntime> {
        Self::build_global_with_factory(project, options, &DefaultModelFactory).await
    }

    pub(crate) async fn build_global_with_factory(
        project: &LoadedProject,
        options: &QueryOptions,
        model_factory: &dyn ModelFactory,
    ) -> Result<GlobalQueryRuntime> {
        validate_global_requirements(project, options)?;
        let data = load_global_data(project, options).await?;
        let models = create_global_models(project, model_factory)?;
        let repository = PromptRepository::new(&project.root);
        let map_prompt = load_global_prompt(
            &repository,
            PromptKind::GlobalSearchMap,
            project.config.global_search.map_prompt.as_deref(),
            "load Global Search map prompt",
            "global_search_map_system_prompt.txt",
        )
        .await?;
        let reduce_prompt = load_global_prompt(
            &repository,
            PromptKind::GlobalSearchReduce,
            project.config.global_search.reduce_prompt.as_deref(),
            "load Global Search reduce prompt",
            "global_search_reduce_system_prompt.txt",
        )
        .await?;
        let knowledge_prompt = load_global_prompt(
            &repository,
            PromptKind::GlobalSearchKnowledge,
            project.config.global_search.knowledge_prompt.as_deref(),
            "load Global Search knowledge prompt",
            "global_search_knowledge_system_prompt.txt",
        )
        .await?;
        let callbacks: Arc<dyn QueryCallbacks> =
            Arc::new(QueryCallbackChain::new(options.callbacks.clone()));
        Ok(GlobalQueryRuntime {
            global_context: Arc::new(GlobalContextBuilder::new(
                project.config.global_search.clone(),
                data,
                models.tokenizer,
            )),
            completion_model: models.completion,
            completion_model_id: models.completion_id,
            completion_config: models.completion_config,
            map_prompt,
            reduce_prompt,
            _knowledge_prompt: knowledge_prompt,
            callbacks,
            concurrent_requests: project.config.concurrent_requests,
            dynamic_community_selection: options.dynamic_community_selection,
        })
    }

    pub(crate) async fn build_drift(
        project: &LoadedProject,
        options: &QueryOptions,
    ) -> Result<DriftQueryRuntime> {
        Self::build_drift_with_factory(project, options, &DefaultModelFactory).await
    }

    pub(crate) async fn build_drift_with_factory(
        project: &LoadedProject,
        options: &QueryOptions,
        model_factory: &dyn ModelFactory,
    ) -> Result<DriftQueryRuntime> {
        let method = SearchMethod::Drift;
        validate_drift_requirements(project, options)?;
        let data = load_drift_data(project, options).await?;
        let index = Arc::new(QueryDataIndex::new(
            &data.entities,
            &data.reports,
            &data.text_units,
            &data.relationships,
            &[],
        ));
        let models = create_query_models(
            project,
            model_factory,
            method,
            &project.config.drift_search.completion_model_id,
            &project.config.drift_search.embedding_model_id,
        )?;
        let vectors = open_drift_vectors(project).await?;
        let repository = PromptRepository::new(&project.root);
        let local_prompt = repository
            .load_configured(
                PromptKind::DriftSearch,
                project.config.drift_search.prompt.as_deref(),
            )
            .await
            .map_err(|source| QueryError::QueryPrompt {
                method,
                operation: "load DRIFT Local prompt",
                prompt: "drift_search_system_prompt.txt",
                source: Box::new(source),
            })?;
        let reduce_prompt = repository
            .load_configured(
                PromptKind::DriftReduce,
                project.config.drift_search.reduce_prompt.as_deref(),
            )
            .await
            .map_err(|source| QueryError::QueryPrompt {
                method,
                operation: "load DRIFT reduce prompt",
                prompt: "drift_reduce_prompt.txt",
                source: Box::new(source),
            })?;
        let local_config = drift_local_config(&project.config.drift_search);
        let local = LocalContextBuilder {
            method,
            config: local_config,
            entities: data.entities,
            reports: data.reports.clone(),
            text_units: data.text_units,
            relationships: data.relationships,
            covariates: Vec::new(),
            index,
            embedding_model: Arc::clone(&models.embedding),
            embedding_model_id: models.embedding_id.clone(),
            vector_store: Arc::clone(&vectors.store),
            vector_schema: vectors.entity_schema,
            tokenizer: Arc::clone(&models.tokenizer),
        };
        let callbacks: Arc<dyn QueryCallbacks> =
            Arc::new(QueryCallbackChain::new(options.callbacks.clone()));
        let mut context = DriftContextBuilder {
            config: project.config.drift_search.clone(),
            reports: data.reports,
            local,
            completion_model: models.completion,
            embedding_model: models.embedding,
            completion_model_id: models.completion_id,
            embedding_model_id: models.embedding_id,
            completion_config: models.completion_config,
            vector_store: vectors.store,
            community_schema: vectors.community_schema,
            tokenizer: models.tokenizer,
        };
        context.hydrate_reports().await?;
        Ok(DriftQueryRuntime {
            context: Arc::new(context),
            local_prompt,
            reduce_prompt,
            callbacks,
        })
    }
}

fn drift_local_config(config: &crate::DriftSearchConfig) -> crate::LocalSearchConfig {
    crate::LocalSearchConfig {
        prompt: config.prompt.clone(),
        completion_model_id: config.completion_model_id.clone(),
        embedding_model_id: config.embedding_model_id.clone(),
        text_unit_prop: config.local_search_text_unit_prop,
        community_prop: config.local_search_community_prop,
        conversation_history_max_turns: 5,
        top_k_entities: config.local_search_top_k_mapped_entities,
        top_k_relationships: config.local_search_top_k_relationships,
        max_context_tokens: config.local_search_max_data_tokens,
    }
}

async fn load_global_prompt(
    repository: &PromptRepository,
    kind: PromptKind,
    configured: Option<&str>,
    operation: &'static str,
    prompt: &'static str,
) -> Result<PromptTemplate> {
    repository
        .load_configured(kind, configured)
        .await
        .map_err(|source| QueryError::QueryPrompt {
            method: SearchMethod::Global,
            operation,
            prompt,
            source: Box::new(source),
        })
}

pub(crate) fn validate_basic_requirements(
    project: &LoadedProject,
    options: &QueryOptions,
) -> Result<()> {
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
    Ok(())
}

pub(crate) fn validate_local_requirements(
    project: &LoadedProject,
    options: &QueryOptions,
) -> Result<()> {
    let method = SearchMethod::Local;
    if options.community_level < 0 {
        return Err(QueryError::InvalidQueryConfig {
            method,
            operation: "validate query options",
            message: "community_level must be non-negative".to_owned(),
        });
    }
    project
        .config
        .local_search
        .validate()
        .map_err(|message| QueryError::InvalidQueryConfig {
            method,
            operation: "validate Local Search config",
            message,
        })?;
    if let Some(history) = &options.conversation_history {
        history
            .validate()
            .map_err(|message| QueryError::InvalidQueryConfig {
                method,
                operation: "validate Local Search conversation history",
                message,
            })?;
    }
    Ok(())
}

pub(crate) fn validate_global_requirements(
    project: &LoadedProject,
    options: &QueryOptions,
) -> Result<()> {
    let method = SearchMethod::Global;
    if options.community_level < 0 {
        return Err(QueryError::InvalidQueryConfig {
            method,
            operation: "validate query options",
            message: "community_level must be non-negative".to_owned(),
        });
    }
    project
        .config
        .global_search
        .validate()
        .map_err(|message| QueryError::InvalidQueryConfig {
            method,
            operation: "validate Global Search config",
            message,
        })?;
    if project.config.concurrent_requests == 0 {
        return Err(QueryError::InvalidQueryConfig {
            method,
            operation: "validate Global Search config",
            message: "concurrent_requests must be greater than zero".to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn validate_drift_requirements(
    project: &LoadedProject,
    options: &QueryOptions,
) -> Result<()> {
    let method = SearchMethod::Drift;
    if options.query.is_empty() {
        return Err(QueryError::InvalidQueryConfig {
            method,
            operation: "validate DRIFT Search query",
            message: "DRIFT Search query cannot be empty".to_owned(),
        });
    }
    if options.community_level < 0 {
        return Err(QueryError::InvalidQueryConfig {
            method,
            operation: "validate query options",
            message: "community_level must be non-negative".to_owned(),
        });
    }
    project
        .config
        .drift_search
        .validate()
        .map_err(|message| QueryError::InvalidQueryConfig {
            method,
            operation: "validate DRIFT Search config",
            message,
        })?;
    Ok(())
}

async fn load_basic_text_units(
    project: &LoadedProject,
    options: &QueryOptions,
) -> Result<Vec<TextUnit>> {
    let table_provider = open_table_provider(project, options, SearchMethod::Basic, "text_units")?;
    Ok(QueryDataLoader::new(table_provider)
        .load_basic()
        .await?
        .text_units)
}

async fn load_local_data(
    project: &LoadedProject,
    options: &QueryOptions,
) -> Result<LocalQueryData> {
    let table_provider = open_table_provider(project, options, SearchMethod::Local, "entities")?;
    QueryDataLoader::new(table_provider)
        .load_local(options.community_level)
        .await
}

async fn load_global_data(
    project: &LoadedProject,
    options: &QueryOptions,
) -> Result<GlobalQueryData> {
    let table_provider = open_table_provider(project, options, SearchMethod::Global, "entities")?;
    QueryDataLoader::new(table_provider)
        .load_global(options.community_level, options.dynamic_community_selection)
        .await
}

async fn load_drift_data(
    project: &LoadedProject,
    options: &QueryOptions,
) -> Result<DriftQueryData> {
    let table_provider = open_table_provider(project, options, SearchMethod::Drift, "entities")?;
    QueryDataLoader::new(table_provider)
        .load_drift(options.community_level)
        .await
}

fn create_basic_models(
    project: &LoadedProject,
    model_factory: &dyn ModelFactory,
) -> Result<QueryModelResources> {
    create_query_models(
        project,
        model_factory,
        SearchMethod::Basic,
        &project.config.basic_search.completion_model_id,
        &project.config.basic_search.embedding_model_id,
    )
}

fn create_local_models(
    project: &LoadedProject,
    model_factory: &dyn ModelFactory,
) -> Result<QueryModelResources> {
    create_query_models(
        project,
        model_factory,
        SearchMethod::Local,
        &project.config.local_search.completion_model_id,
        &project.config.local_search.embedding_model_id,
    )
}

fn create_global_models(
    project: &LoadedProject,
    model_factory: &dyn ModelFactory,
) -> Result<QueryCompletionResources> {
    let method = SearchMethod::Global;
    let completion_id = &project.config.global_search.completion_model_id;
    let completion_config = required_model(
        &project.config.completion_models,
        completion_id,
        method,
        "completion",
    )?
    .clone();
    let completion = model_factory
        .create_completion(
            completion_id,
            &completion_config,
            project.config.concurrent_requests,
        )
        .map_err(|source| QueryError::QueryRuntime {
            method,
            operation: "create Global completion model",
            source: Box::new(source),
        })?;
    let tokenizer: Arc<dyn Tokenizer> = Arc::new(
        TiktokenTokenizer::new(completion_config.effective_tokenizer_encoding()).map_err(
            |source| QueryError::QueryRuntime {
                method,
                operation: "create Global tokenizer",
                source: Box::new(source),
            },
        )?,
    );
    Ok(QueryCompletionResources {
        completion,
        completion_id: completion_id.clone(),
        completion_config,
        tokenizer,
    })
}

async fn open_basic_vectors(project: &LoadedProject) -> Result<QueryVectorResources> {
    open_query_vectors(
        project,
        SearchMethod::Basic,
        crate::TEXT_UNIT_TEXT_EMBEDDING,
    )
    .await
}

async fn open_local_vectors(project: &LoadedProject) -> Result<QueryVectorResources> {
    open_query_vectors(
        project,
        SearchMethod::Local,
        crate::ENTITY_DESCRIPTION_EMBEDDING,
    )
    .await
}

async fn open_drift_vectors(project: &LoadedProject) -> Result<DriftVectorResources> {
    let method = SearchMethod::Drift;
    let entity_schema = project
        .config
        .vector_store
        .schema_for(crate::ENTITY_DESCRIPTION_EMBEDDING);
    let community_schema = project
        .config
        .vector_store
        .schema_for(crate::COMMUNITY_FULL_CONTENT_EMBEDDING);
    if !project.paths.vector_db_uri.is_dir() {
        return Err(QueryError::MissingVectorIndex {
            method,
            operation: "open DRIFT vector database",
            index: community_schema.index_name.clone(),
            source: Box::new(VectorError::MissingIndex {
                index_name: community_schema.index_name.clone(),
            }),
        });
    }
    let store = create_vector_store(&project.config.vector_store)
        .await
        .map_err(|source| QueryError::QueryRuntime {
            method,
            operation: "connect DRIFT vector store",
            source: Box::new(source),
        })?;
    for schema in [&entity_schema, &community_schema] {
        store.count(schema).await.map_err(|source| match source {
            source @ VectorError::MissingIndex { .. } => QueryError::MissingVectorIndex {
                method,
                operation: "validate DRIFT vector index",
                index: schema.index_name.clone(),
                source: Box::new(source),
            },
            source => QueryError::InvalidVectorIndex {
                method,
                operation: "validate DRIFT vector index",
                index: schema.index_name.clone(),
                source: Box::new(source),
            },
        })?;
    }
    Ok(DriftVectorResources {
        store,
        entity_schema,
        community_schema,
    })
}

fn open_table_provider(
    project: &LoadedProject,
    options: &QueryOptions,
    method: SearchMethod,
    representative_table: &'static str,
) -> Result<Arc<dyn TableProvider>> {
    let table_root = resolve_data_root(
        &project.root,
        options.data_dir.as_deref(),
        &project.paths.output_dir,
    );
    if !table_root.is_dir() {
        return Err(QueryError::MissingQueryTable {
            method,
            operation: "open Query table directory",
            table: representative_table,
        });
    }
    let table_storage = Arc::new(FileStorage::existing(&table_root).map_err(|source| {
        QueryError::QueryRuntime {
            method,
            operation: "create Query table provider",
            source: Box::new(source),
        }
    })?);
    Ok(Arc::new(ParquetTableProvider::from_storage(table_storage)))
}

fn create_query_models(
    project: &LoadedProject,
    model_factory: &dyn ModelFactory,
    method: SearchMethod,
    completion_id: &str,
    embedding_id: &str,
) -> Result<QueryModelResources> {
    let completion_config = required_model(
        &project.config.completion_models,
        completion_id,
        method,
        "completion",
    )?
    .clone();
    let embedding_config = required_model(
        &project.config.embedding_models,
        embedding_id,
        method,
        "embedding",
    )?;
    let completion = model_factory
        .create_completion(
            completion_id,
            &completion_config,
            project.config.concurrent_requests,
        )
        .map_err(|source| QueryError::QueryRuntime {
            method,
            operation: match method {
                SearchMethod::Basic => "create Basic completion model",
                SearchMethod::Local => "create Local completion model",
                _ => "create Query completion model",
            },
            source: Box::new(source),
        })?;
    let embedding = model_factory
        .create_embedding(
            embedding_id,
            embedding_config,
            project.config.concurrent_requests,
        )
        .map_err(|source| QueryError::QueryRuntime {
            method,
            operation: match method {
                SearchMethod::Basic => "create Basic embedding model",
                SearchMethod::Local => "create Local embedding model",
                _ => "create Query embedding model",
            },
            source: Box::new(source),
        })?;
    let tokenizer: Arc<dyn Tokenizer> = Arc::new(
        TiktokenTokenizer::new(completion_config.effective_tokenizer_encoding()).map_err(
            |source| QueryError::QueryRuntime {
                method,
                operation: match method {
                    SearchMethod::Basic => "create Basic tokenizer",
                    SearchMethod::Local => "create Local tokenizer",
                    _ => "create Query tokenizer",
                },
                source: Box::new(source),
            },
        )?,
    );
    tracing::debug!(method = %method, "created method-specific Query models");
    Ok(QueryModelResources {
        completion,
        completion_id: completion_id.to_owned(),
        completion_config,
        embedding,
        embedding_id: embedding_id.to_owned(),
        tokenizer,
    })
}

async fn open_query_vectors(
    project: &LoadedProject,
    method: SearchMethod,
    embedding_name: &str,
) -> Result<QueryVectorResources> {
    let schema = project.config.vector_store.schema_for(embedding_name);
    if !project.paths.vector_db_uri.is_dir() {
        return Err(QueryError::MissingVectorIndex {
            method,
            operation: match method {
                SearchMethod::Basic => "open Basic Search vector database",
                SearchMethod::Local => "open Local Search vector database",
                _ => "open Query vector database",
            },
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
            operation: match method {
                SearchMethod::Basic => "validate Basic Search vector index",
                SearchMethod::Local => "validate Local Search vector index",
                _ => "validate Query vector index",
            },
            index: schema.index_name.clone(),
            source: Box::new(source),
        },
        source => QueryError::InvalidVectorIndex {
            method,
            operation: match method {
                SearchMethod::Basic => "validate Basic Search vector index",
                SearchMethod::Local => "validate Local Search vector index",
                _ => "validate Query vector index",
            },
            index: schema.index_name.clone(),
            source: Box::new(source),
        },
    })?;
    tracing::debug!(method = %method, index = %schema.index_name, "opened Query vector index");
    Ok(QueryVectorResources { store, schema })
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

#[cfg(test)]
mod tests {
    use super::drift_local_config;
    use crate::DriftSearchConfig;

    #[test]
    fn test_should_map_only_drift_local_context_fields() {
        let config = DriftSearchConfig {
            local_search_text_unit_prop: 0.8,
            local_search_community_prop: 0.2,
            local_search_top_k_mapped_entities: 7,
            local_search_top_k_relationships: 9,
            local_search_max_data_tokens: 321,
            ..DriftSearchConfig::default()
        };

        let local = drift_local_config(&config);

        assert_eq!(local.text_unit_prop, 0.8);
        assert_eq!(local.community_prop, 0.2);
        assert_eq!(local.top_k_entities, 7);
        assert_eq!(local.top_k_relationships, 9);
        assert_eq!(local.max_context_tokens, 321);
        assert_eq!(local.completion_model_id, config.completion_model_id);
        assert_eq!(local.embedding_model_id, config.embedding_model_id);
    }
}
