//! Project-scoped prompt template loading.

use std::path::{Path, PathBuf};

use super::{PromptKind, PromptSource, PromptTemplate};
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

        build_template(kind, kind.default_template(), PromptSource::BuiltIn)
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
    let content = content.into();
    reject_legacy_single_brace_syntax(kind, &content, &source)?;
    PromptTemplate::try_new(kind, content, source)
}

fn reject_legacy_single_brace_syntax(
    kind: PromptKind,
    template: &str,
    source: &PromptSource,
) -> Result<()> {
    for variable in kind.variables() {
        let legacy = format!("{{{variable}}}");
        let mut remaining = template;
        while let Some(index) = remaining.find(&legacy) {
            let after = index.saturating_add(legacy.len());
            let preceded_by_brace = index
                .checked_sub(1)
                .and_then(|previous| remaining.as_bytes().get(previous))
                == Some(&b'{');
            let followed_by_brace = remaining.as_bytes().get(after) == Some(&b'}');
            if !preceded_by_brace && !followed_by_brace {
                return Err(GraphLoomError::PromptRender {
                    kind: kind.name(),
                    name: kind.filename(),
                    prompt_source: source.to_string(),
                    message: format!(
                        "prompt uses unsupported single-brace syntax `{legacy}`; GraphLoom \
                         prompts use Tera syntax `{{{{ {variable} }}}}`"
                    ),
                });
            }
            remaining = &remaining[after..];
        }
    }
    Ok(())
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
    async fn test_should_ignore_canonical_project_file_without_configured_path() {
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
            .expect("built-in prompt should load");

        assert_eq!(
            template.content(),
            normalized_default(PromptKind::ExtractGraph)
        );
        assert_eq!(template.source(), &PromptSource::BuiltIn);
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
    async fn test_should_reject_legacy_single_brace_prompt_syntax() {
        let project = TempDir::new().expect("project");
        let path = project.path().join("legacy.txt");
        tokio::fs::write(&path, "Text: {input_text}")
            .await
            .expect("legacy prompt");

        let error = PromptRepository::new(project.path())
            .load(PromptKind::ExtractGraph, Some(Path::new("legacy.txt")))
            .await
            .expect_err("legacy syntax should fail");
        let message = error.to_string();

        assert!(message.contains("{input_text}"));
        assert!(message.contains("{{ input_text }}"));
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
