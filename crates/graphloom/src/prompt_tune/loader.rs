//! Document loading and chunking for prompt tuning.

use std::{num::NonZeroUsize, path::Path, sync::Arc};

use futures_util::StreamExt;
use graphloom_chunking::create_chunker;
use graphloom_input::{FileInputReader, InputReader};

use super::selection::{ChunkIdentity, escape_python_format_literal, select_chunks};
use crate::{GraphLoomError, GraphRagConfig, Result};

/// Load documents and chunk them for prompt tuning.
///
/// Uses `FileInputReader` for reading and the project chunking config for
/// splitting documents into prompt-tuning candidates.
///
/// Document braces are doubled after chunking, matching GraphRAG's protection
/// for the generated prompt's later Python `.format()` call.
///
/// # Errors
///
/// Returns an error when the input is empty, chunking config is invalid,
/// or input documents cannot be read.
pub(crate) async fn load_docs_in_chunks(
    config: &GraphRagConfig,
    root: &Path,
    chunk_size: Option<usize>,
    overlap: Option<usize>,
) -> Result<Vec<ChunkIdentity>> {
    // Build effective chunking config with optional overrides
    let mut chunking_config = config.chunking.clone();
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
