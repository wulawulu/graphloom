//! Community report summarization prompt assembly.

use super::super::selection::escape_tera_literal;

/// Canonical output filename.
#[allow(dead_code, reason = "used by CLI and public API")]
pub(crate) const COMMUNITY_SUMMARIZATION_FILENAME: &str = "community_report_graph.txt";

// NOTE: {persona}, {role}, {report_rating_description}, and {language} are
// Python-style placeholders resolved via `.replace()`.  JSON braces are written
// as single `{` / `}` because Tera treats single braces as literal text.  The
// only Tera expression is `{{input_text}}`.

const COMMUNITY_REPORT_SUMMARIZATION_PROMPT: &str =
    include_str!("../templates/community_report_graph.txt");

/// Create a community report summarization prompt from the tuned components.
///
/// The resulting template includes the Tera variable `{{ input_text }}`.
///
/// External text (persona, role, rating, language) is Tera-escaped to prevent
/// LLM-generated content containing `{{`, `{%`, or `{#` from being interpreted
/// as Tera syntax.
pub(crate) fn create_community_summarization_prompt(
    persona: &str,
    role: &str,
    report_rating_description: &str,
    language: &str,
) -> String {
    COMMUNITY_REPORT_SUMMARIZATION_PROMPT
        .replace("{persona}", &escape_tera_literal(persona))
        .replace("{role}", &escape_tera_literal(role))
        .replace(
            "{report_rating_description}",
            &escape_tera_literal(report_rating_description),
        )
        .replace("{language}", &escape_tera_literal(language))
}
