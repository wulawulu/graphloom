//! Document loading and chunking for prompt tuning.

use std::{num::NonZeroUsize, path::Path, sync::Arc};

use futures_util::StreamExt;
use graphloom_chunking::create_chunker;
use graphloom_input::{FileInputReader, InputReader};

use super::selection::{ChunkIdentity, escape_python_format_literal, select_chunks};
use crate::{GraphLoomError, GraphRagConfig, Result};

/// Load documents and chunk them for prompt tuning.
///
/// Uses `FileInputReader` for reading and the configured embedding model's
/// tokenizer for splitting documents into prompt-tuning candidates. GraphRAG
/// 3.1.0 constructs the prompt-tune chunker from the embedding model even when
/// the selection method does not call the embedding API.
///
/// Document braces are doubled after chunking, matching GraphRAG's protection
/// for the generated prompt's later Python `.format()` call.
///
/// # Errors
///
/// Returns an error when the embedding model tokenizer is unavailable, the
/// input is empty, chunking config is invalid, or input documents cannot be
/// read.
pub(crate) async fn load_docs_in_chunks(
    config: &GraphRagConfig,
    root: &Path,
    chunk_size: Option<usize>,
    overlap: Option<usize>,
) -> Result<Vec<ChunkIdentity>> {
    let embedding_model_config = config
        .embedding_models
        .get(&config.embed_text.embedding_model_id)
        .ok_or_else(|| GraphLoomError::InvalidModel {
            model_id: config.embed_text.embedding_model_id.clone(),
            message: "prompt tuning requires the configured embedding model tokenizer".to_owned(),
        })?;

    // GraphRAG passes create_embedding(...).tokenizer to create_chunker, so
    // chunking.encoding_model does not control prompt-tune chunk boundaries.
    let mut chunking_config = config.chunking.clone();
    chunking_config.encoding_model = embedding_model_config
        .effective_tokenizer_encoding()
        .to_owned();
    if let Some(size) = chunk_size {
        chunking_config.size = NonZeroUsize::new(size).ok_or_else(|| {
            GraphLoomError::Chunking(graphloom_chunking::ChunkingError::InvalidConfig(format!(
                "chunk size must be >= 1, got {size}"
            )))
        })?;
    }
    if let Some(overlap) = overlap {
        chunking_config.overlap = overlap;
    }
    let chunker = create_chunker(&chunking_config)?;

    let input_dir = root.join(&config.input_storage.base_dir);
    let reader = FileInputReader::with_file_pattern(&input_dir, &config.input.file_pattern)
        .map_err(GraphLoomError::Input)?;

    let mut all_chunks = Vec::new();
    let mut doc_stream = reader.read_documents();
    while let Some(doc) = doc_stream.next().await {
        let doc = doc.map_err(GraphLoomError::Input)?;
        let doc_id: Arc<str> = Arc::from(doc.title.clone());
        let chunks = chunker.chunk(&doc.text, None)?;

        for (ordinal, chunk) in chunks.into_iter().enumerate() {
            let token_count = chunk
                .token_count
                .unwrap_or_else(|| chunk.text.split_whitespace().count());

            all_chunks.push(ChunkIdentity {
                document_id: Arc::clone(&doc_id),
                chunk_text: Arc::from(escape_python_format_literal(&chunk.text)),
                token_count,
                chunk_ordinal: ordinal,
            });
        }
    }

    if all_chunks.is_empty() {
        return Err(GraphLoomError::MissingInput {
            message: "no chunks produced from input documents for prompt tuning".to_owned(),
        });
    }

    Ok(all_chunks)
}

/// Load, chunk, and select documents based on the specified selection method.
///
/// # Errors
///
/// Returns an error when input reading, chunking, or selection fails.
pub(crate) async fn load_and_select_chunks(
    config: &GraphRagConfig,
    root: &Path,
    options: &super::options::GenerateIndexingPromptsOptions,
    embedding_model: Option<&(Arc<dyn graphloom_llm::EmbeddingModel>, usize)>,
) -> Result<Vec<ChunkIdentity>> {
    let chunks = load_docs_in_chunks(config, root, options.chunk_size, options.overlap).await?;

    select_chunks(
        chunks,
        options.selection_method,
        options.limit,
        options.n_subset_max,
        options.k,
        embedding_model,
    )
    .await
}
