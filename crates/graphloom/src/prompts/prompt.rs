//! First-class prompt templates, bound contexts, and Tera rendering.

use std::{error::Error, fmt, path::PathBuf, sync::Arc};

use serde::Serialize;
use tera::{Context, Tera};

use super::PromptKind;
use crate::{GraphLoomError, Result};

/// Origin of a loaded prompt template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PromptSource {
    /// Template embedded in the `GraphLoom` binary.
    BuiltIn,
    /// Override selected explicitly by configuration.
    Explicit(PathBuf),
    /// Canonical project prompt discovered under `prompts/`.
    Project(PathBuf),
    /// Prompt content supplied directly in configuration.
    Inline,
}

impl fmt::Display for PromptSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BuiltIn => formatter.write_str("built-in defaults"),
            Self::Explicit(path) => write!(formatter, "explicit path {}", path.display()),
            Self::Project(path) => write!(formatter, "project path {}", path.display()),
            Self::Inline => formatter.write_str("inline configuration"),
        }
    }
}

/// An unbound prompt template loaded from a project repository or built-in defaults.
#[derive(Clone, Debug)]
pub(crate) struct PromptTemplate {
    kind: PromptKind,
    content: Arc<str>,
    source: PromptSource,
    template_name: Arc<str>,
    tera: Arc<Tera>,
}

impl PromptTemplate {
    pub(super) fn try_new(
        kind: PromptKind,
        content: impl Into<Arc<str>>,
        source: PromptSource,
    ) -> Result<Self> {
        let content = normalize_python_newlines(content.into());
        let template_name: Arc<str> = kind.filename().into();
        let mut tera = Tera::default();
        tera.add_raw_template(template_name.as_ref(), content.as_ref())
            .map_err(|error| prompt_render_error(kind, &source, tera_error_message(&error)))?;

        Ok(Self {
            kind,
            content,
            source,
            template_name,
            tera: Arc::new(tera),
        })
    }

    /// Return the prompt kind.
    pub(crate) const fn kind(&self) -> PromptKind {
        self.kind
    }

    /// Return the Tera template source text.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "template source access is retained for future graphloom prompt tuning and \
                      evaluation"
        )
    )]
    pub(crate) fn content(&self) -> &str {
        &self.content
    }

    /// Return where the template was loaded from.
    pub(crate) const fn source(&self) -> &PromptSource {
        &self.source
    }

    /// Bind a typed context without rendering the template.
    pub(crate) fn bind<T>(&self, values: &T) -> Result<Prompt>
    where
        T: Serialize,
    {
        let prompt = Prompt {
            kind: self.kind(),
            template_name: Arc::clone(&self.template_name),
            tera: Arc::clone(&self.tera),
            context: Context::new(),
            source: self.source().clone(),
        }
        .with_context(values)?;
        for variable in self.kind().variables() {
            if !prompt.context.contains_key(variable) {
                return Err(self.render_error(format!(
                    "missing required prompt context value `{variable}`"
                )));
            }
        }
        Ok(prompt)
    }

    fn render_error(&self, message: String) -> GraphLoomError {
        prompt_render_error(self.kind, &self.source, message)
    }
}

/// A prompt template paired with its render context.
#[derive(Clone, Debug)]
pub(crate) struct Prompt {
    kind: PromptKind,
    template_name: Arc<str>,
    tera: Arc<Tera>,
    context: Context,
    source: PromptSource,
}

impl Prompt {
    /// Extend the prompt with fields from another typed context.
    pub(crate) fn with_context<T>(mut self, values: &T) -> Result<Self>
    where
        T: Serialize,
    {
        let context = Context::from_serialize(values).map_err(|error| {
            prompt_render_error(self.kind, &self.source, tera_error_message(&error))
        })?;
        self.context.extend(context);
        Ok(self)
    }

    /// Insert one serialized value into the prompt context.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "first-class Prompt context extension API is reserved for graphloom consumers"
        )
    )]
    pub(crate) fn with_context_value<T>(mut self, key: &str, value: &T) -> Result<Self>
    where
        T: Serialize,
    {
        self.context.try_insert(key, value).map_err(|error| {
            prompt_render_error(self.kind, &self.source, tera_error_message(&error))
        })?;
        Ok(self)
    }

    /// Render the precompiled Tera template with this prompt's context.
    pub(crate) fn render(&self) -> Result<String> {
        self.tera
            .render(&self.template_name, &self.context)
            .map_err(|error| {
                prompt_render_error(self.kind, &self.source, tera_error_message(&error))
            })
    }
}

fn normalize_python_newlines(content: Arc<str>) -> Arc<str> {
    if content.contains('\r') {
        Arc::from(content.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        content
    }
}

fn prompt_render_error(kind: PromptKind, source: &PromptSource, message: String) -> GraphLoomError {
    GraphLoomError::PromptRender {
        kind: kind.name(),
        name: kind.filename(),
        prompt_source: source.to_string(),
        message,
    }
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

    use super::*;

    #[derive(Debug, Serialize)]
    struct NameContext<'a> {
        name: &'a str,
        input_text: &'a str,
        entity_types: &'a [&'a str],
    }

    #[derive(Debug, Serialize)]
    struct GreetingContext<'a> {
        greeting: &'a str,
    }

    #[derive(Debug, Serialize)]
    struct GraphContext<'a> {
        input_text: &'a str,
        entity_types: &'a [&'a str],
    }

    fn template(content: &str) -> PromptTemplate {
        PromptTemplate::try_new(
            PromptKind::ExtractGraph,
            content.to_owned(),
            PromptSource::BuiltIn,
        )
        .expect("template should compile")
    }

    fn prepare_prompt(template: &PromptTemplate) -> Result<Prompt> {
        template.bind(&name_context("GraphLoom"))
    }

    fn name_context(name: &str) -> NameContext<'_> {
        NameContext {
            name,
            input_text: "",
            entity_types: &[],
        }
    }

    #[test]
    fn test_should_bind_typed_context_and_render_prompt() {
        let template = template("Hello {{ name }}");
        let prompt = prepare_prompt(&template).expect("prompt should be prepared");
        let rendered = prompt.render().expect("prompt should render");

        assert_eq!(rendered, "Hello GraphLoom");
    }

    #[test]
    fn test_should_normalize_prompt_newlines_like_python_text_mode() {
        let rendered = template("Hello\r\n{{ name }}\rGoodbye\n")
            .bind(&name_context("GraphLoom"))
            .expect("context should bind")
            .render()
            .expect("prompt should render");

        assert_eq!(rendered, "Hello\nGraphLoom\nGoodbye\n");
    }

    #[test]
    fn test_should_extend_prompt_context() {
        let rendered = template("{{ greeting }} {{ name }}{{ punctuation }}")
            .bind(&name_context("GraphLoom"))
            .expect("context should bind")
            .with_context(&GreetingContext { greeting: "Hello" })
            .expect("context should be extended")
            .with_context_value("punctuation", &"!")
            .expect("context value should be added")
            .render()
            .expect("prompt should render");

        assert_eq!(rendered, "Hello GraphLoom!");
    }

    #[test]
    fn test_should_overwrite_context_values_when_extending_prompt() {
        let rendered = template("Hello {{ name }}")
            .bind(&name_context("before"))
            .expect("context should bind")
            .with_context(&name_context("after"))
            .expect("context should be extended")
            .render()
            .expect("prompt should render");

        assert_eq!(rendered, "Hello after");
    }

    #[test]
    fn test_should_render_one_compiled_template_with_multiple_contexts() {
        let template = template("Hello {{ name }}");

        let first = template
            .bind(&name_context("A"))
            .expect("first context should bind")
            .render()
            .expect("first prompt should render");
        let second = template
            .bind(&name_context("B"))
            .expect("second context should bind")
            .render()
            .expect("second prompt should render");

        assert_eq!(first, "Hello A");
        assert_eq!(second, "Hello B");
    }

    #[test]
    fn test_prompt_template_should_be_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<PromptTemplate>();
    }

    #[test]
    fn test_should_render_tera_filters() {
        let rendered = template("{{ input_text | trim }}\n{{ entity_types | join(sep=\",\") }}")
            .bind(&GraphContext {
                input_text: "  Alice met Bob.  ",
                entity_types: &["person", "organization"],
            })
            .expect("context should bind")
            .render()
            .expect("filters should render");

        assert_eq!(rendered, "Alice met Bob.\nperson,organization");
    }

    #[test]
    fn test_should_render_tera_condition_and_loop() {
        let rendered = template(
            "{% if enabled %}{% for item in items %}{{ item }}{% if not loop.last %},{% endif \
             %}{% endfor %}{% endif %}",
        )
        .bind(&serde_json::json!({
            "enabled": true,
            "items": ["a", "b"],
            "input_text": "",
            "entity_types": [],
        }))
        .expect("context should bind")
        .render()
        .expect("control statements should render");

        assert_eq!(rendered, "a,b");
    }

    #[test]
    fn test_should_reject_missing_tera_variable() {
        let error = template("{{ missing_variable }}")
            .bind(&serde_json::json!({"input_text": "", "entity_types": []}))
            .expect("empty context should bind")
            .render()
            .expect_err("missing variable should fail");
        let message = error.to_string();

        assert!(message.contains("missing_variable"));
        assert!(message.contains("extract_graph.txt"));
    }

    #[test]
    fn test_should_report_builtin_source_for_template_compile_error() {
        let error = PromptTemplate::try_new(
            PromptKind::ExtractGraph,
            "{% if enabled %}",
            PromptSource::BuiltIn,
        )
        .expect_err("invalid built-in template should fail to compile");
        let message = error.to_string();

        assert!(message.contains("extract_graph.txt"));
        assert!(message.contains("built-in defaults"));
    }

    #[test]
    fn test_should_render_all_builtin_prompt_contracts() {
        for kind in PromptKind::all() {
            let template =
                PromptTemplate::try_new(*kind, kind.default_template(), PromptSource::BuiltIn)
                    .expect("built-in template should compile");
            let values = match kind {
                PromptKind::ExtractGraph => serde_json::json!({
                    "entity_types": "person",
                    "input_text": "Alice met Bob.",
                }),
                PromptKind::SummarizeDescriptions => serde_json::json!({
                    "entity_name": "Alice",
                    "description_list": "[\"Person\"]",
                    "max_length": 100,
                }),
                PromptKind::ExtractClaims => serde_json::json!({
                    "entity_specs": ["person"],
                    "claim_description": "claims",
                    "input_text": "Alice reported Bob.",
                }),
                PromptKind::CommunityReportGraph | PromptKind::CommunityReportText => {
                    serde_json::json!({
                        "input_text": "Entities and relationships",
                        "max_report_length": 2_000,
                    })
                }
                PromptKind::BasicSearch | PromptKind::LocalSearch => serde_json::json!({
                    "context_data": "id|text\n0|Alice met Bob.\n",
                    "response_type": "Multiple Paragraphs",
                }),
                PromptKind::DriftSearch => serde_json::json!({
                    "context_data": "Community context",
                    "response_type": "Multiple Paragraphs",
                    "global_query": "What happened?",
                    "followups": "Who was involved?",
                }),
                PromptKind::DriftReduce => serde_json::json!({
                    "context_data": "Partial answers",
                    "response_type": "Multiple Paragraphs",
                }),
                PromptKind::GlobalSearchMap => serde_json::json!({
                    "context_data": "Community reports",
                    "max_length": 2_000,
                }),
                PromptKind::GlobalSearchReduce => serde_json::json!({
                    "report_data": "Analyst reports",
                    "response_type": "Multiple Paragraphs",
                    "max_length": 2_000,
                }),
                PromptKind::GlobalSearchKnowledge => serde_json::json!({}),
                PromptKind::QuestionGeneration => serde_json::json!({
                    "question_count": 5,
                    "context_data": "Conversation history",
                }),
            };

            let rendered = template
                .bind(&values)
                .and_then(|prompt| prompt.render())
                .unwrap_or_else(|error| panic!("{} failed to render: {error}", kind.filename()));
            assert!(!rendered.trim().is_empty());
        }
    }

    #[test]
    fn test_should_preserve_template_like_text_from_input_value() {
        let input = "{input_text} {{ input_text }} {% if example %}";
        let rendered = template("{{ input_text }}")
            .bind(&serde_json::json!({"input_text": input, "entity_types": []}))
            .expect("context should bind")
            .render()
            .expect("input should render once");

        assert_eq!(rendered, input);
    }
}
