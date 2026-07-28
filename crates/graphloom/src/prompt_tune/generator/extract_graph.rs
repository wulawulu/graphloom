//! Entity extraction prompt assembly for GraphRAG prompt tuning.

use graphloom_llm::Tokenizer;

use super::without_storage_lf;
use crate::{GraphLoomError, Result};

/// Canonical output filename.
#[allow(dead_code, reason = "used by CLI and public API")]
pub(crate) const EXTRACT_GRAPH_FILENAME: &str = "extract_graph.txt";

// ---- base templates -------------------------------------------------------

const GRAPH_EXTRACTION_PROMPT_SOURCE: &str = include_str!("../templates/extract_graph.txt");

const UNTYPED_GRAPH_EXTRACTION_PROMPT: &str =
    include_str!("../templates/extract_graph_untyped.txt");

const EXAMPLE_EXTRACTION_TEMPLATE: &str = include_str!("../templates/extract_graph_example.txt");

const UNTYPED_EXAMPLE_EXTRACTION_TEMPLATE: &str =
    include_str!("../templates/extract_graph_untyped_example.txt");

fn counted(tokenizer: &dyn Tokenizer, text: &str) -> Result<usize> {
    tokenizer.count(text).map_err(GraphLoomError::Llm)
}

/// Create an entity extraction prompt from the tuned components.
///
/// # GraphRAG 3.1.0 token budget reference
///
/// GraphLoom uses logical unescaped text for token counting.
///
/// ```python
/// prompt = GRAPH_EXTRACTION_PROMPT  # raw, BEFORE .format()
/// if isinstance(entity_types, list):
///     entity_types = ", ".join(map(str, entity_types))
///
/// tokenizer = tokenizer or get_tokenizer()
///
/// tokens_left = (
///     max_token_count
///     - tokenizer.num_tokens(prompt)  # raw template with {entity_types} etc.
///     - tokenizer.num_tokens(entity_types)  # the joined string
///     if entity_types
///     else 0  # UNTYPED: tokens_left = 0
/// )
///
/// for i, output in enumerate(examples):
///     example_formatted = EXAMPLE_TEMPLATE.format(
///         n=i+1, input_text=input, entity_types=entity_types, output=output
///     )
///     example_tokens = tokenizer.num_tokens(example_formatted)
///     if i >= min_examples_required and example_tokens > tokens_left:
///         break
///     examples_prompt += example_formatted
///     tokens_left -= example_tokens
///
/// prompt = prompt.format(
///     entity_types=entity_types, examples=examples_prompt, language=language
/// )
/// ```
///
/// # Errors
///
/// Returns an error when the tokenizer fails to count tokens.
pub(crate) fn create_extract_graph_prompt(
    entity_types: Option<&[String]>,
    docs: &[String],
    examples: &[String],
    language: &str,
    max_token_count: usize,
    tokenizer: &dyn Tokenizer,
    min_examples_required: usize,
) -> Result<String> {
    if let Some(types) = entity_types {
        // ---- typed extraction ----
        let graph_extraction_prompt = without_storage_lf(GRAPH_EXTRACTION_PROMPT_SOURCE);
        // Token counting: use raw (unescaped) text for budget.
        // Entity types are counted per the GraphRAG 3.1.0 spec: tokenize
        // the raw template with {entity_types} placeholder present, plus
        // the raw entity types string.
        let raw_types_str = types.join(", ");
        let prompt_tokens = counted(tokenizer, graph_extraction_prompt)?;
        let entity_tokens = counted(tokenizer, &raw_types_str)?;
        let mut tokens_left: i64 =
            (max_token_count as i64) - (prompt_tokens as i64) - (entity_tokens as i64);

        let mut examples_prompt = String::new();
        for (i, output) in examples.iter().enumerate() {
            let Some(input) = docs.get(i) else { break };

            // Count tokens on raw text (no Tera escaping)
            let raw_example = EXAMPLE_EXTRACTION_TEMPLATE
                .replace("{n}", &(i + 1).to_string())
                .replace("{input_text}", input)
                .replace("{entity_types}", &raw_types_str)
                .replace("{output}", output);
            let example_tokens = counted(tokenizer, &raw_example)? as i64;

            if i >= min_examples_required && example_tokens > tokens_left {
                break;
            }

            let formatted_example = EXAMPLE_EXTRACTION_TEMPLATE
                .replace("{n}", &(i + 1).to_string())
                .replace("{input_text}", input)
                .replace("{entity_types}", &raw_types_str)
                .replace("{output}", output);
            examples_prompt.push_str(&formatted_example);
            examples_prompt.push('\n');
            tokens_left -= example_tokens;
        }

        let final_prompt = graph_extraction_prompt
            .replace("{examples}", &examples_prompt)
            .replace("{entity_types}", &raw_types_str)
            .replace("{language}", language);

        Ok(final_prompt)
    } else {
        // ---- untyped extraction ----
        let mut tokens_left: i64 = 0;

        let mut examples_prompt = String::new();
        for (i, output) in examples.iter().enumerate() {
            let Some(input) = docs.get(i) else { break };

            // Count tokens on raw text
            let raw_example = UNTYPED_EXAMPLE_EXTRACTION_TEMPLATE
                .replace("{n}", &(i + 1).to_string())
                .replace("{input_text}", input)
                .replace("{output}", output);
            let example_tokens = counted(tokenizer, &raw_example)? as i64;

            if i >= min_examples_required && example_tokens > tokens_left {
                break;
            }

            let formatted_example = UNTYPED_EXAMPLE_EXTRACTION_TEMPLATE
                .replace("{n}", &(i + 1).to_string())
                .replace("{input_text}", input)
                .replace("{output}", output);
            examples_prompt.push_str(&formatted_example);
            examples_prompt.push('\n');
            tokens_left -= example_tokens;
        }

        let final_prompt = UNTYPED_GRAPH_EXTRACTION_PROMPT
            .replace("{examples}", &examples_prompt)
            .replace("{language}", language);

        Ok(final_prompt)
    }
}

#[cfg(test)]
mod budget_tests {
    use graphloom_llm::TiktokenTokenizer;

    use super::*;

    fn tk() -> TiktokenTokenizer {
        TiktokenTokenizer::new("cl100k_base").expect("tokenizer")
    }

    #[test]
    fn test_untyped_tokens_left_is_zero() {
        // GraphRAG 3.1.0: untyped mode sets tokens_left = 0.
        // With tokens_left=0 and min_examples_required=2:
        // - Examples 0,1: always included (i < min)
        // - Example 2: example_tokens > 0 (always true) → break
        // So we get exactly min_examples_required examples.
        let tokenizer = tk();
        let docs: Vec<String> = (0..5)
            .map(|i| format!("document text number {i}"))
            .collect();
        let examples: Vec<String> = (0..5).map(|i| format!("example output {i}")).collect();

        let prompt =
            create_extract_graph_prompt(None, &docs, &examples, "English", 2000, &tokenizer, 2)
                .expect("prompt");

        assert!(prompt.contains("Example 1:"));
        assert!(prompt.contains("Example 2:"));
        assert!(!prompt.contains("Example 3:"));
    }

    #[test]
    fn test_untyped_min_examples_required_1() {
        let tokenizer = tk();
        let docs: Vec<String> = (0..5).map(|i| format!("doc {i}")).collect();
        let examples: Vec<String> = (0..5).map(|i| format!("ex {i}")).collect();

        let prompt =
            create_extract_graph_prompt(None, &docs, &examples, "English", 2000, &tokenizer, 1)
                .expect("prompt");

        assert!(prompt.contains("Example 1:"));
        assert!(!prompt.contains("Example 2:"));
    }

    #[test]
    fn test_typed_counts_raw_template_before_format() {
        // The token count should be on the raw template with {entity_types}
        // and {language} placeholders, NOT on the pre-substituted version.
        let tokenizer = tk();
        let docs = vec!["short doc".to_owned()];
        let examples = vec!["out".to_owned()];
        let entity_types = vec!["person".to_owned(), "org".to_owned()];

        // Raw template token count (with {entity_types} etc.) + entity_types string
        let raw_tokens = counted(
            &tokenizer,
            without_storage_lf(GRAPH_EXTRACTION_PROMPT_SOURCE),
        )
        .unwrap();
        let et_tokens = counted(&tokenizer, "person, org").unwrap();
        let base = raw_tokens + et_tokens;

        // With max_tokens = base + small margin, the first example should fit
        let prompt = create_extract_graph_prompt(
            Some(&entity_types),
            &docs,
            &examples,
            "English",
            base + 100,
            &tokenizer,
            2,
        )
        .expect("prompt");

        assert!(prompt.contains("Example 1:"));
    }

    #[test]
    fn test_typed_minimal_max_tokens() {
        let tokenizer = tk();
        let docs = vec!["short doc".to_owned(), "another doc".to_owned()];
        let examples = vec!["out1".to_owned(), "out2".to_owned()];
        let entity_types = vec!["person".to_owned(), "org".to_owned()];

        // max_tokens=0 means tokens_left is deeply negative
        // min_examples_required=2 means first 2 always included
        let prompt = create_extract_graph_prompt(
            Some(&entity_types),
            &docs,
            &examples,
            "English",
            0,
            &tokenizer,
            2,
        )
        .expect("prompt");

        assert!(prompt.contains("Example 1:"));
        assert!(prompt.contains("Example 2:"));
    }
}
