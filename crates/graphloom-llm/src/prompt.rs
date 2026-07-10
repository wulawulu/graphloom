//! Tera prompt loading and rendering.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use tera::{Context, Tera};

use crate::{LlmError, Result};

/// Built-in prompt identifiers required by Phase 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefaultPrompt {
    /// Entity and relationship extraction.
    ExtractGraph,
    /// Extraction continuation prompt.
    ExtractGraphContinue,
    /// Extraction loop check prompt.
    ExtractGraphLoop,
    /// Entity/relationship description summarization.
    SummarizeDescriptions,
    /// Claim/covariate extraction.
    ExtractClaims,
    /// Claim continuation prompt.
    ExtractClaimsContinue,
    /// Claim loop check prompt.
    ExtractClaimsLoop,
    /// Community report generation.
    CommunityReport,
}

impl DefaultPrompt {
    /// Return the canonical template filename.
    #[must_use]
    pub fn filename(self) -> &'static str {
        match self {
            Self::ExtractGraph => "extract_graph.txt",
            Self::ExtractGraphContinue => "extract_graph_continue.txt",
            Self::ExtractGraphLoop => "extract_graph_loop.txt",
            Self::SummarizeDescriptions => "summarize_descriptions.txt",
            Self::ExtractClaims => "extract_claims.txt",
            Self::ExtractClaimsContinue => "extract_claims_continue.txt",
            Self::ExtractClaimsLoop => "extract_claims_loop.txt",
            Self::CommunityReport => "community_report.txt",
        }
    }

    /// Return the canonical built-in template.
    #[must_use]
    pub const fn template(self) -> &'static str {
        match self {
            Self::ExtractGraph => include_str!("prompts/extract_graph.txt"),
            Self::ExtractGraphContinue => {
                "MANY entities and relationships were missed in the last extraction. Remember to \
                 ONLY emit entities that match any of the previously extracted types. Add them \
                 below using the same format:\n"
            }
            Self::ExtractGraphLoop => {
                "It appears some entities and relationships may have still been missed. Answer Y \
                 if there are still entities or relationships that need to be added, or N if there \
                 are none. Please answer with a single letter Y or N.\n"
            }
            Self::SummarizeDescriptions => {
                include_str!("prompts/summarize_descriptions.txt")
            }
            Self::ExtractClaims => include_str!("prompts/extract_claims.txt"),
            Self::ExtractClaimsContinue => {
                "MANY entities were missed in the last extraction.  Add them below using the same \
                 format:\n"
            }
            Self::ExtractClaimsLoop => {
                "It appears some entities may have still been missed. Answer Y if there are still \
                 entities that need to be added, or N if there are none. Please answer with a \
                 single letter Y or N.\n"
            }
            Self::CommunityReport => include_str!("prompts/community_report.txt"),
        }
    }
}

/// Prompt loader with GraphRAG-compatible precedence.
#[derive(Debug, Clone)]
pub struct PromptLoader {
    project_root: PathBuf,
}

impl PromptLoader {
    /// Create a prompt loader rooted at a `GraphRAG` project directory.
    #[must_use]
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
        }
    }

    /// Render a prompt using explicit path, project override, then built-in default.
    ///
    /// # Errors
    ///
    /// Returns an error when loading or rendering fails. Missing variables are
    /// not substituted with empty strings.
    pub async fn render<T>(
        &self,
        prompt: DefaultPrompt,
        explicit_path: Option<&Path>,
        values: &T,
    ) -> Result<String>
    where
        T: Serialize,
    {
        let name = prompt.filename();
        let template = self.load_template(prompt, explicit_path).await?;
        let mut tera = Tera::default();
        if let Err(source) = tera.add_raw_template(name, &template) {
            return render_graphrag_format(name, &template, values).map_err(|()| {
                LlmError::PromptRender {
                    name: name.to_owned(),
                    source,
                }
            });
        }
        let context = Context::from_serialize(values).map_err(|source| LlmError::PromptRender {
            name: name.to_owned(),
            source,
        })?;
        match tera.render(name, &context) {
            Ok(rendered) => Ok(rendered),
            Err(source) => render_graphrag_format(name, &template, values).map_err(|()| {
                LlmError::PromptRender {
                    name: name.to_owned(),
                    source,
                }
            }),
        }
    }

    async fn load_template(
        &self,
        prompt: DefaultPrompt,
        explicit_path: Option<&Path>,
    ) -> Result<String> {
        if let Some(path) = explicit_path {
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                self.project_root.join(path)
            };
            return read_prompt(&path).await;
        }

        let override_path = self.project_root.join("prompts").join(prompt.filename());
        if tokio::fs::try_exists(&override_path)
            .await
            .map_err(|source| LlmError::PromptIo {
                path: override_path.clone(),
                source,
            })?
        {
            return read_prompt(&override_path).await;
        }

        Ok(prompt.template().to_owned())
    }
}

async fn read_prompt(path: &Path) -> Result<String> {
    tokio::fs::read_to_string(path)
        .await
        .map_err(|source| LlmError::PromptIo {
            path: path.to_path_buf(),
            source,
        })
}

fn render_graphrag_format<T>(
    name: &str,
    template: &str,
    values: &T,
) -> std::result::Result<String, ()>
where
    T: Serialize,
{
    let data = serde_json::to_value(values).map_err(|_| ())?;
    let object = data.as_object().ok_or(())?;
    let chars = template.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(template.len());
    let mut index = 0usize;
    while index < chars.len() {
        match chars[index] {
            '{' if chars.get(index.saturating_add(1)) == Some(&'{') => {
                output.push('{');
                index = index.saturating_add(2);
            }
            '}' if chars.get(index.saturating_add(1)) == Some(&'}') => {
                output.push('}');
                index = index.saturating_add(2);
            }
            '{' => {
                let mut end = index.saturating_add(1);
                while end < chars.len() && chars[end] != '}' {
                    end = end.saturating_add(1);
                }
                if end >= chars.len() {
                    return Err(());
                }
                let key = chars[index.saturating_add(1)..end]
                    .iter()
                    .collect::<String>();
                let value = object.get(key.trim()).ok_or(())?;
                output.push_str(&value_as_prompt_text(value));
                index = end.saturating_add(1);
            }
            '}' => return Err(()),
            ch => {
                output.push(ch);
                index = index.saturating_add(1);
            }
        }
    }
    if output.is_empty() {
        return Err(());
    }
    let _ = name;
    Ok(output)
}

fn value_as_prompt_text(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => String::new(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}
