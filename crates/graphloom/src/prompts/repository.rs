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

    /// Load an explicit template, canonical project override, or built-in default.
    pub(crate) async fn load(
        &self,
        kind: PromptKind,
        explicit_path: Option<&Path>,
    ) -> Result<PromptTemplate> {
        if let Some(path) = explicit_path {
            let path = self.resolve(path);
            return load_file(kind, path, PromptSource::Explicit).await;
        }

        let canonical = self.project_root.join("prompts").join(kind.filename());
        if path_exists(&canonical).await? {
            return load_file(kind, canonical, PromptSource::ProjectOverride).await;
        }

        Ok(PromptTemplate::new(
            kind,
            kind.default_template(),
            PromptSource::BuiltIn,
        ))
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
                path: path.clone(),
                source: error,
            })?;
    let template = PromptTemplate::new(kind, content, source(path.clone()));
    reject_legacy_single_brace_syntax(kind, template.content(), &path)?;
    Ok(template)
}

async fn path_exists(path: &Path) -> Result<bool> {
    tokio::fs::try_exists(path)
        .await
        .map_err(|source| GraphLoomError::PromptLoad {
            path: path.to_path_buf(),
            source,
        })
}

fn reject_legacy_single_brace_syntax(kind: PromptKind, template: &str, path: &Path) -> Result<()> {
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
                    prompt_source: path.display().to_string(),
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
    async fn test_should_load_project_override_with_source() {
        let project = TempDir::new().expect("project");
        let prompts = project.path().join("prompts");
        tokio::fs::create_dir(&prompts).await.expect("prompts");
        let path = prompts.join("extract_graph.txt");
        tokio::fs::write(&path, "Project {{ input_text }}")
            .await
            .expect("project prompt");

        let template = PromptRepository::new(project.path())
            .load(PromptKind::ExtractGraph, None)
            .await
            .expect("project prompt should load");

        assert_eq!(template.content(), "Project {{ input_text }}");
        assert_eq!(template.source(), &PromptSource::ProjectOverride(path));
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
            PromptKind::ExtractGraph.default_template()
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
}
