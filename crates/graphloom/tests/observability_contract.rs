use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

use graphloom::observability::{
    OBSERVABILITY_CONTRACT_VERSION, error_kind, event_name, field_name, operation, span_name,
    status,
};
mod support;

use support::capture::{CaptureState, capture_subscriber};

#[test]
fn test_should_pin_contract_version_to_one() {
    assert_eq!(OBSERVABILITY_CONTRACT_VERSION, 1);
}

#[test]
fn test_should_expose_exact_span_names() {
    assert_eq!(span_name::QUERY_LOCAL, "graphloom.query.local");
    assert_eq!(span_name::QUERY_RUNTIME, "graphloom.query.runtime");
    assert_eq!(span_name::QUERY_CONTEXT, "graphloom.query.context");
    assert_eq!(
        span_name::QUERY_ENTITY_MAPPING,
        "graphloom.query.entity_mapping"
    );
    assert_eq!(span_name::EMBEDDING_REQUEST, "graphloom.embedding.request");
    assert_eq!(span_name::VECTOR_SEARCH, "graphloom.vector.search");
    assert_eq!(
        span_name::QUERY_GRAPH_EXPANSION,
        "graphloom.query.graph_expansion"
    );
    assert_eq!(span_name::QUERY_PROMPT, "graphloom.query.prompt");
    assert_eq!(span_name::LLM_REQUEST, "graphloom.llm.request");
}

#[test]
fn test_should_expose_exact_event_names() {
    assert_eq!(event_name::CLI_QUERY_STARTED, "graphloom.cli.query.started");
    assert_eq!(
        event_name::CLI_QUERY_COMPLETED,
        "graphloom.cli.query.completed"
    );
    assert_eq!(event_name::CLI_QUERY_FAILED, "graphloom.cli.query.failed");
    assert_eq!(
        event_name::CLI_EXPLAINABILITY_ENABLED,
        "graphloom.cli.explainability.enabled"
    );
    assert_eq!(
        event_name::CLI_EXPLAINABILITY_SHUTDOWN_FAILED,
        "graphloom.cli.explainability.shutdown_failed"
    );
    assert_eq!(
        event_name::QUERY_EXPLAINABILITY_DELIVERY_FAILED,
        "graphloom.query.explainability.delivery_failed"
    );
    assert_eq!(
        event_name::QUERY_EXPLAINABILITY_CONTRACT_FAILED,
        "graphloom.query.explainability.contract_failed"
    );
    assert_eq!(
        event_name::QUERY_EXPLAINABILITY_SIDECAR_INCOMPLETE,
        "graphloom.query.explainability.sidecar_incomplete"
    );
    assert_eq!(
        event_name::QUERY_EXPLAINABILITY_FINISH_FAILED,
        "graphloom.query.explainability.finish_failed"
    );
    assert_eq!(
        event_name::QUERY_ENTITY_MAPPING_STALE_REFERENCE,
        "graphloom.query.entity_mapping.stale_reference"
    );
}

#[test]
fn test_should_expose_exact_field_names() {
    assert_eq!(
        field_name::OBSERVABILITY_VERSION,
        "graphloom.observability.version"
    );
    assert_eq!(field_name::RUN_ID, "graphloom.run.id");
    assert_eq!(field_name::OPERATION, "graphloom.operation");
    assert_eq!(field_name::QUERY_METHOD, "graphloom.query.method");
    assert_eq!(field_name::QUERY_STREAMING, "graphloom.query.streaming");
    assert_eq!(
        field_name::EXPLAINABILITY_ENABLED,
        "graphloom.explainability.enabled"
    );
    assert_eq!(field_name::MODEL_INSTANCE, "graphloom.model.instance");
    assert_eq!(field_name::MODEL_PROVIDER, "graphloom.model.provider");
    assert_eq!(field_name::VECTOR_INDEX, "graphloom.vector.index");
    assert_eq!(field_name::RETRIEVAL_TOP_K, "graphloom.retrieval.top_k");
    assert_eq!(field_name::INPUT_COUNT, "graphloom.input.count");
    assert_eq!(field_name::INPUT_TOKENS, "graphloom.input.tokens");
    assert_eq!(field_name::OUTPUT_TOKENS, "graphloom.output.tokens");
    assert_eq!(field_name::CONTEXT_TOKENS, "graphloom.context.tokens");
    assert_eq!(
        field_name::EMBEDDING_DIMENSIONS,
        "graphloom.embedding.dimensions"
    );
    assert_eq!(field_name::CANDIDATE_COUNT, "graphloom.candidate.count");
    assert_eq!(field_name::SELECTED_COUNT, "graphloom.selected.count");
    assert_eq!(field_name::LLM_CALLS, "graphloom.llm.calls");
    assert_eq!(field_name::STATUS, "graphloom.status");
    assert_eq!(field_name::ERROR_KIND, "graphloom.error.kind");
    assert_eq!(field_name::ELAPSED_MS, "graphloom.elapsed_ms");
}

#[test]
fn test_should_expose_exact_status_and_operation_values() {
    assert_eq!(status::OK, "ok");
    assert_eq!(status::ERROR, "error");
    assert_eq!(status::ABANDONED, "abandoned");

    assert_eq!(operation::QUERY, "query");
    assert_eq!(operation::RUNTIME_LOAD, "runtime_load");
    assert_eq!(operation::CONTEXT_BUILD, "context_build");
    assert_eq!(operation::ENTITY_MAPPING, "entity_mapping");
    assert_eq!(operation::EMBEDDING, "embedding");
    assert_eq!(operation::VECTOR_SEARCH, "vector_search");
    assert_eq!(operation::GRAPH_EXPANSION, "graph_expansion");
    assert_eq!(operation::PROMPT_RENDER, "prompt_render");
    assert_eq!(operation::COMPLETION, "completion");
}

#[test]
fn test_should_expose_stable_error_kinds() {
    for kind in [
        error_kind::INVALID_QUERY_CONFIG,
        error_kind::MISSING_QUERY_TABLE,
        error_kind::INVALID_QUERY_TABLE,
        error_kind::MISSING_VECTOR_INDEX,
        error_kind::INVALID_VECTOR_INDEX,
        error_kind::QUERY_PROMPT,
        error_kind::QUERY_EMBEDDING,
        error_kind::QUERY_COMPLETION,
        error_kind::QUERY_PARSE,
        error_kind::QUERY_CONTEXT,
        error_kind::QUERY_RUNTIME,
        error_kind::QUERY_METHOD,
        error_kind::EXPLAINABILITY_DELIVERY,
        error_kind::EVENT_CONTRACT,
        error_kind::EXPLAINABILITY_FINISH,
        error_kind::EXPLAINABILITY_OUTPUT,
        error_kind::STALE_REFERENCE,
    ] {
        assert!(!kind.is_empty());
    }
}

#[test]
fn test_should_keep_span_and_event_names_unique_and_low_cardinality() {
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
    let unique = names.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(unique.len(), names.len(), "span/event names must be unique");
    for name in names {
        assert!(
            name.chars().filter(|ch| *ch == '.').count() >= 2,
            "span/event name {name} must be dot-separated and low cardinality"
        );
        assert!(!name.contains(' '));
        assert_eq!(name, name.to_ascii_lowercase());
    }
}

#[test]
fn test_should_prefix_every_field_name_with_graphloom() {
    let fields = [
        field_name::OBSERVABILITY_VERSION,
        field_name::RUN_ID,
        field_name::OPERATION,
        field_name::QUERY_METHOD,
        field_name::QUERY_STREAMING,
        field_name::EXPLAINABILITY_ENABLED,
        field_name::MODEL_INSTANCE,
        field_name::MODEL_PROVIDER,
        field_name::VECTOR_INDEX,
        field_name::RETRIEVAL_TOP_K,
        field_name::INPUT_COUNT,
        field_name::INPUT_TOKENS,
        field_name::OUTPUT_TOKENS,
        field_name::CONTEXT_TOKENS,
        field_name::EMBEDDING_DIMENSIONS,
        field_name::CANDIDATE_COUNT,
        field_name::SELECTED_COUNT,
        field_name::LLM_CALLS,
        field_name::STATUS,
        field_name::ERROR_KIND,
        field_name::ELAPSED_MS,
    ];
    for field in fields {
        assert!(
            field.starts_with("graphloom."),
            "field {field} must start with graphloom."
        );
        assert!(!field.contains(' '));
    }
}

#[test]
fn test_should_capture_named_events_and_span_parents_structurally() {
    let state = Arc::new(Mutex::new(CaptureState::default()));
    let subscriber = capture_subscriber(state.clone());
    let _guard = tracing::subscriber::set_default(subscriber);

    let parent = tracing::info_span!(
        "graphloom.contract.parent",
        "graphloom.run.id" = "run-1",
        "graphloom.status" = tracing::field::Empty,
    );
    let child = tracing::info_span!(parent: &parent, "graphloom.contract.child");
    let _child_enter = child.enter();
    tracing::info!(
        name: "graphloom.contract.event",
        {
            "graphloom.status" = "ok",
            "graphloom.elapsed_ms" = 7_u64,
        },
        "contract event"
    );
    parent.record("graphloom.status", "ok");
    drop(_child_enter);
    drop(child);
    drop(parent);

    let state = state.lock().expect("state");
    assert_eq!(state.events.len(), 1);
    assert_eq!(state.events[0].name, "graphloom.contract.event");
    assert_eq!(state.events[0].field("graphloom.status"), Some("\"ok\""));
    assert_eq!(state.events[0].field("graphloom.elapsed_ms"), Some("7"));

    assert_eq!(state.spans.len(), 2);
    let parent_span = &state.spans[0];
    let child_span = &state.spans[1];
    assert_eq!(parent_span.name, "graphloom.contract.parent");
    assert_eq!(child_span.name, "graphloom.contract.child");
    assert_eq!(child_span.parent.as_ref(), Some(&parent_span.id));
    assert!(parent_span.closed);
    assert!(child_span.closed);
    assert_eq!(parent_span.field("graphloom.run.id"), Some("\"run-1\""));
    assert_eq!(parent_span.field("graphloom.status"), Some("\"ok\""));
}
