//! Community report summarization prompt assembly.

use super::without_storage_lf;

/// Canonical output filename.
#[allow(dead_code, reason = "used by CLI and public API")]
pub(crate) const COMMUNITY_SUMMARIZATION_FILENAME: &str = "community_report_graph.txt";

// NOTE: {persona}, {role}, {report_rating_description}, and {language} are
// Python-style placeholders resolved via `.replace()`. JSON braces remain
// doubled and the runtime input placeholder remains single-braced, exactly as
// GraphRAG's first `.format()` call emits them.

const COMMUNITY_REPORT_SUMMARIZATION_PROMPT_SOURCE: &str =
    include_str!("../templates/community_report_graph.txt");

/// Create a community report summarization prompt from the tuned components.
///
/// The resulting template includes GraphRAG's `{input_text}` format variable.
pub(crate) fn create_community_summarization_prompt(
    persona: &str,
    role: &str,
    report_rating_description: &str,
    language: &str,
) -> String {
    without_storage_lf(COMMUNITY_REPORT_SUMMARIZATION_PROMPT_SOURCE)
        .replace("{persona}", persona)
        .replace("{role}", role)
        .replace("{report_rating_description}", report_rating_description)
        .replace("{language}", language)
}
