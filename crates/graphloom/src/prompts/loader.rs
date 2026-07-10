//! Prompt override loading and explicit syntax selection.

use std::{
    error::Error,
    path::{Path, PathBuf},
};

use serde::Serialize;
use tera::{Context, Tera};

use super::{
    PromptKind,
    renderer::{prompt_values, render_graphrag_prompt},
};
use crate::{GraphLoomError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptSyntax {
    GraphRag,
    Tera,
}

#[derive(Debug)]
struct LoadedPrompt {
    content: String,
    syntax: PromptSyntax,
}

/// Loads `GraphRAG` prompts using project-aware override precedence.
#[derive(Debug, Clone)]
pub(crate) struct PromptLoader {
    project_root: PathBuf,
}

impl PromptLoader {
    /// Create a loader rooted at a `GraphLoom` project directory.
    pub(crate) fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
        }
    }

    /// Render an explicit override, project override, or built-in prompt.
    pub(crate) async fn render<T>(
        &self,
        kind: PromptKind,
        explicit_path: Option<&Path>,
        values: &T,
    ) -> Result<String>
    where
        T: Serialize,
    {
        let loaded = self.load_template(kind, explicit_path).await?;
        match loaded.syntax {
            PromptSyntax::GraphRag => render_graphrag_prompt(kind, &loaded.content, values),
            PromptSyntax::Tera => render_tera_prompt(kind, &loaded.content, values),
        }
    }

    async fn load_template(
        &self,
        kind: PromptKind,
        explicit_path: Option<&Path>,
    ) -> Result<LoadedPrompt> {
        if let Some(path) = explicit_path {
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                self.project_root.join(path)
            };
            return load_override(kind, &path).await;
        }

        let prompts = self.project_root.join("prompts");
        let canonical = prompts.join(kind.filename());
        if path_exists(&canonical).await? {
            return load_override(kind, &canonical).await;
        }
        if let Some(legacy_filename) = kind.legacy_filename() {
            let legacy = prompts.join(legacy_filename);
            if path_exists(&legacy).await? {
                return load_override(kind, &legacy).await;
            }
        }

        Ok(LoadedPrompt {
            content: kind.default_template().to_owned(),
            syntax: PromptSyntax::GraphRag,
        })
    }
}

async fn load_override(kind: PromptKind, path: &Path) -> Result<LoadedPrompt> {
    let content =
        tokio::fs::read_to_string(path)
            .await
            .map_err(|source| GraphLoomError::PromptLoad {
                path: path.to_path_buf(),
                source,
            })?;
    let syntax = detect_prompt_syntax(kind, &content)?;
    Ok(LoadedPrompt { content, syntax })
}

async fn path_exists(path: &Path) -> Result<bool> {
    tokio::fs::try_exists(path)
        .await
        .map_err(|source| GraphLoomError::PromptLoad {
            path: path.to_path_buf(),
            source,
        })
}

fn detect_prompt_syntax(kind: PromptKind, template: &str) -> Result<PromptSyntax> {
    let mut has_graphrag = false;
    let mut has_tera = false;
    for variable in kind.required_variables() {
        has_graphrag |= contains_graphrag_variable(template, variable);
        has_tera |= contains_tera_variable(template, variable);
    }
    if has_graphrag && has_tera {
        return Err(GraphLoomError::PromptRender {
            name: kind.filename(),
            message: "mixed prompt syntax: GraphRAG and Tera variables cannot be combined"
                .to_owned(),
        });
    }
    Ok(if has_tera {
        PromptSyntax::Tera
    } else {
        PromptSyntax::GraphRag
    })
}

fn contains_graphrag_variable(template: &str, expected: &str) -> bool {
    let bytes = template.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        if bytes[cursor] != b'{' {
            cursor = cursor.saturating_add(1);
            continue;
        }
        if bytes.get(cursor.saturating_add(1)) == Some(&b'{') {
            cursor = cursor.saturating_add(2);
            continue;
        }
        let start = cursor.saturating_add(1);
        let Some(relative_end) = template[start..].find('}') else {
            return false;
        };
        let end = start.saturating_add(relative_end);
        if template[start..end].trim() == expected {
            return true;
        }
        cursor = end.saturating_add(1);
    }
    false
}

fn contains_tera_variable(template: &str, expected: &str) -> bool {
    let mut remaining = template;
    while let Some(start) = remaining.find("{{") {
        let expression = &remaining[start.saturating_add(2)..];
        let Some(end) = expression.find("}}") else {
            return false;
        };
        if tera_expression_references_variable(&expression[..end], expected) {
            return true;
        }
        remaining = &expression[end.saturating_add(2)..];
    }
    false
}

fn tera_expression_references_variable(expression: &str, expected: &str) -> bool {
    let expression = expression.trim();
    let Some(suffix) = expression.strip_prefix(expected) else {
        return false;
    };
    let Some(next) = suffix.chars().next() else {
        return true;
    };
    next.is_whitespace() || matches!(next, '|' | '.' | '[')
}

fn render_tera_prompt<T>(kind: PromptKind, template: &str, values: &T) -> Result<String>
where
    T: Serialize,
{
    let values = prompt_values(kind, values)?;
    let context = Context::from_value(serde_json::Value::Object(values)).map_err(|source| {
        GraphLoomError::PromptRender {
            name: kind.filename(),
            message: tera_error_message(&source),
        }
    })?;
    Tera::one_off(template, &context, false).map_err(|source| GraphLoomError::PromptRender {
        name: kind.filename(),
        message: tera_error_message(&source),
    })
}

fn tera_error_message(error: &tera::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use serde::Serialize;
    use tempfile::TempDir;

    use super::*;

    #[derive(Debug, Serialize)]
    struct ClaimsValues<'a> {
        input_text: &'a str,
        entity_specs: &'a [&'a str],
        claim_description: &'a str,
    }

    #[derive(Debug, Serialize)]
    struct GraphValues<'a> {
        input_text: &'a str,
        entity_types: &'a [&'a str],
    }

    #[derive(Debug, Serialize)]
    struct ReportValues<'a> {
        input_text: &'a str,
        max_report_length: usize,
    }

    #[tokio::test]
    async fn test_should_render_builtin_extract_claims_prompt() {
        let rendered = PromptLoader::new(".")
            .render(
                PromptKind::ExtractClaims,
                None,
                &ClaimsValues {
                    input_text: "tu-2 should fail.",
                    entity_specs: &["person"],
                    claim_description: "claims",
                },
            )
            .await
            .expect("built-in claims prompt should render");

        assert!(rendered.contains("tu-2 should fail."));
        assert!(rendered.contains("person"));
        assert!(rendered.contains("claims"));
        assert!(!rendered.contains("{input_text}"));
        assert!(!rendered.contains("{entity_specs}"));
        assert!(!rendered.contains("{claim_description}"));
    }

    #[tokio::test]
    async fn test_should_render_builtin_extract_graph_prompt() {
        let rendered = PromptLoader::new(".")
            .render(
                PromptKind::ExtractGraph,
                None,
                &GraphValues {
                    input_text: "Alice met Bob.",
                    entity_types: &["person"],
                },
            )
            .await
            .expect("built-in graph prompt should render");

        assert!(rendered.contains("Alice met Bob."));
        assert!(rendered.contains("person"));
        assert!(!rendered.contains("{input_text}"));
        assert!(!rendered.contains("{entity_types}"));
    }

    #[tokio::test]
    async fn test_should_preserve_graphrag_placeholder_text_from_input_value() {
        let input_text = "Documentation mentions {input_text} and {entity_types}.";
        let rendered = PromptLoader::new(".")
            .render(
                PromptKind::ExtractGraph,
                None,
                &GraphValues {
                    input_text,
                    entity_types: &["person"],
                },
            )
            .await
            .expect("placeholder text from the input value should be preserved");

        assert!(rendered.contains(input_text));
    }

    #[tokio::test]
    async fn test_should_preserve_claim_placeholder_text_from_input_value() {
        let input_text = "The text contains {claim_description} literally.";
        let rendered = PromptLoader::new(".")
            .render(
                PromptKind::ExtractClaims,
                None,
                &ClaimsValues {
                    input_text,
                    entity_specs: &["person"],
                    claim_description: "claims",
                },
            )
            .await
            .expect("claim placeholder text from the input value should be preserved");

        assert!(rendered.contains(input_text));
    }

    #[tokio::test]
    async fn test_should_render_tera_override() {
        let tempdir = prompt_project("Text: {{ input_text }}\nTypes: {{ entity_types }}").await;
        let rendered = render_graph_override(&tempdir)
            .await
            .expect("Tera override");

        assert!(rendered.contains("Text: Alice"));
        assert!(rendered.contains("Types:"));
        assert!(rendered.contains("person"));
        assert!(!rendered.contains("{{ entity_types }}"));
    }

    #[tokio::test]
    async fn test_should_preserve_tera_placeholder_text_from_input_value() {
        let tempdir = prompt_project("Text: {{ input_text }}").await;
        let input_text = "Example containing {{ input_text }} and {input_text}";
        let rendered = PromptLoader::new(tempdir.path())
            .render(
                PromptKind::ExtractGraph,
                None,
                &GraphValues {
                    input_text,
                    entity_types: &["person"],
                },
            )
            .await
            .expect("Tera placeholder text from the input value should be preserved");

        assert_eq!(rendered, format!("Text: {input_text}"));
    }

    #[tokio::test]
    async fn test_should_render_tera_override_with_filters() {
        let tempdir = prompt_project(
            "Text: {{ input_text | trim }}\nTypes: {{ entity_types | join(sep=\",\") }}",
        )
        .await;
        let rendered = PromptLoader::new(tempdir.path())
            .render(
                PromptKind::ExtractGraph,
                None,
                &GraphValues {
                    input_text: "  Alice met Bob.  ",
                    entity_types: &["person", "organization"],
                },
            )
            .await
            .expect("Tera filters should render");

        assert_eq!(rendered, "Text: Alice met Bob.\nTypes: person,organization");
    }

    #[tokio::test]
    async fn test_should_render_graphrag_override() {
        let tempdir = prompt_project("Text: {input_text}\nTypes: {entity_types}").await;
        let rendered = render_graph_override(&tempdir)
            .await
            .expect("GraphRAG override");

        assert!(rendered.contains("Text: Alice"));
        assert!(rendered.contains(r#"Types: ["person"]"#));
    }

    #[tokio::test]
    async fn test_should_reject_mixed_prompt_syntax() {
        let tempdir = prompt_project("Text: {input_text}\nTypes: {{ entity_types }}").await;
        let error = render_graph_override(&tempdir)
            .await
            .expect_err("mixed syntax must fail");

        assert!(error.to_string().contains("mixed prompt syntax"));
    }

    #[tokio::test]
    async fn test_should_reject_missing_prompt_variable() {
        let tempdir = prompt_project("Text: {input_text}\nMissing: {missing_variable}").await;
        let error = render_graph_override(&tempdir)
            .await
            .expect_err("missing variable must fail");

        assert!(error.to_string().contains("missing_variable"));
    }

    #[tokio::test]
    async fn test_should_reject_missing_tera_prompt_variable() {
        let tempdir =
            prompt_project("Text: {{ input_text }}\nUnknown: {{ missing_variable }}").await;
        let error = render_graph_override(&tempdir)
            .await
            .expect_err("missing Tera variable must fail");

        assert!(error.to_string().contains("missing_variable"));
    }

    #[tokio::test]
    async fn test_should_reject_missing_required_value() {
        #[derive(Debug, Serialize)]
        struct IncompleteValues<'a> {
            input_text: &'a str,
        }

        let error = PromptLoader::new(".")
            .render(
                PromptKind::ExtractGraph,
                None,
                &IncompleteValues {
                    input_text: "Alice",
                },
            )
            .await
            .expect_err("missing required value must fail");

        assert!(error.to_string().contains("entity_types"));
    }

    #[tokio::test]
    async fn test_should_render_graphrag_json_literal() {
        let tempdir = prompt_project(
            "Example: {{\"title\":\"example\"}}\nText: {input_text}\nTypes: {entity_types}",
        )
        .await;
        let rendered = render_graph_override(&tempdir)
            .await
            .expect("JSON literal should render");

        assert!(rendered.contains(r#"{"title":"example"}"#));
    }

    #[test]
    fn test_should_detect_tera_required_variable_expressions() {
        for expression in [
            "{{ input_text }}",
            "{{input_text}}",
            "{{ input_text | trim }}",
            "{{ input_text|escape }}",
            "{{ input_text.value }}",
            "{{ input_text[\"value\"] }}",
            "{{ input_text[0] }}",
        ] {
            assert!(
                contains_tera_variable(expression, "input_text"),
                "expected Tera expression to be detected: {expression}",
            );
        }

        for expression in [
            "{{ input_text_extra }}",
            "{{ other_input_text }}",
            "{{ input_text2 }}",
        ] {
            assert!(
                !contains_tera_variable(expression, "input_text"),
                "expected prefix lookalike not to be detected: {expression}",
            );
        }
    }

    #[test]
    fn test_should_classify_tera_attribute_and_index_access() {
        for template in [
            "Text: {{ input_text.value }}",
            "Types: {{ entity_types[0] }}",
        ] {
            assert_eq!(
                detect_prompt_syntax(PromptKind::ExtractGraph, template)
                    .expect("syntax should be detected"),
                PromptSyntax::Tera,
            );
        }
    }

    #[test]
    fn test_should_not_classify_json_literal_as_tera() {
        assert_eq!(
            detect_prompt_syntax(
                PromptKind::ExtractGraph,
                r#"Example: {{"input_text":"literal"}}"#,
            )
            .expect("JSON literal should be classified"),
            PromptSyntax::GraphRag,
        );
        assert_eq!(
            detect_prompt_syntax(PromptKind::ExtractGraph, "Text: {{ input_text | trim }}",)
                .expect("filter expression should be classified"),
            PromptSyntax::Tera,
        );
    }

    #[tokio::test]
    async fn test_should_reject_malformed_graphrag_braces() {
        for template in [
            "Text: {input_text}\nTypes: {entity_types",
            "Text: {input_text}\nTypes: {entity_types}}",
        ] {
            let tempdir = prompt_project(template).await;
            let error = render_graph_override(&tempdir)
                .await
                .expect_err("malformed braces must fail");

            assert!(
                error.to_string().contains("unclosed") || error.to_string().contains("isolated")
            );
        }
    }

    #[tokio::test]
    async fn test_should_load_legacy_community_report_override() {
        let tempdir = TempDir::new().expect("tempdir");
        let prompts = tempdir.path().join("prompts");
        tokio::fs::create_dir(&prompts).await.expect("prompts");
        tokio::fs::write(
            prompts.join("community_report_graph.txt"),
            "Legacy: {input_text}; limit={max_report_length}",
        )
        .await
        .expect("legacy override");

        let rendered = PromptLoader::new(tempdir.path())
            .render(
                PromptKind::CommunityReport,
                None,
                &ReportValues {
                    input_text: "Alice",
                    max_report_length: 42,
                },
            )
            .await
            .expect("legacy community override should render");

        assert_eq!(rendered, "Legacy: Alice; limit=42");
    }

    async fn prompt_project(template: &str) -> TempDir {
        let tempdir = TempDir::new().expect("tempdir");
        let prompts = tempdir.path().join("prompts");
        tokio::fs::create_dir(&prompts).await.expect("prompts");
        tokio::fs::write(prompts.join("extract_graph.txt"), template)
            .await
            .expect("override");
        tempdir
    }

    async fn render_graph_override(tempdir: &TempDir) -> Result<String> {
        PromptLoader::new(tempdir.path())
            .render(
                PromptKind::ExtractGraph,
                None,
                &GraphValues {
                    input_text: "Alice",
                    entity_types: &["person"],
                },
            )
            .await
    }
}
