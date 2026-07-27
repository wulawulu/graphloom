//! Internal meta-prompts used by prompt-tuning generators.
//!
//! These are the prompts sent TO the LLM during prompt tuning, NOT the
//! final Tera templates that users run at indexing time.  They use
//! Python-style `{variable}` placeholders resolved via Rust `format!`
//! before the LLM call.
//!
//! All text matches GraphRAG 3.1.0 prompt content exactly, except that
//! literal braces in example JSON are preserved through `format!`-safe
//! escaping (`{{` / `}}`).

// ---- domain ----------------------------------------------------------------

pub(crate) const GENERATE_DOMAIN_PROMPT: &str = include_str!("../prompts/generate_domain.txt");

// ---- language --------------------------------------------------------------

pub(crate) const DETECT_LANGUAGE_PROMPT: &str = include_str!("../prompts/detect_language.txt");

// ---- persona ---------------------------------------------------------------

pub(crate) const GENERATE_PERSONA_PROMPT: &str = include_str!("../prompts/generate_persona.txt");

// ---- entity types ----------------------------------------------------------

pub(crate) const ENTITY_TYPE_GENERATION_PROMPT: &str =
    include_str!("../prompts/generate_entity_types.txt");

pub(crate) const ENTITY_TYPE_GENERATION_JSON_PROMPT: &str =
    include_str!("../prompts/generate_entity_types_json.txt");

// ---- community report rating -----------------------------------------------

pub(crate) const GENERATE_REPORT_RATING_PROMPT: &str =
    include_str!("../prompts/generate_report_rating.txt");

// ---- community reporter role -----------------------------------------------

pub(crate) const GENERATE_COMMUNITY_REPORTER_ROLE_PROMPT: &str =
    include_str!("../prompts/generate_community_reporter_role.txt");

// ---- entity relationship examples (typed) ----------------------------------

pub(crate) const ENTITY_RELATIONSHIPS_GENERATION_PROMPT: &str =
    include_str!("../prompts/generate_entity_relationship_examples.txt");

pub(crate) const UNTYPED_ENTITY_RELATIONSHIPS_GENERATION_PROMPT: &str =
    include_str!("../prompts/generate_entity_relationship_examples_untyped.txt");

// ---- default task ----------------------------------------------------------

pub(crate) const DEFAULT_TASK: &str = "Identify the relations and structure of the community of \
                                       interest, specifically within the {domain} domain.";

// ---- default constants -----------------------------------------------------

#[allow(dead_code, reason = "retained for GraphRAG 3.1.0 parity")]
pub(crate) const K: usize = 15;
#[allow(dead_code, reason = "retained for GraphRAG 3.1.0 parity")]
pub(crate) const LIMIT: usize = 15;
#[allow(dead_code, reason = "retained for GraphRAG 3.1.0 parity")]
pub(crate) const MAX_TOKEN_COUNT: usize = 2000;
#[allow(dead_code, reason = "retained for GraphRAG 3.1.0 parity")]
pub(crate) const N_SUBSET_MAX: usize = 300;
pub(crate) const PROMPT_TUNING_MODEL_ID: &str = "default_completion_model";
pub(crate) const MAX_EXAMPLES: usize = 5;

// ---- template extraction helpers -------------------------------------------

/// Replaces Python-style `{` and `}` in document text with Tera-safe `{{` and `}}`
/// (but only when they aren't already doubled).  GraphRAG 3.1.0 does this before
/// feeding chunks into `str.format()`.
#[allow(
    dead_code,
    reason = "no longer needed; document braces are Tera-safe as-is"
)]
pub(crate) fn escape_doc_braces(text: &str) -> String {
    text.replace('{', "{{").replace('}', "}}")
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_meta_prompt_assets_non_empty() {
        let assets: &[(&str, &str)] = &[
            ("GENERATE_DOMAIN_PROMPT", GENERATE_DOMAIN_PROMPT),
            ("DETECT_LANGUAGE_PROMPT", DETECT_LANGUAGE_PROMPT),
            ("GENERATE_PERSONA_PROMPT", GENERATE_PERSONA_PROMPT),
            (
                "ENTITY_TYPE_GENERATION_PROMPT",
                ENTITY_TYPE_GENERATION_PROMPT,
            ),
            (
                "ENTITY_TYPE_GENERATION_JSON_PROMPT",
                ENTITY_TYPE_GENERATION_JSON_PROMPT,
            ),
            (
                "GENERATE_REPORT_RATING_PROMPT",
                GENERATE_REPORT_RATING_PROMPT,
            ),
            (
                "GENERATE_COMMUNITY_REPORTER_ROLE_PROMPT",
                GENERATE_COMMUNITY_REPORTER_ROLE_PROMPT,
            ),
            (
                "ENTITY_RELATIONSHIPS_GENERATION_PROMPT",
                ENTITY_RELATIONSHIPS_GENERATION_PROMPT,
            ),
            (
                "UNTYPED_ENTITY_RELATIONSHIPS_GENERATION_PROMPT",
                UNTYPED_ENTITY_RELATIONSHIPS_GENERATION_PROMPT,
            ),
        ];
        for (name, content) in assets {
            assert!(!content.trim().is_empty(), "{name} should be non-empty");
        }
    }

    #[test]
    fn domain_prompt_has_placeholder() {
        assert!(GENERATE_DOMAIN_PROMPT.contains("{input_text}"));
    }

    #[test]
    fn language_prompt_has_placeholder() {
        assert!(DETECT_LANGUAGE_PROMPT.contains("{input_text}"));
    }

    #[test]
    fn persona_prompt_has_placeholder() {
        assert!(GENERATE_PERSONA_PROMPT.contains("{sample_task}"));
    }

    #[test]
    fn entity_types_prompt_has_placeholders() {
        assert!(ENTITY_TYPE_GENERATION_PROMPT.contains("{task}"));
        assert!(ENTITY_TYPE_GENERATION_PROMPT.contains("{input_text}"));
    }

    #[test]
    fn entity_types_json_prompt_has_placeholders() {
        assert!(ENTITY_TYPE_GENERATION_JSON_PROMPT.contains("{task}"));
        assert!(ENTITY_TYPE_GENERATION_JSON_PROMPT.contains("{input_text}"));
    }

    #[test]
    fn report_rating_prompt_has_placeholders() {
        assert!(GENERATE_REPORT_RATING_PROMPT.contains("{domain}"));
        assert!(GENERATE_REPORT_RATING_PROMPT.contains("{persona}"));
        assert!(GENERATE_REPORT_RATING_PROMPT.contains("{input_text}"));
    }

    #[test]
    fn community_reporter_role_prompt_has_placeholders() {
        assert!(GENERATE_COMMUNITY_REPORTER_ROLE_PROMPT.contains("{persona}"));
        assert!(GENERATE_COMMUNITY_REPORTER_ROLE_PROMPT.contains("{domain}"));
        assert!(GENERATE_COMMUNITY_REPORTER_ROLE_PROMPT.contains("{input_text}"));
    }

    #[test]
    fn entity_relationships_prompt_has_placeholders() {
        assert!(ENTITY_RELATIONSHIPS_GENERATION_PROMPT.contains("{entity_types}"));
        assert!(ENTITY_RELATIONSHIPS_GENERATION_PROMPT.contains("{input_text}"));
        assert!(ENTITY_RELATIONSHIPS_GENERATION_PROMPT.contains("{language}"));
    }

    #[test]
    fn untyped_entity_relationships_prompt_has_placeholders() {
        assert!(UNTYPED_ENTITY_RELATIONSHIPS_GENERATION_PROMPT.contains("{input_text}"));
        assert!(UNTYPED_ENTITY_RELATIONSHIPS_GENERATION_PROMPT.contains("{language}"));
    }
}
