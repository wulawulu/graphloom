//! Entity summarization prompt assembly.

use super::without_storage_lf;

/// Canonical output filename.
#[allow(dead_code, reason = "used by CLI and public API")]
pub(crate) const ENTITY_SUMMARIZATION_FILENAME: &str = "summarize_descriptions.txt";

const ENTITY_SUMMARIZATION_PROMPT_SOURCE: &str =
    include_str!("../templates/summarize_descriptions.txt");

/// Create an entity summarization prompt from the tuned persona and language.
pub(crate) fn create_entity_summarization_prompt(persona: &str, language: &str) -> String {
    without_storage_lf(ENTITY_SUMMARIZATION_PROMPT_SOURCE)
        .replace("{persona}", persona)
        .replace("{language}", language)
}
