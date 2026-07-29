//! Project-scoped prompt template loading.

use std::path::{Path, PathBuf};

use super::{PromptKind, PromptSource, PromptTemplate, prompt::prompt_render_error};
use crate::{GraphLoomError, Result};

/// Loads prompt templates for exactly one `GraphLoom` project root.
#[derive(Clone, Debug)]
pub(crate) struct PromptRepository {
    project_root: PathBuf,
}

impl PromptRepository {
    /// Create a repository rooted at one `GraphLoom` project directory.
    pub(crate) fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
        }
    }

    /// Load an explicitly configured template or the built-in default.
    pub(crate) async fn load(
        &self,
        kind: PromptKind,
        configured_path: Option<&Path>,
    ) -> Result<PromptTemplate> {
        if let Some(path) = configured_path {
            let path = self.resolve(path);
            return load_file(kind, path, PromptSource::Explicit).await;
        }

        let project_path = self.project_root.join("prompts").join(kind.filename());
        if tokio::fs::try_exists(&project_path)
            .await
            .map_err(|source| GraphLoomError::PromptLoad {
                kind: kind.name(),
                name: kind.filename(),
                path: project_path.clone(),
                source,
            })?
        {
            return load_file(kind, project_path, PromptSource::Project).await;
        }

        build_template(kind, kind.default_template(), PromptSource::BuiltIn)
    }

    /// Load a configured path or inline template, then project/default fallbacks.
    pub(crate) async fn load_configured(
        &self,
        kind: PromptKind,
        configured: Option<&str>,
    ) -> Result<PromptTemplate> {
        let Some(configured) = configured else {
            return self.load(kind, None).await;
        };
        if configured.contains('\n') {
            return build_template(kind, configured, PromptSource::Inline);
        }
        let path = Path::new(configured);
        let resolved = self.resolve(path);
        if tokio::fs::try_exists(&resolved)
            .await
            .map_err(|source| GraphLoomError::PromptLoad {
                kind: kind.name(),
                name: kind.filename(),
                path: resolved.clone(),
                source,
            })?
        {
            return load_file(kind, resolved, PromptSource::Explicit).await;
        }
        if configured.contains("{{") || configured.contains("{%") {
            return build_template(kind, configured, PromptSource::Inline);
        }
        load_file(kind, resolved, PromptSource::Explicit).await
    }

    fn resolve(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.project_root.join(path)
        }
    }
}

async fn load_file(
    kind: PromptKind,
    path: PathBuf,
    source: fn(PathBuf) -> PromptSource,
) -> Result<PromptTemplate> {
    let content =
        tokio::fs::read_to_string(&path)
            .await
            .map_err(|error| GraphLoomError::PromptLoad {
                kind: kind.name(),
                name: kind.filename(),
                path: path.clone(),
                source: error,
            })?;
    build_template(kind, content, source(path))
}

fn build_template(
    kind: PromptKind,
    content: impl Into<std::sync::Arc<str>>,
    source: PromptSource,
) -> Result<PromptTemplate> {
    let content = convert_graphrag_format_syntax(kind, content.into(), &source)?;
    PromptTemplate::try_new(kind, content, source)
}

fn convert_graphrag_format_syntax(
    kind: PromptKind,
    content: std::sync::Arc<str>,
    source: &PromptSource,
) -> Result<std::sync::Arc<str>> {
    if !contains_graphrag_field(kind, &content) {
        return Ok(content);
    }

    let parts = scan_graphrag_format(&content)
        .map_err(|message| prompt_render_error(kind, source, message))?;
    let mut converted = String::with_capacity(content.len());
    for part in parts {
        match part {
            GraphRagFormatPart::Literal(literal) => {
                converted.push_str(&escape_tera_delimiters(&literal));
            }
            GraphRagFormatPart::Field(field) => {
                converted.push_str("{{ ");
                converted.push_str(&field);
                converted.push_str(" }}");
            }
        }
    }
    Ok(std::sync::Arc::from(converted))
}

fn escape_tera_delimiters(input: &str) -> String {
    input
        .replace("{{", "{{ \"{{\" }}")
        .replace("{%", "{{ \"{%\" }}")
        .replace("{#", "{{ \"{#\" }}")
}

#[derive(Debug, PartialEq, Eq)]
enum GraphRagFormatPart {
    Literal(String),
    Field(String),
}

fn contains_graphrag_field(kind: PromptKind, template: &str) -> bool {
    let bytes = template.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'{' {
            index += 1;
            continue;
        }
        if bytes.get(index + 1) == Some(&b'{') {
            index += 2;
            continue;
        }

        let field_start = index + 1;
        let mut cursor = field_start;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'{' => {
                    index = cursor;
                    break;
                }
                b'}' => {
                    let field = &template[field_start..cursor];
                    if kind.variables().contains(&field) {
                        return true;
                    }
                    index = cursor + 1;
                    break;
                }
                _ => cursor += 1,
            }
        }
        if cursor == bytes.len() {
            index = field_start;
        }
    }
    false
}

fn scan_graphrag_format(template: &str) -> std::result::Result<Vec<GraphRagFormatPart>, String> {
    let bytes = template.as_bytes();
    let mut parts = Vec::new();
    let mut literal = String::with_capacity(template.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'{' if bytes.get(index + 1) == Some(&b'{') => {
                literal.push('{');
                index += 2;
            }
            b'{' => {
                push_literal_part(&mut parts, &mut literal);
                let field_start = index + 1;
                let mut field_end = field_start;
                while field_end < bytes.len() && bytes[field_end] != b'}' {
                    if bytes[field_end] == b'{' {
                        return Err(format!(
                            "invalid GraphRAG format string: unmatched `{{` before field fragment \
                             `{}`",
                            &template[field_start..field_end]
                        ));
                    }
                    field_end += 1;
                }
                if field_end == bytes.len() {
                    return Err("invalid GraphRAG format string: unmatched `{`".to_owned());
                }

                let field = &template[field_start..field_end];
                if !is_simple_format_field(field) {
                    return Err(format!(
                        "unsupported GraphRAG format field `{{{field}}}`; expected a simple \
                         identifier matching [A-Za-z_][A-Za-z0-9_]*"
                    ));
                }
                parts.push(GraphRagFormatPart::Field(field.to_owned()));
                index = field_end + 1;
            }
            b'}' if bytes.get(index + 1) == Some(&b'}') => {
                literal.push('}');
                index += 2;
            }
            b'}' => {
                return Err("invalid GraphRAG format string: unmatched `}`".to_owned());
            }
            _ => {
                let character = template[index..]
                    .chars()
                    .next()
                    .ok_or_else(|| "invalid UTF-8 character boundary".to_owned())?;
                literal.push(character);
                index += character.len_utf8();
            }
        }
    }

    push_literal_part(&mut parts, &mut literal);
    Ok(parts)
}

fn push_literal_part(parts: &mut Vec<GraphRagFormatPart>, literal: &mut String) {
    if !literal.is_empty() {
        parts.push(GraphRagFormatPart::Literal(std::mem::take(literal)));
    }
}

fn is_simple_format_field(field: &str) -> bool {
    let mut bytes = field.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn normalized_default(kind: PromptKind) -> String {
        kind.default_template()
            .replace("\r\n", "\n")
            .replace('\r', "\n")
    }

    #[tokio::test]
    async fn test_should_load_explicit_prompt_with_source() {
        let project = TempDir::new().expect("project");
        let prompts = project.path().join("prompts");
        tokio::fs::create_dir(&prompts).await.expect("prompts");
        tokio::fs::write(
            prompts.join("extract_graph.txt"),
            "Project {{ input_text }}",
        )
        .await
        .expect("project prompt");
        let path = project.path().join("custom.txt");
        tokio::fs::write(&path, "Explicit {{ input_text }}")
            .await
            .expect("explicit prompt");

        let template = PromptRepository::new(project.path())
            .load(PromptKind::ExtractGraph, Some(Path::new("custom.txt")))
            .await
            .expect("explicit prompt should load");

        assert_eq!(template.content(), "Explicit {{ input_text }}");
        assert_eq!(template.source(), &PromptSource::Explicit(path));
        assert_eq!(template.kind(), PromptKind::ExtractGraph);
    }

    #[tokio::test]
    async fn test_should_load_canonical_project_file_without_configured_path() {
        let project = TempDir::new().expect("project");
        let prompts = project.path().join("prompts");
        tokio::fs::create_dir(&prompts).await.expect("prompts");
        tokio::fs::write(
            prompts.join("extract_graph.txt"),
            "Project {{ input_text }}",
        )
        .await
        .expect("project prompt");

        let template = PromptRepository::new(project.path())
            .load(PromptKind::ExtractGraph, None)
            .await
            .expect("project prompt should load");

        assert_eq!(template.content(), "Project {{ input_text }}");
        assert_eq!(
            template.source(),
            &PromptSource::Project(prompts.join("extract_graph.txt"))
        );
    }

    #[tokio::test]
    async fn test_should_load_inline_configured_prompt_without_treating_it_as_a_path() {
        let project = TempDir::new().expect("project");
        let template = PromptRepository::new(project.path())
            .load_configured(
                PromptKind::BasicSearch,
                Some("Inline {{ context_data }} / {{ response_type }}"),
            )
            .await
            .expect("inline prompt");

        assert_eq!(template.source(), &PromptSource::Inline);
        assert_eq!(
            template.content(),
            "Inline {{ context_data }} / {{ response_type }}"
        );
    }

    #[tokio::test]
    async fn test_should_load_builtin_prompt_with_source() {
        let project = TempDir::new().expect("project");

        let template = PromptRepository::new(project.path())
            .load(PromptKind::ExtractGraph, None)
            .await
            .expect("built-in prompt should load");

        assert_eq!(
            template.content(),
            normalized_default(PromptKind::ExtractGraph)
        );
        assert_eq!(template.source(), &PromptSource::BuiltIn);
    }

    #[tokio::test]
    async fn test_should_load_graphrag_single_brace_prompt_syntax() {
        let project = TempDir::new().expect("project");
        let path = project.path().join("legacy.txt");
        tokio::fs::write(
            &path,
            "Example: {{\"name\": \"value\"}}\nText: {input_text}",
        )
        .await
        .expect("legacy prompt");

        let template = PromptRepository::new(project.path())
            .load(PromptKind::ExtractGraph, Some(Path::new("legacy.txt")))
            .await
            .expect("GraphRAG prompt should load");
        let rendered = template
            .bind(&serde_json::json!({
                "entity_types": [],
                "input_text": "Alice"
            }))
            .expect("bind GraphRAG prompt")
            .render()
            .expect("render GraphRAG prompt");

        assert_eq!(rendered, "Example: {\"name\": \"value\"}\nText: Alice");
    }

    #[tokio::test]
    async fn test_should_render_graphrag_known_format_fields() {
        let project = TempDir::new().expect("project");
        let template = PromptRepository::new(project.path())
            .load_configured(
                PromptKind::ExtractGraph,
                Some("Text: {input_text}\nTypes: {entity_types}"),
            )
            .await
            .expect("GraphRAG prompt should load");
        let rendered = template
            .bind(&serde_json::json!({
                "entity_types": "person,organization",
                "input_text": "Alice"
            }))
            .expect("bind GraphRAG prompt")
            .render()
            .expect("render GraphRAG prompt");

        assert_eq!(rendered, "Text: Alice\nTypes: person,organization");
    }

    #[tokio::test]
    async fn test_should_render_graphrag_json_and_latex_literal_braces() {
        let project = TempDir::new().expect("project");
        let template = PromptRepository::new(project.path())
            .load_configured(
                PromptKind::ExtractGraph,
                Some("JSON: {{\"name\":\"value\"}}\nLaTeX: \\frac{{a}}{{b}}\nText: {input_text}"),
            )
            .await
            .expect("GraphRAG prompt should load");
        let rendered = template
            .bind(&serde_json::json!({
                "entity_types": [],
                "input_text": "Alice"
            }))
            .expect("bind GraphRAG prompt")
            .render()
            .expect("render GraphRAG prompt");

        assert_eq!(
            rendered,
            "JSON: {\"name\":\"value\"}\nLaTeX: \\frac{a}{b}\nText: Alice"
        );
    }

    #[tokio::test]
    async fn test_should_render_graphrag_escaped_unknown_field_as_literal() {
        let project = TempDir::new().expect("project");
        let template = PromptRepository::new(project.path())
            .load_configured(
                PromptKind::ExtractGraph,
                Some("Literal: {{unknown}}\nText: {input_text}"),
            )
            .await
            .expect("GraphRAG prompt should load");
        let rendered = template
            .bind(&serde_json::json!({
                "entity_types": [],
                "input_text": "Alice"
            }))
            .expect("bind GraphRAG prompt")
            .render()
            .expect("render GraphRAG prompt");

        assert_eq!(rendered, "Literal: {unknown}\nText: Alice");
    }

    #[tokio::test]
    async fn test_should_reject_unknown_graphrag_field_during_extraction_render() {
        let project = TempDir::new().expect("project");
        let path = project.path().join("extract_graph.txt");
        tokio::fs::write(
            &path,
            "Unknown: {unknown}\nText: {input_text}\nTypes: {entity_types}",
        )
        .await
        .expect("GraphRAG prompt");
        let template = PromptRepository::new(project.path())
            .load(
                PromptKind::ExtractGraph,
                Some(Path::new("extract_graph.txt")),
            )
            .await
            .expect("GraphRAG prompt should load");

        let error = template
            .bind(&serde_json::json!({
                "entity_types": "person,organization",
                "input_text": "Alice"
            }))
            .expect("bind extraction context")
            .render()
            .expect_err("unknown GraphRAG field should fail during render");

        match error {
            GraphLoomError::PromptRender {
                kind,
                name,
                prompt_source,
                message,
            } => {
                assert_eq!(kind, "ExtractGraph");
                assert_eq!(name, "extract_graph.txt");
                assert_eq!(prompt_source, PromptSource::Explicit(path).to_string());
                assert!(message.contains("unknown"), "{message}");
            }
            other => panic!("expected PromptRender error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_should_reject_unmatched_graphrag_format_braces() {
        for (template, expected_message) in [
            ("Broken: {\nText: {input_text}", "unmatched `{`"),
            ("Broken: }\nText: {input_text}", "unmatched `}`"),
        ] {
            let project = TempDir::new().expect("project");
            let error = PromptRepository::new(project.path())
                .load_configured(PromptKind::ExtractGraph, Some(template))
                .await
                .expect_err("unmatched GraphRAG brace should fail while loading");

            match error {
                GraphLoomError::PromptRender {
                    kind,
                    name,
                    prompt_source,
                    message,
                } => {
                    assert_eq!(kind, "ExtractGraph");
                    assert_eq!(name, "extract_graph.txt");
                    assert_eq!(prompt_source, PromptSource::Inline.to_string());
                    assert!(message.contains(expected_message), "{message}");
                }
                other => panic!("expected PromptRender error, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn test_should_reject_unsupported_graphrag_format_fields() {
        for field in [
            "field!r",
            "field:>10",
            "object.name",
            "items[0]",
            "% syntax %",
            "0",
            "",
        ] {
            let project = TempDir::new().expect("project");
            let content = format!("Unsupported: {{{field}}}\nText: {{input_text}}");
            let error = PromptRepository::new(project.path())
                .load_configured(PromptKind::ExtractGraph, Some(&content))
                .await
                .expect_err("complex GraphRAG field should fail while loading");

            match error {
                GraphLoomError::PromptRender { message, .. } => {
                    assert!(message.contains(&format!("{{{field}}}")), "{message}");
                }
                other => panic!("expected PromptRender error, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn test_should_preserve_native_tera_prompt_mode() {
        let project = TempDir::new().expect("project");
        let template = PromptRepository::new(project.path())
            .load_configured(
                PromptKind::ExtractGraph,
                Some("Text: {{ input_text }}\nLiteral single brace: {not_python_mode}"),
            )
            .await
            .expect("native Tera prompt should load");
        let rendered = template
            .bind(&serde_json::json!({
                "entity_types": [],
                "input_text": "Alice"
            }))
            .expect("bind native Tera prompt")
            .render()
            .expect("render native Tera prompt");

        assert_eq!(
            rendered,
            "Text: Alice\nLiteral single brace: {not_python_mode}"
        );
    }

    #[tokio::test]
    async fn test_should_reject_invalid_tera_template_when_loading() {
        let project = TempDir::new().expect("project");
        let configured_path = Path::new("prompts").join("extract_graph.txt");
        let prompts = project.path().join("prompts");
        tokio::fs::create_dir(&prompts).await.expect("prompts");
        let path = project.path().join(&configured_path);
        tokio::fs::write(&path, "{% if enabled %}")
            .await
            .expect("invalid project prompt");

        let error = PromptRepository::new(project.path())
            .load(PromptKind::ExtractGraph, Some(&configured_path))
            .await
            .expect_err("invalid configured template should fail while loading");

        match error {
            GraphLoomError::PromptRender {
                kind,
                name,
                prompt_source,
                message,
            } => {
                assert_eq!(kind, "ExtractGraph");
                assert_eq!(name, "extract_graph.txt");
                assert_eq!(
                    prompt_source,
                    PromptSource::Explicit(path.clone()).to_string()
                );
                assert!(!message.is_empty());
            }
            other => panic!("expected PromptRender error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_should_report_explicit_path_for_template_compile_error() {
        let project = TempDir::new().expect("project");
        let path = project.path().join("invalid.txt");
        tokio::fs::write(&path, "{% if enabled %}")
            .await
            .expect("invalid explicit prompt");

        let error = PromptRepository::new(project.path())
            .load(PromptKind::ExtractGraph, Some(Path::new("invalid.txt")))
            .await
            .expect_err("invalid explicit template should fail while loading");

        match error {
            GraphLoomError::PromptRender {
                kind,
                name,
                prompt_source,
                message,
            } => {
                assert_eq!(kind, "ExtractGraph");
                assert_eq!(name, "extract_graph.txt");
                assert_eq!(
                    prompt_source,
                    PromptSource::Explicit(path.clone()).to_string()
                );
                assert!(!message.is_empty());
            }
            other => panic!("expected PromptRender error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_should_report_prompt_identity_and_path_for_load_error() {
        let project = TempDir::new().expect("project");
        let path = project.path().join("missing.txt");

        let error = PromptRepository::new(project.path())
            .load(PromptKind::ExtractGraph, Some(Path::new("missing.txt")))
            .await
            .expect_err("missing configured template should fail");
        let message = error.to_string();

        assert!(message.contains("ExtractGraph"));
        assert!(message.contains("extract_graph.txt"));
        match error {
            GraphLoomError::PromptLoad {
                kind,
                name,
                path: actual_path,
                ..
            } => {
                assert_eq!(kind, "ExtractGraph");
                assert_eq!(name, "extract_graph.txt");
                assert_eq!(actual_path, path);
            }
            other => panic!("expected PromptLoad error, got {other:?}"),
        }
    }

    #[test]
    fn test_should_compile_all_builtin_prompt_templates() {
        for kind in PromptKind::all() {
            PromptTemplate::try_new(*kind, kind.default_template(), PromptSource::BuiltIn)
                .unwrap_or_else(|error| panic!("{} failed to compile: {error}", kind.filename()));
        }
    }
}
