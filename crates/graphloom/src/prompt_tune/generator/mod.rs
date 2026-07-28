//! LLM-based prompt generation steps.

mod community_report;
mod community_report_rating;
mod community_reporter_role;
mod domain;
mod entity_relationship;
mod entity_types;
mod extract_graph;
mod language;
mod meta_prompts;
mod persona;
mod summarize_descriptions;

pub(crate) use community_report::create_community_summarization_prompt;
pub(crate) use community_report_rating::generate_community_report_rating;
pub(crate) use community_reporter_role::generate_community_reporter_role;
pub(crate) use domain::generate_domain;
pub(crate) use entity_relationship::generate_entity_relationship_examples;
pub(crate) use entity_types::generate_entity_types;
pub(crate) use extract_graph::create_extract_graph_prompt;
pub(crate) use language::detect_language;
pub(crate) use meta_prompts::PROMPT_TUNING_MODEL_ID;
pub(crate) use persona::generate_persona;
pub(crate) use summarize_descriptions::create_entity_summarization_prompt;

/// Remove the repository storage LF from GraphRAG constants whose Python
/// triple-quoted value intentionally has no trailing newline.
fn without_storage_lf(source: &str) -> &str {
    source.strip_suffix('\n').unwrap_or(source)
}
