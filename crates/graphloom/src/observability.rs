//! Stable `GraphLoom` Observability contract.
//!
//! This module defines the stable names, field types, and status values that
//! `GraphLoom` emits through `tracing` spans and events. It is the input contract
//! for future OpenTelemetry adapters: an adapter can rely on these names without
//! coupling to internal module paths or function names.
//!
//! This module only defines the contract. It never initializes a subscriber,
//! creates spans, or records events.
//!
//! # Stability
//!
//! The contract version is [`OBSERVABILITY_CONTRACT_VERSION`]. Version upgrades
//! follow these rules:
//!
//! * Adding an optional span, event, or field does not require an upgrade.
//! * Removing a stable field requires an upgrade.
//! * Renaming a stable field requires an upgrade.
//! * Changing the semantic meaning or type of a field value requires an upgrade.
//! * Changing the core parent/child meaning of a span requires an upgrade.

/// Version of the stable `GraphLoom` Observability contract.
pub const OBSERVABILITY_CONTRACT_VERSION: u32 = 1;

/// Stable `tracing` span names.
///
/// Span names are low-cardinality, dot-separated identifiers. Dynamic values
/// (query text, IDs, model names, paths) must be recorded as fields, never
/// appended to these names.
pub mod span_name {
    /// Request-scoped Local Query root span.
    pub const QUERY_LOCAL: &str = "graphloom.query.local";
    /// Local Query runtime lookup and resource assembly.
    pub const QUERY_RUNTIME: &str = "graphloom.query.runtime";
    /// Local Query context construction.
    pub const QUERY_CONTEXT: &str = "graphloom.query.context";
    /// Local Query entity mapping.
    pub const QUERY_ENTITY_MAPPING: &str = "graphloom.query.entity_mapping";
    /// One real embedding provider request.
    pub const EMBEDDING_REQUEST: &str = "graphloom.embedding.request";
    /// One real vector store search.
    pub const VECTOR_SEARCH: &str = "graphloom.vector.search";
    /// Local Query graph expansion.
    pub const QUERY_GRAPH_EXPANSION: &str = "graphloom.query.graph_expansion";
    /// Local Query prompt bind/render/validation.
    pub const QUERY_PROMPT: &str = "graphloom.query.prompt";
    /// One completion provider request, including stream consumption.
    pub const LLM_REQUEST: &str = "graphloom.llm.request";
}

/// Stable `tracing` event names.
///
/// Events are named with the stable identifier so adapters can route them
/// without parsing the human-readable message.
pub mod event_name {
    /// CLI Query run started.
    pub const CLI_QUERY_STARTED: &str = "graphloom.cli.query.started";
    /// CLI Query run completed.
    pub const CLI_QUERY_COMPLETED: &str = "graphloom.cli.query.completed";
    /// CLI Query run failed.
    pub const CLI_QUERY_FAILED: &str = "graphloom.cli.query.failed";
    /// CLI Explainability JSONL output enabled.
    pub const CLI_EXPLAINABILITY_ENABLED: &str = "graphloom.cli.explainability.enabled";
    /// CLI Explainability Recorder shutdown failed.
    pub const CLI_EXPLAINABILITY_SHUTDOWN_FAILED: &str =
        "graphloom.cli.explainability.shutdown_failed";
    /// Explainability record delivery to the sink failed.
    pub const QUERY_EXPLAINABILITY_DELIVERY_FAILED: &str =
        "graphloom.query.explainability.delivery_failed";
    /// Explainability event failed contract validation.
    pub const QUERY_EXPLAINABILITY_CONTRACT_FAILED: &str =
        "graphloom.query.explainability.contract_failed";
    /// Explainability sidecar data is incomplete.
    pub const QUERY_EXPLAINABILITY_SIDECAR_INCOMPLETE: &str =
        "graphloom.query.explainability.sidecar_incomplete";
    /// Explainability sink finalization failed.
    pub const QUERY_EXPLAINABILITY_FINISH_FAILED: &str =
        "graphloom.query.explainability.finish_failed";
    /// Entity mapping ignored a stale vector reference.
    pub const QUERY_ENTITY_MAPPING_STALE_REFERENCE: &str =
        "graphloom.query.entity_mapping.stale_reference";
}

/// Stable `tracing` field names and their fixed wire types.
///
/// Field types are fixed by the contract:
///
/// | Field | Type |
/// | --- | --- |
/// | `OBSERVABILITY_VERSION` | `u64` |
/// | `RUN_ID` | string |
/// | `OPERATION` | string |
/// | `QUERY_METHOD` | string |
/// | `QUERY_STREAMING` | bool |
/// | `EXPLAINABILITY_ENABLED` | bool |
/// | `MODEL_INSTANCE` | string |
/// | `MODEL_PROVIDER` | string |
/// | `VECTOR_INDEX` | string |
/// | `RETRIEVAL_TOP_K` | `u64` |
/// | `INPUT_COUNT` | `u64` |
/// | `INPUT_TOKENS` | `u64` |
/// | `OUTPUT_TOKENS` | `u64` |
/// | `CONTEXT_TOKENS` | `u64` |
/// | `EMBEDDING_DIMENSIONS` | `u64` |
/// | `CANDIDATE_COUNT` | `u64` |
/// | `SELECTED_COUNT` | `u64` |
/// | `LLM_CALLS` | `u64` |
/// | `STATUS` | string |
/// | `ERROR_KIND` | string |
/// | `ELAPSED_MS` | `u64` |
///
/// Counters and durations are always recorded as `u64`; platform `usize`
/// widths are never exposed through the contract.
pub mod field_name {
    /// Observability contract version.
    pub const OBSERVABILITY_VERSION: &str = "graphloom.observability.version";
    /// Caller-provided Explainability run ID used for cross-channel correlation.
    pub const RUN_ID: &str = "graphloom.run.id";
    /// Stable operation name.
    pub const OPERATION: &str = "graphloom.operation";
    /// Query method (`local`, `global`, `drift`, `basic`).
    pub const QUERY_METHOD: &str = "graphloom.query.method";
    /// Whether the request consumes a streaming event stream.
    pub const QUERY_STREAMING: &str = "graphloom.query.streaming";
    /// Whether request Explainability is enabled.
    pub const EXPLAINABILITY_ENABLED: &str = "graphloom.explainability.enabled";
    /// Configured model instance identifier.
    pub const MODEL_INSTANCE: &str = "graphloom.model.instance";
    /// Configured model provider type.
    pub const MODEL_PROVIDER: &str = "graphloom.model.provider";
    /// Vector index name used by a search.
    pub const VECTOR_INDEX: &str = "graphloom.vector.index";
    /// Effective ANN `top_k` passed to the vector store.
    pub const RETRIEVAL_TOP_K: &str = "graphloom.retrieval.top_k";
    /// Number of input items sent to a provider.
    pub const INPUT_COUNT: &str = "graphloom.input.count";
    /// Input/prompt tokens.
    pub const INPUT_TOKENS: &str = "graphloom.input.tokens";
    /// Generated/output tokens.
    pub const OUTPUT_TOKENS: &str = "graphloom.output.tokens";
    /// Context tokens.
    pub const CONTEXT_TOKENS: &str = "graphloom.context.tokens";
    /// Embedding vector dimensions.
    pub const EMBEDDING_DIMENSIONS: &str = "graphloom.embedding.dimensions";
    /// Candidate count.
    pub const CANDIDATE_COUNT: &str = "graphloom.candidate.count";
    /// Selected count.
    pub const SELECTED_COUNT: &str = "graphloom.selected.count";
    /// Completion/embedding model calls.
    pub const LLM_CALLS: &str = "graphloom.llm.calls";
    /// Terminal span status.
    pub const STATUS: &str = "graphloom.status";
    /// Stable, low-cardinality error category.
    pub const ERROR_KIND: &str = "graphloom.error.kind";
    /// Wall-clock elapsed milliseconds.
    pub const ELAPSED_MS: &str = "graphloom.elapsed_ms";
}

/// Stable terminal status values.
pub mod status {
    /// The operation completed successfully.
    pub const OK: &str = "ok";
    /// The operation failed with a business or provider error.
    pub const ERROR: &str = "error";
    /// The operation was dropped before reaching a terminal state.
    pub const ABANDONED: &str = "abandoned";
}

/// Stable operation values.
///
/// Operation values are algorithm stages, never function names, model names,
/// or Query content.
pub mod operation {
    /// Whole Query request.
    pub const QUERY: &str = "query";
    /// Query runtime lookup and resource assembly.
    pub const RUNTIME_LOAD: &str = "runtime_load";
    /// Local context construction.
    pub const CONTEXT_BUILD: &str = "context_build";
    /// Entity mapping.
    pub const ENTITY_MAPPING: &str = "entity_mapping";
    /// Embedding provider request.
    pub const EMBEDDING: &str = "embedding";
    /// Vector store search.
    pub const VECTOR_SEARCH: &str = "vector_search";
    /// Graph expansion.
    pub const GRAPH_EXPANSION: &str = "graph_expansion";
    /// Prompt bind/render/validation.
    pub const PROMPT_RENDER: &str = "prompt_render";
    /// Completion provider request.
    pub const COMPLETION: &str = "completion";
}

/// Stable, low-cardinality error categories.
///
/// These values are shared by Explainability `RunFailed` records and the
/// `graphloom.error.kind` tracing field.
pub mod error_kind {
    /// Invalid Query configuration or request options.
    pub const INVALID_QUERY_CONFIG: &str = "invalid_query_config";
    /// A required Query table is missing.
    pub const MISSING_QUERY_TABLE: &str = "missing_query_table";
    /// A Query table has an incompatible field or value.
    pub const INVALID_QUERY_TABLE: &str = "invalid_query_table";
    /// A required vector index is missing.
    pub const MISSING_VECTOR_INDEX: &str = "missing_vector_index";
    /// A vector index or ANN result is invalid.
    pub const INVALID_VECTOR_INDEX: &str = "invalid_vector_index";
    /// Prompt loading, binding, or rendering failed.
    pub const QUERY_PROMPT: &str = "query_prompt";
    /// Query embedding failed.
    pub const QUERY_EMBEDDING: &str = "query_embedding";
    /// Query completion failed.
    pub const QUERY_COMPLETION: &str = "query_completion";
    /// Structured Query output parsing failed.
    pub const QUERY_PARSE: &str = "query_parse";
    /// Context construction failed.
    pub const QUERY_CONTEXT: &str = "query_context";
    /// Query runtime assembly failed.
    pub const QUERY_RUNTIME: &str = "query_runtime";
    /// Query method is unknown or unavailable.
    pub const QUERY_METHOD: &str = "query_method";
    /// Explainability record delivery failed.
    pub const EXPLAINABILITY_DELIVERY: &str = "explainability_delivery";
    /// Explainability event failed contract validation.
    pub const EVENT_CONTRACT: &str = "event_contract";
    /// Explainability sidecar data is incomplete.
    pub const EXPLAINABILITY_SIDECAR: &str = "explainability_sidecar";
    /// Explainability sink finalization failed.
    pub const EXPLAINABILITY_FINISH: &str = "explainability_finish";
    /// Explainability output creation/shutdown failed.
    pub const EXPLAINABILITY_OUTPUT: &str = "explainability_output";
    /// Entity mapping ignored a stale vector reference.
    pub const STALE_REFERENCE: &str = "stale_reference";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_pin_contract_version() {
        assert_eq!(OBSERVABILITY_CONTRACT_VERSION, 1);
    }

    #[test]
    fn test_should_keep_public_span_and_event_names_unique() {
        let mut names = Vec::new();
        names.extend([
            span_name::QUERY_LOCAL,
            span_name::QUERY_RUNTIME,
            span_name::QUERY_CONTEXT,
            span_name::QUERY_ENTITY_MAPPING,
            span_name::EMBEDDING_REQUEST,
            span_name::VECTOR_SEARCH,
            span_name::QUERY_GRAPH_EXPANSION,
            span_name::QUERY_PROMPT,
            span_name::LLM_REQUEST,
        ]);
        names.extend([
            event_name::CLI_QUERY_STARTED,
            event_name::CLI_QUERY_COMPLETED,
            event_name::CLI_QUERY_FAILED,
            event_name::CLI_EXPLAINABILITY_ENABLED,
            event_name::CLI_EXPLAINABILITY_SHUTDOWN_FAILED,
            event_name::QUERY_EXPLAINABILITY_DELIVERY_FAILED,
            event_name::QUERY_EXPLAINABILITY_CONTRACT_FAILED,
            event_name::QUERY_EXPLAINABILITY_SIDECAR_INCOMPLETE,
            event_name::QUERY_EXPLAINABILITY_FINISH_FAILED,
            event_name::QUERY_ENTITY_MAPPING_STALE_REFERENCE,
        ]);
        for name in &names {
            assert!(name.chars().filter(|ch| *ch == '.').count() >= 2);
        }
        let unique = names.iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), names.len());
    }
}
