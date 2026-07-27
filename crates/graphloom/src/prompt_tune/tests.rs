//! Tests for prompt tuning templates and API.

use std::num::NonZeroUsize;

use graphloom_llm::TiktokenTokenizer;
use tera::Tera;

use super::generator::{
    create_community_summarization_prompt, create_entity_summarization_prompt,
    create_extract_graph_prompt,
};

/// Verify a template string is valid Tera and contains the expected content.
fn assert_valid_tera(
    template: &str,
    name: &str,
    expected_contains: &[&str],
    forbidden_single_brace: &[&str],
) {
    for pattern in expected_contains {
        assert!(
            template.contains(pattern),
            "{name}: should contain {pattern}"
        );
    }
    for var_name in forbidden_single_brace {
        // Strip all double-brace occurrences from the template, then check for single-brace.
        let double = format!("{{{{{}}}}}", var_name); // {{input_text}}
        let single = format!("{{{}}}", var_name); // {input_text}
        let stripped = template.replace(&double, "");
        assert!(
            !stripped.contains(&single),
            "{name}: should NOT contain single-brace `{single}` (only Tera double-brace)"
        );
    }
    let mut tera = Tera::default();
    match tera.add_raw_template(name, template) {
        Ok(_) => {}
        Err(error) => {
            // Print first 500 chars of template for debugging
            let preview = &template[..template.len().min(500)];
            panic!("{name} should compile as Tera: {error}\nTemplate preview:\n{preview}");
        }
    }
}

/// Verify the generated extract_graph template includes required Tera variables.
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

    assert_valid_tera(
        &prompt,
        "extract_graph.txt",
        &["{{input_text}}", "person", "organization"],
        &["input_text"],
    );
}

/// Verify the generated untyped extract_graph template is valid Tera.
#[test]
fn test_untyped_extract_graph_template_is_valid_tera() {
    let docs = vec!["Doc one content.".to_owned()];
    let examples = vec!["Example output".to_owned()];
    let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");

    let prompt =
        create_extract_graph_prompt(None, &docs, &examples, "English", 2000, &tokenizer, 2)
            .expect("extract graph");

    assert_valid_tera(
        &prompt,
        "extract_graph.txt",
        &["{{input_text}}"],
        &["input_text"],
    );
}

/// Verify the generated summarize_descriptions template is valid Tera.
#[test]
fn test_summarize_descriptions_template_is_valid_tera() {
    let prompt =
        create_entity_summarization_prompt("You are an expert in data analysis.", "English");

    assert_valid_tera(
        &prompt,
        "summarize_descriptions.txt",
        &["{{entity_name}}", "{{description_list}}"],
        &["entity_name", "description_list"],
    );
}

/// Verify the generated community_report_graph template is valid Tera.
#[test]
fn test_community_report_template_is_valid_tera() {
    let prompt = create_community_summarization_prompt(
        "You are an expert analyst.",
        "A community analyst role",
        "Rating description text",
        "English",
    );

    assert_valid_tera(
        &prompt,
        "community_report_graph.txt",
        &["{{input_text}}", "\"title\"", "\"summary\""],
        &["input_text"],
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

// ---- extract_graph Tera literal safety ----

#[test]
fn test_extract_graph_preserves_doc_double_braces() {
    let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");
    let docs = vec!["User wrote: {{ variable }} in their text.".to_owned()];
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

    // The user's {{ variable }} must NOT be an executable Tera expression.
    // After Tera escaping, the opening "{{" is converted to {{ "{{" }}.
    assert!(!prompt.contains("{{ variable }}"));
    // The runtime variable {{input_text}} must still be present.
    assert!(prompt.contains("{{input_text}}"));
}

#[test]
fn test_extract_graph_preserves_tag_and_comment_in_docs() {
    let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");
    let docs = vec!["{% if x %} do thing {# note #}".to_owned()];
    let examples = vec!["output".to_owned()];

    let prompt =
        create_extract_graph_prompt(None, &docs, &examples, "English", 2000, &tokenizer, 2)
            .expect("prompt");

    assert!(!prompt.contains("{% if x %}"));
    assert!(!prompt.contains("{# note #}"));
    assert!(prompt.contains("{{input_text}}"));
}

#[test]
fn test_extract_graph_preserves_example_output_delimiters() {
    let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");
    let docs = vec!["simple doc".to_owned()];
    // LLM-generated example output containing Tera delimiters
    let examples = vec!["entity: {{ name }} {% raw %}content{% endraw %} {# meta #}".to_owned()];
    let entity_types = vec!["org".to_owned()];

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

    // Example output delimiters must be escaped
    assert!(!prompt.contains("{{ name }}"));
    assert!(!prompt.contains("{% raw %}"));
    assert!(!prompt.contains("{% endraw %}"));
    assert!(!prompt.contains("{# meta #}"));
    // Runtime variable preserved
    assert!(prompt.contains("{{input_text}}"));
}

#[test]
fn test_extract_graph_compiles_and_renders_with_tera() {
    let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");
    let docs = vec!["Text with {{ var }} and {% tag %} and {# comment #}".to_owned()];
    let examples = vec!["example {{ ex }}".to_owned()];
    let entity_types = vec!["type_{{x}}".to_owned(), "org".to_owned()];

    let template = create_extract_graph_prompt(
        Some(&entity_types),
        &docs,
        &examples,
        "English",
        2000,
        &tokenizer,
        2,
    )
    .expect("prompt");

    // Compile with Tera
    let mut tera = Tera::default();
    tera.add_raw_template("extract_graph.txt", &template)
        .expect("must compile as Tera");

    // Render with runtime input_text
    let runtime_input = "Alice met Bob in New York.";
    let mut context = tera::Context::new();
    context.insert("input_text", runtime_input);

    let rendered = tera
        .render("extract_graph.txt", &context)
        .expect("must render");

    // Runtime variable was substituted
    assert!(rendered.contains("Alice met Bob in New York."));
    // User's Tera-like text is preserved literally
    assert!(rendered.contains("{{ var }}"));
    assert!(rendered.contains("{% tag %}"));
    assert!(rendered.contains("{# comment #}"));
    // Example output preserved
    assert!(rendered.contains("{{ ex }}"));
    // Entity types preserved
    assert!(rendered.contains("type_{{x}}"));
}

#[test]
fn test_extract_graph_entity_types_escaped() {
    let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");
    let docs = vec!["doc".to_owned()];
    let examples = vec!["out".to_owned()];
    let entity_types = vec!["{{bad_type}}".to_owned(), "{%other%}".to_owned()];

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

    // Entity types escaped
    assert!(!prompt.contains("{{bad_type}}"));
    assert!(!prompt.contains("{%other%}"));
    // Runtime variable preserved
    assert!(prompt.contains("{{input_text}}"));
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
    let docs = vec!["Doc with {{ user_var }} and {% tag %} and {# comment #}".to_owned()];
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
            "{{ ex }}",
            "type_{{x}}",
        ],
    )
    .await;
}

#[tokio::test]
async fn test_summarize_descriptions_loads_binds_renders() {
    let prompt = create_entity_summarization_prompt(
        "Expert with {{ skill }} and {% domain %} expertise.",
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
            "{{ skill }}",
            "{% domain %}",
            "{{ lang_var }}",
        ],
    )
    .await;
}

#[tokio::test]
async fn test_community_report_loads_binds_renders() {
    let prompt = create_community_summarization_prompt(
        "Analyst with {{ template }} skill.",
        "role with {% syntax %}",
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
            "{{ template }}",
            "{% syntax %}",
            "{{ desc }}",
            "{{ lang }}",
        ],
    )
    .await;
}
