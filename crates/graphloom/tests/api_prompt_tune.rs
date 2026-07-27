//! Public API integration tests for prompt tuning.
//!
//! These tests use ONLY `graphloom::api::*` — no crate-private internals.

use graphloom::api::{
    DocSelectionType, GenerateIndexingPromptsOptions, GeneratedIndexingPrompts,
    generate_indexing_prompts,
};

/// Verify the main API types and entry point are publicly accessible.
/// (Compile-time check — does not run the actual LLM pipeline.)
#[test]
fn test_api_entrypoint_is_public() {
    // Verify types are accessible from the public API
    let _options =
        GenerateIndexingPromptsOptions::new(".").with_selection_method(DocSelectionType::Top);

    // Verify GeneratedIndexingPrompts struct fields are accessible
    let _prompts = GeneratedIndexingPrompts {
        extract_graph: String::new(),
        summarize_descriptions: String::new(),
        community_report_graph: String::new(),
    };
}

/// Verify the async entry point can be called from an async context
/// (compile-time check — panics at runtime only if LLM isn't configured)
#[tokio::test]
async fn test_api_function_compiles_and_rejects_missing_config() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let options = GenerateIndexingPromptsOptions::new(tmp.path())
        .with_selection_method(DocSelectionType::Top)
        .with_limit(2)
        .with_domain("test")
        .with_language("English")
        .with_discover_entity_types(false)
        .with_cache(false);

    // Must fail with a clear error (not a compilation error)
    let result = generate_indexing_prompts(&options).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("settings") || err.contains("config"));
}

/// Verify that an external crate can construct options via the public API.
#[test]
fn test_can_construct_options_with_builder() {
    let _options = GenerateIndexingPromptsOptions::new(".")
        .with_selection_method(DocSelectionType::Top)
        .with_limit(10)
        .with_domain("software engineering")
        .with_language("English")
        .with_discover_entity_types(false)
        .with_min_examples_required(3)
        .with_max_tokens(1000)
        .with_chunk_size(500)
        .with_overlap(50)
        .with_cache(false);
}

/// Verify defaults match GraphRAG 3.1.0.
#[test]
fn test_defaults_match_graphrag_3_1() {
    let options = GenerateIndexingPromptsOptions::new(".");
    assert_eq!(options.limit, 15);
    assert_eq!(options.max_tokens, 2000);
    assert!(options.discover_entity_types);
    assert_eq!(options.min_examples_required, 2);
    assert_eq!(options.n_subset_max, 300);
    assert_eq!(options.k, 15);
    assert!(options.domain.is_none());
    assert!(options.language.is_none());
}

/// Verify DocSelectionType enum is accessible.
#[test]
fn test_doc_selection_type_public() {
    let methods = [
        DocSelectionType::Top,
        DocSelectionType::Random,
        DocSelectionType::Auto,
        DocSelectionType::All,
    ];
    assert_eq!(methods.len(), 4);
}
