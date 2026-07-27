//! GraphRAG 3.1.0 compatible prompt tuning.
//!
//! The public API generates three indexing prompts from project documents:
//! - `extract_graph.txt`
//! - `summarize_descriptions.txt`
//! - `community_report_graph.txt`

mod generator;
mod loader;
mod options;
mod selection;

#[cfg(test)]
mod tests;

use std::{path::Path, sync::Arc};

use generator::PROMPT_TUNING_MODEL_ID;
use graphloom_llm::EmbeddingModel;
use loader::load_and_select_chunks;
pub use options::{DocSelectionType, GenerateIndexingPromptsOptions, GeneratedIndexingPrompts};
pub use selection::ChunkIdentity;
use tracing::info;

use crate::{GraphLoomError, Result, runtime::ModelFactory};

/// Operation identity prefix used for cache isolation.
const CONSUMER_PREFIX: &str = "prompt_tune";

/// Generate indexing prompts from project documents.
///
/// This is the main public API for prompt tuning. It follows the GraphRAG 3.1.0
/// generation sequence exactly.
///
/// # Errors
///
/// Returns an error when configuration is invalid, input is empty, LLM calls
/// fail, or any intermediate step produces unusable output.
pub async fn generate_indexing_prompts(
    options: &GenerateIndexingPromptsOptions,
) -> Result<GeneratedIndexingPrompts> {
    // Load project config
    let project = crate::config::load::load_project_config(&options.root).await?;
    let config = &project.config;

    // Create completion model for prompt tuning (with or without cache)
    let model_config = config
        .completion_models
        .get(PROMPT_TUNING_MODEL_ID)
        .ok_or_else(|| GraphLoomError::InvalidModel {
            model_id: PROMPT_TUNING_MODEL_ID.to_owned(),
            message: "prompt tuning requires a configured default_completion_model".to_owned(),
        })?;

    let llm = create_prompt_tune_completion(
        model_config,
        config.concurrent_requests,
        &project.paths,
        options.cache_enabled,
    )?;

    // Create embedding model when auto selection is needed
    let embedding: Option<(Arc<dyn EmbeddingModel>, usize)> =
        if options.selection_method == DocSelectionType::Auto {
            let emb_config = config
                .embedding_models
                .get(&config.embed_text.embedding_model_id)
                .ok_or_else(|| GraphLoomError::InvalidModel {
                    model_id: config.embed_text.embedding_model_id.clone(),
                    message: "auto selection requires an embedding model".to_owned(),
                })?;
            let emb_model = crate::runtime::DefaultModelFactory.create_embedding(
                &config.embed_text.embedding_model_id,
                emb_config,
                config.concurrent_requests,
            )?;
            // Optionally wrap with cache
            let emb: Arc<dyn EmbeddingModel> = if options.cache_enabled {
                let cache_storage = graphloom_storage::FileStorage::new(&project.paths.cache_dir)
                    .map_err(GraphLoomError::Storage)?;
                let cache = Arc::new(graphloom_cache::JsonCache::new(Arc::new(cache_storage)));
                Arc::new(graphloom_llm::CachedEmbeddingModel::new(emb_model, cache))
            } else {
                emb_model
            };
            Some((emb, config.embed_text.batch_size))
        } else {
            None
        };

    // Load, chunk, and SELECT documents
    info!("Loading and selecting document chunks...");
    let doc_chunks =
        load_and_select_chunks(config, &project.root, options, embedding.as_ref()).await?;

    if doc_chunks.is_empty() {
        return Err(GraphLoomError::MissingInput {
            message: "no document chunks selected for prompt tuning".to_owned(),
        });
    }

    // Extract text from selected chunks for LLM prompts
    let docs: Vec<String> = doc_chunks
        .iter()
        .map(|c| c.chunk_text.as_ref().to_owned())
        .collect();

    // 3. Generate domain (or use provided)
    let domain = match &options.domain {
        Some(d) => d.clone(),
        None => {
            info!("Generating domain...");
            generator::generate_domain(&llm, &docs, CONSUMER_PREFIX).await?
        }
    };

    // 4. Detect language (or use provided)
    let language = match &options.language {
        Some(l) => l.clone(),
        None => {
            info!("Detecting language...");
            generator::detect_language(&llm, &docs, CONSUMER_PREFIX).await?
        }
    };

    // 5. Generate persona
    info!("Generating persona...");
    let persona = generator::generate_persona(&llm, &domain, CONSUMER_PREFIX).await?;

    // 6. Generate community report rating
    info!("Generating community report ranking description...");
    let community_report_ranking = generator::generate_community_report_rating(
        &llm,
        &domain,
        &persona,
        &docs,
        CONSUMER_PREFIX,
    )
    .await?;

    // 7. Optionally discover entity types
    let entity_types: Option<Vec<String>> = if options.discover_entity_types {
        info!("Generating entity types...");
        let types =
            generator::generate_entity_types(&llm, &domain, &persona, &docs, true, CONSUMER_PREFIX)
                .await?;
        if types.is_empty() { None } else { Some(types) }
    } else {
        None
    };

    // 8. Generate entity/relationship examples
    info!("Generating entity relationship examples...");
    let examples = generator::generate_entity_relationship_examples(
        &llm,
        &persona,
        entity_types.as_deref(),
        &docs,
        &language,
        CONSUMER_PREFIX,
    )
    .await?;

    // 9. Assemble extract_graph prompt
    // GraphRAG 3.1.0: use extract_graph model's tokenizer, not prompt-tune model's tokenizer
    info!("Generating entity extraction prompt...");
    let extract_graph_model_config = config
        .completion_models
        .get(&config.extract_graph.completion_model_id)
        .ok_or_else(|| GraphLoomError::InvalidModel {
            model_id: config.extract_graph.completion_model_id.clone(),
            message: "extract_graph model not configured".to_owned(),
        })?;
    let tokenizer = graphloom_llm::TiktokenTokenizer::new(
        extract_graph_model_config.effective_tokenizer_encoding(),
    )
    .map_err(GraphLoomError::Llm)?;
    let extract_graph = generator::create_extract_graph_prompt(
        entity_types.as_deref(),
        &docs,
        &examples,
        &language,
        options.max_tokens,
        &tokenizer,
        options.min_examples_required,
    )?;

    // 10. Assemble summarize_descriptions prompt
    info!("Generating entity summarization prompt...");
    let summarize_descriptions = generator::create_entity_summarization_prompt(&persona, &language);

    // 11. Generate community reporter role
    info!("Generating community reporter role...");
    let community_reporter_role = generator::generate_community_reporter_role(
        &llm,
        &domain,
        &persona,
        &docs,
        CONSUMER_PREFIX,
    )
    .await?;

    // 12. Assemble community_report_graph prompt
    info!("Generating community summarization prompt...");
    let community_report_graph = generator::create_community_summarization_prompt(
        &persona,
        &community_reporter_role,
        &community_report_ranking,
        &language,
    );

    Ok(GeneratedIndexingPrompts {
        extract_graph,
        summarize_descriptions,
        community_report_graph,
    })
}

/// Create a completion model, optionally wrapped with cache middleware.
fn create_prompt_tune_completion(
    model_config: &graphloom_llm::ModelConfig,
    concurrent_requests: usize,
    paths: &crate::project::ProjectPaths,
    cache_enabled: bool,
) -> Result<Arc<dyn graphloom_llm::CompletionModel>> {
    use crate::runtime::ModelFactory;

    let model = crate::runtime::DefaultModelFactory.create_completion(
        PROMPT_TUNING_MODEL_ID,
        model_config,
        concurrent_requests,
    )?;

    if !cache_enabled {
        return Ok(model);
    }

    let cache_storage =
        graphloom_storage::FileStorage::new(&paths.cache_dir).map_err(GraphLoomError::Storage)?;
    let cache = Arc::new(graphloom_cache::JsonCache::new(Arc::new(cache_storage)));

    Ok(Arc::new(graphloom_llm::CachedCompletionModel::new(
        model, cache,
    )))
}

/// Load documents and chunk them for prompt tuning, without running LLM calls.
///
/// Use this to preview what chunks would be selected.
///
/// # Errors
///
/// Returns an error when input reading, chunking, or selection fails.
pub async fn load_and_chunk_docs(
    config: &crate::GraphRagConfig,
    root: &Path,
    options: &GenerateIndexingPromptsOptions,
    embedding_model: Option<(Arc<dyn EmbeddingModel>, usize)>,
) -> Result<Vec<ChunkIdentity>> {
    load_and_select_chunks(config, root, options, embedding_model.as_ref()).await
}
