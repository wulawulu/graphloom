//! Tests for prompt tuning templates and API.

use std::{num::NonZeroUsize, sync::Arc};

use graphloom_llm::{ChatMessage, CompletionModel, CompletionRequest, TiktokenTokenizer};

use super::{
    generator::{
        create_community_summarization_prompt, create_entity_summarization_prompt,
        create_extract_graph_prompt, detect_language, generate_community_report_rating,
        generate_community_reporter_role, generate_domain, generate_entity_relationship_examples,
        generate_persona,
    },
    test_support::{PromptTuneReplayModel, PromptTuneReplayRecord, RecordingModel},
};

const RAW_COMPLETION_CASES: [&str; 5] = ["", "   ", "\n\t", "  value  ", "value\n"];

#[test]
fn test_should_reject_zero_for_graphrag_positive_prompt_tune_options() {
    let cases = [
        (
            "limit",
            super::GenerateIndexingPromptsOptions::new(".").with_limit(0),
        ),
        (
            "min_examples_required",
            super::GenerateIndexingPromptsOptions::new(".").with_min_examples_required(0),
        ),
        (
            "n_subset_max",
            super::GenerateIndexingPromptsOptions::new(".").with_n_subset_max(0),
        ),
        (
            "k",
            super::GenerateIndexingPromptsOptions::new(".").with_k(0),
        ),
    ];

    for (field, options) in cases {
        let error = super::validate_options(&options).expect_err("zero must be rejected");
        assert!(
            matches!(
                &error,
                crate::GraphLoomError::InvalidData {
                    workflow: "prompt_tune",
                    ..
                }
            ),
            "unexpected error for {field}: {error}",
        );
        assert!(
            error.to_string().contains(field),
            "error must identify {field}: {error}",
        );
    }
}

#[derive(Clone, Copy, Debug)]
enum RawTextGenerator {
    Domain,
    Language,
    Persona,
    Rating,
    ReporterRole,
}

async fn run_raw_text_generator(
    generator: RawTextGenerator,
    model: &Arc<dyn CompletionModel>,
) -> crate::Result<String> {
    let docs = vec!["doc".to_owned()];

    match generator {
        RawTextGenerator::Domain => generate_domain(model, &docs, "test.raw-content").await,
        RawTextGenerator::Language => detect_language(model, &docs, "test.raw-content").await,
        RawTextGenerator::Persona => generate_persona(model, "domain", "test.raw-content").await,
        RawTextGenerator::Rating => {
            generate_community_report_rating(model, "domain", "persona", &docs, "test.raw-content")
                .await
        }
        RawTextGenerator::ReporterRole => {
            generate_community_reporter_role(model, "domain", "persona", &docs, "test.raw-content")
                .await
        }
    }
}

async fn assert_raw_completion_content_is_preserved(generator: RawTextGenerator) {
    for expected in RAW_COMPLETION_CASES {
        let recording_model = Arc::new(RecordingModel::new(vec![expected.to_owned()]));
        let model: Arc<dyn CompletionModel> = recording_model;
        let actual = run_raw_text_generator(generator, &model)
            .await
            .unwrap_or_else(|error| panic!("{generator:?} rejected {expected:?}: {error}"));

        assert_eq!(
            actual.as_bytes(),
            expected.as_bytes(),
            "{generator:?} changed completion content",
        );
    }
}

macro_rules! raw_text_generator_test {
    ($name:ident, $generator:ident) => {
        #[tokio::test]
        async fn $name() {
            assert_raw_completion_content_is_preserved(RawTextGenerator::$generator).await;
        }
    };
}

raw_text_generator_test!(domain_preserves_raw_completion_content, Domain);
raw_text_generator_test!(language_preserves_raw_completion_content, Language);
raw_text_generator_test!(persona_preserves_raw_completion_content, Persona);
raw_text_generator_test!(rating_preserves_raw_completion_content, Rating);
raw_text_generator_test!(reporter_role_preserves_raw_completion_content, ReporterRole);

#[tokio::test]
async fn relationship_examples_preserve_raw_content_and_order() {
    let docs = vec!["first doc".to_owned(), "second doc".to_owned()];
    let request = CompletionRequest::new(
        std::iter::once(ChatMessage::system("persona"))
            .chain(docs.iter().map(|doc| {
                let user_message =
                    include_str!("prompts/generate_entity_relationship_examples_untyped.txt")
                        .replace("{input_text}", doc)
                        .replace("{language}", "English");
                ChatMessage::user(user_message)
            }))
            .collect(),
    );

    for expected in ["", " \n\t "] {
        let records = docs
            .iter()
            .map(|_| PromptTuneReplayRecord::new(request.clone(), expected))
            .collect();
        let replay_model = Arc::new(PromptTuneReplayModel::new(records));
        let model: Arc<dyn CompletionModel> = replay_model.clone();

        let actual = generate_entity_relationship_examples(
            &model,
            "persona",
            None,
            &docs,
            "English",
            "test.raw-content",
        )
        .await
        .expect("relationship examples");

        assert_eq!(actual.len(), docs.len(), "one response per input document");
        for (index, actual) in actual.iter().enumerate() {
            assert_eq!(
                actual.as_bytes(),
                expected.as_bytes(),
                "relationship response {index} changed raw content",
            );
        }
        replay_model.assert_exhausted();
    }
}

#[tokio::test]
async fn prompt_tune_replay_rejects_unknown_requests() {
    let expected = CompletionRequest::new(vec![ChatMessage::user("known")]);
    let replay =
        PromptTuneReplayModel::new(vec![PromptTuneReplayRecord::new(expected, "response")]);

    let error = replay
        .complete(CompletionRequest::new(vec![ChatMessage::user("unknown")]))
        .await
        .expect_err("unknown request must fail");

    assert!(
        error
            .to_string()
            .contains("no unconsumed exact request match")
    );
}

#[tokio::test]
async fn prompt_tune_replay_rejects_ambiguous_duplicate_responses() {
    let request = CompletionRequest::new(vec![ChatMessage::user("same")]);
    let replay = PromptTuneReplayModel::new(vec![
        PromptTuneReplayRecord::new(request.clone(), "first"),
        PromptTuneReplayRecord::new(request.clone(), "second"),
    ]);

    let error = replay
        .complete(request)
        .await
        .expect_err("identical request with different responses must fail");

    assert!(
        error
            .to_string()
            .contains("identical requests map to different responses")
    );
}

#[tokio::test]
async fn prompt_tune_replay_consumes_declared_multiplicity_once() {
    let request = CompletionRequest::new(vec![
        ChatMessage::system("persona"),
        ChatMessage::user("shared request"),
    ]);
    let replay = PromptTuneReplayModel::new(vec![
        PromptTuneReplayRecord::new(request.clone(), "same response"),
        PromptTuneReplayRecord::new(request.clone(), "same response"),
    ]);

    for _ in 0..2 {
        let response = replay
            .complete(request.clone())
            .await
            .expect("declared request occurrence");
        assert_eq!(
            response.content().expect("completion content"),
            "same response"
        );
    }
    let error = replay
        .complete(request)
        .await
        .expect_err("third request exceeds declared multiplicity");
    assert!(
        error
            .to_string()
            .contains("no unconsumed exact request match")
    );
    replay.assert_exhausted();
}

/// Verify a generated GraphRAG-format template's byte-level markers.
fn assert_graphrag_template(
    template: &str,
    name: &str,
    expected_contains: &[&str],
    forbidden_contains: &[&str],
) {
    for pattern in expected_contains {
        assert!(
            template.contains(pattern),
            "{name}: should contain {pattern}"
        );
    }
    for pattern in forbidden_contains {
        assert!(
            !template.contains(pattern),
            "{name}: should not contain {pattern}"
        );
    }
}

#[test]
fn prompt_tune_request_assets_use_lf_line_endings() {
    let assets = [
        (
            "entity_types_json_request",
            include_str!("fixtures/entity_types_json_request.txt"),
        ),
        (
            "generate_entity_types_json",
            include_str!("prompts/generate_entity_types_json.txt"),
        ),
        ("extract_graph", include_str!("templates/extract_graph.txt")),
        (
            "extract_graph_untyped",
            include_str!("templates/extract_graph_untyped.txt"),
        ),
    ];

    for (name, content) in assets {
        assert!(
            !content.as_bytes().contains(&b'\r'),
            "{name} must use LF-only line endings",
        );
    }
}

/// Verify the generated extract_graph template uses GraphRAG format variables.
#[test]
fn test_extract_graph_template_has_tera_input_text() {
    let docs = vec!["Doc one content.".to_owned(), "Doc two content.".to_owned()];
    let examples = vec!["Example output".to_owned()];
    let entity_types = vec!["person".to_owned(), "organization".to_owned()];
    let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");

    let prompt = create_extract_graph_prompt(
        Some(&entity_types),
        &docs,
        &examples,
        "English",
        2000,
        &tokenizer,
        2,
    )
    .expect("extract graph");

    assert_graphrag_template(
        &prompt,
        "extract_graph.txt",
        &["{input_text}", "person", "organization"],
        &["{{input_text}}"],
    );
}

/// Verify the generated untyped extract_graph template uses GraphRAG syntax.
#[test]
fn test_untyped_extract_graph_template_is_valid_tera() {
    let docs = vec!["Doc one content.".to_owned()];
    let examples = vec!["Example output".to_owned()];
    let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");

    let prompt =
        create_extract_graph_prompt(None, &docs, &examples, "English", 2000, &tokenizer, 2)
            .expect("extract graph");

    assert_graphrag_template(
        &prompt,
        "extract_graph.txt",
        &["{input_text}"],
        &["{{input_text}}"],
    );
}

/// Verify the generated summarize_descriptions template uses GraphRAG syntax.
#[test]
fn test_summarize_descriptions_template_is_valid_tera() {
    let prompt =
        create_entity_summarization_prompt("You are an expert in data analysis.", "English");

    assert_graphrag_template(
        &prompt,
        "summarize_descriptions.txt",
        &["{entity_name}", "{description_list}"],
        &["{{entity_name}}", "{{description_list}}"],
    );
}

/// Verify the generated community_report_graph template uses GraphRAG syntax.
#[test]
fn test_community_report_template_is_valid_tera() {
    let prompt = create_community_summarization_prompt(
        "You are an expert analyst.",
        "A community analyst role",
        "Rating description text",
        "English",
    );

    assert_graphrag_template(
        &prompt,
        "community_report_graph.txt",
        &["{input_text}", "{{", "\"title\"", "\"summary\""],
        &["{{input_text}}"],
    );
}

/// Verify token budget: min_examples_required examples always included.
#[test]
fn test_token_budget_respects_min_examples() {
    let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");
    // Create very long docs that will quickly exhaust the budget
    let long_text = "token ".repeat(100);
    let docs: Vec<String> = (0..10).map(|_| long_text.clone()).collect();
    let examples: Vec<String> = (0..10).map(|i| format!("output {i}")).collect();
    let entity_types = vec!["person".to_owned(), "organization".to_owned()];

    // With max_tokens=1, budget is always exhausted after base prompt
    let prompt = create_extract_graph_prompt(
        Some(&entity_types),
        &docs,
        &examples,
        "English",
        1,
        &tokenizer,
        2,
    )
    .expect("extract graph");

    // Should still contain at least the first 2 examples
    assert!(prompt.contains("Example 1:"));
    assert!(prompt.contains("Example 2:"));
    // But Example 3 should be excluded by budget
    assert!(!prompt.contains("Example 3:"));
}

// ---- GraphRAG Python-format literal contract ----

#[test]
fn test_extract_graph_preserves_graphrag_escaped_document_braces() {
    let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");
    let docs = vec![super::selection::escape_python_format_literal(
        r#"JSON {"name":"Alice"} and \frac{a}{b}"#,
    )];
    let examples = vec!["output".to_owned()];
    let entity_types = vec!["person".to_owned()];

    let prompt = create_extract_graph_prompt(
        Some(&entity_types),
        &docs,
        &examples,
        "English",
        2000,
        &tokenizer,
        2,
    )
    .expect("prompt");

    assert!(prompt.contains(r#"JSON {{"name":"Alice"}}"#));
    assert!(prompt.contains(r#"\frac{{a}}{{b}}"#));
    assert!(prompt.contains("{input_text}"));
    assert!(!prompt.contains("{{input_text}}"));
}

#[test]
fn test_extract_graph_preserves_raw_relationship_response_bytes() {
    let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");
    let docs = vec!["simple doc".to_owned()];
    let examples = vec!["  response {{with}} braces  \n".to_owned()];

    let prompt =
        create_extract_graph_prompt(None, &docs, &examples, "English", 2000, &tokenizer, 1)
            .expect("prompt");

    assert!(prompt.contains("  response {{with}} braces  \n"));
}

/// Verify chunk_size and overlap overrides use correct types.
#[test]
fn test_chunk_size_override_uses_nonzero() {
    assert!(NonZeroUsize::new(500).is_some());
    assert!(NonZeroUsize::new(0).is_none());
}

// ---- real PromptRepository load/bind/render tests ----

use tempfile::TempDir;

use crate::prompts::{PromptKind, PromptRepository, PromptSource};

async fn assert_prompt_loads_and_renders(
    name: &str,
    prompt_text: &str,
    kind: PromptKind,
    context: serde_json::Value,
    expected_after_render: &[&str],
) {
    let tmp = TempDir::new().expect("temp dir");
    let prompts_dir = tmp.path().join("prompts");
    tokio::fs::create_dir(&prompts_dir)
        .await
        .expect("prompts dir");
    let prompt_path = prompts_dir.join(kind.filename());
    tokio::fs::write(&prompt_path, prompt_text)
        .await
        .expect("write prompt");

    let repo = PromptRepository::new(tmp.path().to_path_buf());
    let template = repo
        .load(kind, Some(&prompt_path))
        .await
        .unwrap_or_else(|e| panic!("{name}: PromptRepository load failed: {e}"));

    // Verify it came from the explicit path
    assert!(matches!(template.source(), PromptSource::Explicit(_)));

    // Bind and render
    let rendered = template
        .bind(&context)
        .unwrap_or_else(|e| panic!("{name}: bind failed: {e}"))
        .render()
        .unwrap_or_else(|e| panic!("{name}: render failed: {e}"));

    for expected in expected_after_render {
        assert!(
            rendered.contains(expected),
            "{name}: render output should contain {expected:?}, got:\n{rendered}"
        );
    }
}

#[tokio::test]
async fn test_extract_graph_loads_binds_renders() {
    let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");
    let docs = vec![super::selection::escape_python_format_literal(
        "Doc with {{ user_var }} and {% tag %} and {# comment #}",
    )];
    let examples = vec!["example {{ ex }}".to_owned()];
    let entity_types = vec!["person".to_owned(), "type_{{x}}".to_owned()];

    let prompt = create_extract_graph_prompt(
        Some(&entity_types),
        &docs,
        &examples,
        "English",
        2000,
        &tokenizer,
        2,
    )
    .expect("prompt");

    assert_prompt_loads_and_renders(
        "extract_graph",
        &prompt,
        PromptKind::ExtractGraph,
        serde_json::json!({"input_text": "Alice met Bob.", "entity_types": []}),
        &[
            "Alice met Bob.",
            "{{ user_var }}",
            "{% tag %}",
            "{# comment #}",
            "{ ex }",
            "type_{x}",
        ],
    )
    .await;
}

#[tokio::test]
async fn test_summarize_descriptions_loads_binds_renders() {
    let prompt = create_entity_summarization_prompt(
        "Expert with {{ skill }} and {{% domain %}} expertise.",
        "English {{ lang_var }}",
    );

    assert_prompt_loads_and_renders(
        "summarize_descriptions",
        &prompt,
        PromptKind::SummarizeDescriptions,
        serde_json::json!({
            "entity_name": "Alice",
            "description_list": "[\"Engineer\"]",
            "max_length": 500,
        }),
        &[
            "Alice",
            "Engineer",
            "{ skill }",
            "{% domain %}",
            "{ lang_var }",
        ],
    )
    .await;
}

#[tokio::test]
async fn test_community_report_loads_binds_renders() {
    let prompt = create_community_summarization_prompt(
        "Analyst with {{ template }} skill.",
        "role with {{% syntax %}}",
        "rating {{ desc }}",
        "English {{ lang }}",
    );

    assert_prompt_loads_and_renders(
        "community_report",
        &prompt,
        PromptKind::CommunityReportGraph,
        serde_json::json!({
            "input_text": "Community data here.",
            "max_report_length": 2000,
        }),
        &[
            "Community data here.",
            "{ template }",
            "{% syntax %}",
            "{ desc }",
            "{ lang }",
        ],
    )
    .await;
}

// ---------------------------------------------------------------------------
// Untyped fallback: empty discovered types → untyped extract_graph path
// ---------------------------------------------------------------------------

/// Verify that an empty discovered entity type list selects the untyped path.
#[test]
fn empty_discovered_types_select_none() {
    let result = super::normalize_discovered_entity_types(Vec::new());
    assert_eq!(result, None);
}

/// Verify that non-empty discovered entity types are preserved.
#[test]
fn non_empty_discovered_types_select_typed() {
    let types = vec!["person".to_owned()];
    let result = super::normalize_discovered_entity_types(types.clone());
    assert_eq!(result, Some(types));
}

// ---- template-level typed/untyped selection -------------------------------

/// None selects the untyped extract-graph template.
///
/// Distinction: untyped template contains "Suggest several labels or categories"
/// and does NOT mention entity_types.
#[test]
fn none_selects_untyped_extract_graph_template() {
    let docs = vec!["doc".to_owned()];
    let examples = vec!["output".to_owned()];
    let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");

    let prompt =
        create_extract_graph_prompt(None, &docs, &examples, "English", 2000, &tokenizer, 2)
            .expect("prompt");

    // Untyped-only text
    assert!(
        prompt.contains("Suggest several labels or categories for the entity."),
        "untyped template has its own entity description text"
    );
    assert!(
        !prompt.contains("entity_types: ["),
        "untyped template has no entity_types bracket"
    );
}

/// Some types select the typed extract-graph template.
///
/// Distinction: typed template contains "One of the following types" and
/// includes the entity_types inline.
#[test]
fn some_types_select_typed_extract_graph_template() {
    let docs = vec!["doc".to_owned()];
    let examples = vec!["output".to_owned()];
    let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");

    let types = vec!["person".to_owned(), "organization".to_owned()];

    let prompt = create_extract_graph_prompt(
        Some(&types),
        &docs,
        &examples,
        "English",
        2000,
        &tokenizer,
        2,
    )
    .expect("prompt");

    // Typed-only text
    assert!(
        prompt.contains("One of the following types:"),
        "typed template has entity type constraint text"
    );
    assert!(
        prompt.contains("entity_types: [person, organization]"),
        "typed template includes entity_types"
    );
}

// ---- real-chain tests: typed/untyped through relationship generator --------

/// Empty discovered types → None → untyped relationship request → untyped extract graph.
///
/// This test actually calls `generate_entity_relationship_examples` and
/// `create_extract_graph_prompt` in sequence, proving the full path from
/// empty entity types to untyped template assembly.
#[tokio::test]
async fn empty_discovered_types_drive_untyped_relationship_and_extract_graph() {
    let normalized = super::normalize_discovered_entity_types(Vec::new());
    assert_eq!(normalized, None);

    let model = Arc::new(RecordingModel::new(vec![
        r#"("entity"<|>ALICE<|>person<|>Alice)##<|COMPLETE|>"#.to_owned(),
    ]));

    let docs = vec!["doc".to_owned()];

    // Step 1: relationship generation uses untyped prompt
    let examples = generate_entity_relationship_examples(
        &(model.clone() as Arc<dyn graphloom_llm::CompletionModel>),
        "Expert.",
        normalized.as_deref(),
        &docs,
        "English",
        "test.chain",
    )
    .await
    .expect("relationship generation");

    // Step 2: extract graph uses untyped template
    let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");
    let prompt = create_extract_graph_prompt(
        normalized.as_deref(),
        &docs,
        &examples,
        "English",
        2000,
        &tokenizer,
        1,
    )
    .expect("extract graph prompt");

    // Untyped-specific evidence
    assert!(
        prompt.contains("Suggest several labels or categories for the entity."),
        "untyped template selected"
    );
    assert!(
        !prompt.contains("entity_types: ["),
        "untyped template has no entity_types bracket"
    );

    // Verify the relationship request was recorded
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "one relationship request");
    assert_eq!(requests[0].response_format, None);
    assert_eq!(requests[0].messages.len(), 2);
    assert_eq!(
        requests[0].messages[0].role,
        graphloom_llm::ChatRole::System
    );
    assert_eq!(requests[0].messages[1].role, graphloom_llm::ChatRole::User);
    // Untyped relationship prompt does NOT contain entity_types string
    assert!(
        !requests[0].messages[1]
            .content
            .as_str()
            .contains("entity_types:"),
        "untyped relationship request has no entity_types field"
    );
}

/// Non-empty types → Some → typed relationship request → typed extract graph.
///
/// This test actually calls `generate_entity_relationship_examples` and
/// `create_extract_graph_prompt` in sequence, proving the full path from
/// discovered entity types to typed template assembly.
#[tokio::test]
async fn non_empty_discovered_types_drive_typed_relationship_and_extract_graph() {
    let types = vec!["person".to_owned(), "organization".to_owned()];
    let normalized = super::normalize_discovered_entity_types(types);
    assert!(normalized.is_some());

    let model = Arc::new(RecordingModel::new(vec![
        r#"("entity"<|>ALICE<|>person<|>Alice)##<|COMPLETE|>"#.to_owned(),
    ]));

    let docs = vec!["doc".to_owned()];

    // Step 1: relationship generation uses typed prompt
    let examples = generate_entity_relationship_examples(
        &(model.clone() as Arc<dyn graphloom_llm::CompletionModel>),
        "Expert.",
        normalized.as_deref(),
        &docs,
        "English",
        "test.chain",
    )
    .await
    .expect("relationship generation");

    // Step 2: extract graph uses typed template
    let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");
    let prompt = create_extract_graph_prompt(
        normalized.as_deref(),
        &docs,
        &examples,
        "English",
        2000,
        &tokenizer,
        1,
    )
    .expect("extract graph prompt");

    // Typed-specific evidence
    assert!(
        prompt.contains("One of the following types:"),
        "typed template selected"
    );
    assert!(
        prompt.contains("entity_types: [person, organization]"),
        "typed template includes entity_types"
    );

    // Verify the relationship request was recorded
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "one relationship request");
    assert_eq!(requests[0].response_format, None);
    assert_eq!(requests[0].messages.len(), 2);
    // Typed relationship prompt contains entity_types string
    assert!(
        requests[0].messages[1]
            .content
            .as_str()
            .contains("entity_types: person, organization"),
        "typed relationship request contains entity_types"
    );
}
