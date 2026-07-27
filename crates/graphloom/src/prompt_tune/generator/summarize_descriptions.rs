//! Entity summarization prompt assembly.

use super::super::selection::escape_tera_literal;

/// Canonical output filename.
#[allow(dead_code, reason = "used by CLI and public API")]
pub(crate) const ENTITY_SUMMARIZATION_FILENAME: &str = "summarize_descriptions.txt";

const ENTITY_SUMMARIZATION_PROMPT: &str = include_str!("../templates/summarize_descriptions.txt");

/// Create an entity summarization prompt from the tuned persona and language.
///
/// External text (persona, language) is Tera-escaped to prevent LLM-generated
/// content containing `{{`, `{%`, or `{#` from being interpreted as Tera syntax.
pub(crate) fn create_entity_summarization_prompt(persona: &str, language: &str) -> String {
    ENTITY_SUMMARIZATION_PROMPT
        .replace("{persona}", &escape_tera_literal(persona))
        .replace("{language}", &escape_tera_literal(language))
}
