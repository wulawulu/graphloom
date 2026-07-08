//! Embedding operation domain types.

use graphloom_vectors::VectorDocument;

/// Source row prepared for embedding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EmbeddingSourceRow {
    pub(crate) id: String,
    pub(crate) text: String,
}

/// Embedding operation configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EmbeddingOperationConfig {
    pub(crate) batch_size: usize,
    pub(crate) batch_max_tokens: usize,
    pub(crate) concurrency: usize,
    pub(crate) chunk_overlap: usize,
    pub(crate) expected_vector_size: usize,
    pub(crate) model_instance_name: String,
    pub(crate) embedding_name: String,
}

/// Embedding operation output.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct EmbeddingBatchOutput {
    pub(crate) documents: Vec<VectorDocument>,
    pub(crate) attempted_rows: usize,
    pub(crate) skipped_rows: usize,
    pub(crate) snippet_count: usize,
    pub(crate) request_count: usize,
    pub(crate) cache_hits: usize,
    pub(crate) cache_misses: usize,
    pub(crate) input_tokens: usize,
}
