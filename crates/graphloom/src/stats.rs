//! `IndexPipeline` run statistics.

use std::{collections::BTreeMap, time::Duration};

/// Mutable statistics accumulated during a pipeline run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct IndexRunStats {
    /// Number of documents read.
    pub document_count: usize,
    /// Number of text units created.
    pub text_unit_count: usize,
    /// Number of entities created.
    pub entity_count: usize,
    /// Number of relationships created.
    pub relationship_count: usize,
    /// Number of communities created.
    pub community_count: usize,
    /// Number of community reports created.
    pub report_count: usize,
    /// Number of original rows embedded and written to the vector store.
    pub embedding_count: usize,
    /// Number of LLM requests.
    pub llm_request_count: usize,
    /// LLM cache hits.
    pub cache_hit_count: usize,
    /// LLM cache misses.
    pub cache_miss_count: usize,
    /// LLM input tokens.
    pub input_token_count: usize,
    /// LLM output tokens.
    pub output_token_count: usize,
    /// Total elapsed time in milliseconds.
    pub elapsed_ms: u128,
    /// Per-workflow elapsed time in milliseconds.
    pub workflow_elapsed_ms: BTreeMap<String, u128>,
}

impl IndexRunStats {
    /// Record workflow elapsed time.
    pub fn record_workflow_elapsed(&mut self, workflow: &str, elapsed: Duration) {
        self.workflow_elapsed_ms
            .insert(workflow.to_owned(), elapsed.as_millis());
    }
}
