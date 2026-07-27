//! Internal GraphRAG prompt-tuning meta-prompts.
//!
//! The text assets store the final GraphLoom-side template representation.
//! Named placeholders such as `{input_text}` are resolved with explicit
//! replacement before the LLM request.
//!
//! Python-only `.format()` brace escapes from GraphRAG source templates are
//! already normalized to the literal text GraphRAG sends to the model.

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

pub(crate) const DEFAULT_TASK: &str = concat!(
    "\nIdentify the relations and structure of the community of interest, ",
    "specifically within the {domain} domain.\n",
);

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

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_task_matches_graphrag_triple_quoted_bytes() {
        assert_eq!(
            DEFAULT_TASK.as_bytes(),
            b"\nIdentify the relations and structure of the community of interest, specifically within the {domain} domain.\n",
        );
    }

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

    // ---- brace semantics (Python .format() normalization) ----

    #[test]
    fn persona_uses_single_braces_for_example_role() {
        // After Python .format() normalization, {{role}} → {role}
        assert!(GENERATE_PERSONA_PROMPT.contains("an expert {role}"));
        assert!(!GENERATE_PERSONA_PROMPT.contains("{{{{role}}}}"));
        assert!(!GENERATE_PERSONA_PROMPT.contains("{{role}}"));
    }

    #[test]
    fn entity_types_uses_single_braces_for_output_marker() {
        // After Python .format() normalization, {{<entity_types>}} → {<entity_types>}
        assert!(ENTITY_TYPE_GENERATION_PROMPT.contains("{<entity_types>}"));
        assert!(!ENTITY_TYPE_GENERATION_PROMPT.contains("{{<entity_types>}}"));
    }

    #[test]
    fn entity_types_json_uses_single_braces_for_json_examples() {
        // After Python .format() normalization, {{"entity_types":...}} → {"entity_types":...}
        assert!(ENTITY_TYPE_GENERATION_JSON_PROMPT.contains("{\"entity_types\":"));
        assert!(!ENTITY_TYPE_GENERATION_JSON_PROMPT.contains("{{\"entity_types\":"));
    }

    // ---- leading/trailing newlines ----

    #[test]
    fn all_prompts_start_with_newline_except_report_rating() {
        // All GraphRAG prompt strings use triple-quoted """...""" which
        // introduces a leading newline.
        let prompts: &[(&str, &str)] = &[
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
        for (name, content) in prompts {
            assert!(
                content.starts_with('\n'),
                "{name} should start with \\n, got: {:?}",
                &content[..content.len().min(40)]
            );
        }
    }

    #[test]
    fn report_rating_starts_with_double_newline() {
        // GENERATE_REPORT_RATING_PROMPT has a blank line after the opening
        // triple-quote in the Python source, producing two leading newlines.
        assert!(GENERATE_REPORT_RATING_PROMPT.starts_with("\n\n"));
    }
}
