//! Project config loading and validation.

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::{Path, PathBuf},
};

use graphloom_llm::TiktokenTokenizer;
use graphloom_storage::{FileStorage, Storage};
use regex::Regex;
use serde_json::Value;

use crate::{
    CREATE_BASE_TEXT_UNITS_WORKFLOW, CREATE_COMMUNITY_REPORTS_WORKFLOW,
    EXTRACT_COVARIATES_WORKFLOW, EXTRACT_GRAPH_WORKFLOW, GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
    GraphLoomError, GraphRagConfig, LOAD_INPUT_DOCUMENTS_WORKFLOW, Result, WorkflowRegistry,
    config::{effective_completion_encoding, effective_embedding_encoding},
    project::{LoadedProject, ProjectPaths},
    register_standard_workflows,
};

/// Load a project config from a root directory or concrete config file.
///
/// # Errors
///
/// Returns an error when settings are missing, environment substitution fails,
/// or the config cannot be parsed.
pub async fn load_project_config(root_or_config: impl AsRef<Path>) -> Result<LoadedProject> {
    load_project_config_with_env(root_or_config.as_ref(), &BTreeMap::new()).await
}

/// Load a project config with extra environment values used by tests.
///
/// # Errors
///
/// Returns a load or parse error.
pub async fn load_project_config_with_env(
    root_or_config: &Path,
    process_env: &BTreeMap<String, String>,
) -> Result<LoadedProject> {
    let config_path = find_config_path(root_or_config).await?;
    let root = config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let dotenv = read_dotenv(&root.join(".env")).await?;
    let raw = tokio::fs::read_to_string(&config_path)
        .await
        .map_err(|source| GraphLoomError::Io {
            operation: "read settings",
            path: config_path.clone(),
            source,
        })?;
    let expanded = substitute_env(&raw, &dotenv, process_env)?;
    let mut config = parse_config(&config_path, &expanded)?;
    let paths = ProjectPaths::resolve(&root, &config)?;
    config.vector_store.db_uri = paths.vector_db_uri.to_string_lossy().to_string();
    Ok(LoadedProject {
        root: paths.root.clone(),
        config_path,
        config,
        paths,
    })
}

async fn find_config_path(root_or_config: &Path) -> Result<PathBuf> {
    if root_or_config.is_file() {
        return root_or_config
            .canonicalize()
            .map_err(|source| GraphLoomError::Io {
                operation: "canonicalize settings",
                path: root_or_config.to_path_buf(),
                source,
            });
    }
    if !root_or_config.is_dir() {
        return Err(GraphLoomError::InvalidRoot {
            path: root_or_config.to_path_buf(),
            message: "root must be a directory or settings file".to_owned(),
        });
    }
    for name in ["settings.yaml", "settings.yml", "settings.json"] {
        let path = root_or_config.join(name);
        if tokio::fs::try_exists(&path)
            .await
            .map_err(|source| GraphLoomError::Io {
                operation: "check settings",
                path: path.clone(),
                source,
            })?
        {
            return path.canonicalize().map_err(|source| GraphLoomError::Io {
                operation: "canonicalize settings",
                path,
                source,
            });
        }
    }
    Err(GraphLoomError::MissingSettings {
        root: root_or_config.to_path_buf(),
    })
}

fn parse_config(path: &Path, raw: &str) -> Result<GraphRagConfig> {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("yaml" | "yml") => {
            serde_yaml::from_str(raw).map_err(|source| GraphLoomError::ConfigParse {
                path: path.to_path_buf(),
                source: Box::new(source),
            })
        }
        Some("json") => serde_json::from_str(raw).map_err(|source| GraphLoomError::ConfigParse {
            path: path.to_path_buf(),
            source: Box::new(source),
        }),
        _ => Err(GraphLoomError::UnsupportedConfigFormat {
            path: path.to_path_buf(),
        }),
    }
}

async fn read_dotenv(path: &Path) -> Result<BTreeMap<String, String>> {
    let raw = match tokio::fs::read_to_string(path).await {
        Ok(raw) => raw,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(source) => {
            return Err(GraphLoomError::Io {
                operation: "read .env",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    parse_dotenv(&raw)
}

fn parse_dotenv(raw: &str) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for item in dotenvy::Iter::new(raw.as_bytes()) {
        let (key, value) = item.map_err(|source| GraphLoomError::DotenvParse {
            line: dotenv_error_line(raw, &source),
        })?;
        values.insert(key, value);
    }
    Ok(values)
}

fn dotenv_error_line(raw: &str, error: &dotenvy::Error) -> usize {
    if let dotenvy::Error::LineParse(line, _) = error {
        return raw
            .lines()
            .position(|candidate| candidate == line)
            .map_or(1, |index| index.saturating_add(1));
    }
    1
}

fn substitute_env(
    raw: &str,
    dotenv: &BTreeMap<String, String>,
    test_env: &BTreeMap<String, String>,
) -> Result<String> {
    let mut output = String::with_capacity(raw.len());
    let chars = raw.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] != '$' {
            output.push(chars[index]);
            index = index.saturating_add(1);
            continue;
        }
        let Some(next) = chars.get(index.saturating_add(1)).copied() else {
            output.push('$');
            break;
        };
        if next == '$' {
            output.push('$');
            index = index.saturating_add(2);
            continue;
        }
        let (name, consumed) = if next == '{' {
            let mut end = index.saturating_add(2);
            while end < chars.len() && chars[end] != '}' {
                end = end.saturating_add(1);
            }
            if end >= chars.len() {
                output.push('$');
                index = index.saturating_add(1);
                continue;
            }
            (
                chars[index.saturating_add(2)..end]
                    .iter()
                    .collect::<String>(),
                end.saturating_sub(index).saturating_add(1),
            )
        } else if is_env_start(next) {
            let mut end = index.saturating_add(1);
            while end < chars.len() && is_env_continue(chars[end]) {
                end = end.saturating_add(1);
            }
            (
                chars[index.saturating_add(1)..end]
                    .iter()
                    .collect::<String>(),
                end.saturating_sub(index),
            )
        } else {
            output.push('$');
            index = index.saturating_add(1);
            continue;
        };
        output.push_str(&lookup_env(&name, dotenv, test_env)?);
        index = index.saturating_add(consumed);
    }
    Ok(output)
}

fn is_env_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_env_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn lookup_env(
    name: &str,
    dotenv: &BTreeMap<String, String>,
    test_env: &BTreeMap<String, String>,
) -> Result<String> {
    if let Some(value) = test_env.get(name) {
        return Ok(value.clone());
    }
    if let Ok(value) = env::var(name) {
        return Ok(value);
    }
    dotenv
        .get(name)
        .cloned()
        .ok_or_else(|| GraphLoomError::MissingEnvironmentVariable {
            name: name.to_owned(),
        })
}

/// Validate config values required for standard indexing.
///
/// # Errors
///
/// Returns an error for unsupported or unsafe settings.
pub async fn validate_project(project: &LoadedProject, skip_optional: bool) -> Result<()> {
    validate_required(project)?;
    if skip_optional {
        return Ok(());
    }
    validate_optional(project).await
}

/// Validate an index project before building or dry-running an index.
///
/// # Errors
///
/// Returns an error for unsupported, unsafe, or incomplete indexing settings.
pub async fn validate_index_project(project: &LoadedProject, mode: ValidationMode) -> Result<()> {
    validate_project(project, matches!(mode, ValidationMode::SkipOptional)).await
}

/// Validation depth for index requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationMode {
    /// Validate all preflight checks that can run without model calls or output mutation.
    Full,
    /// Skip optional existence/model/tokenizer checks while retaining safety checks.
    SkipOptional,
}

fn validate_required(project: &LoadedProject) -> Result<()> {
    validate_storage("input", &project.config.input_storage.storage_type)?;
    validate_storage("output", &project.config.output_storage.storage_type)?;
    validate_storage("cache", &project.config.cache.storage.storage_type)?;
    validate_reporting(&project.config.reporting.reporting_type)?;
    validate_input(&project.config.input.input_type)?;
    validate_cache(&project.config.cache.cache_type)?;
    project.paths.validate_destructive_paths()?;
    let mut registry = WorkflowRegistry::new();
    register_standard_workflows(&mut registry);
    registry
        .validate_names(&project.config.workflow_order())
        .map_err(|source| GraphLoomError::RuntimeBuild {
            source: Box::new(source),
        })?;
    Ok(())
}

async fn validate_optional(project: &LoadedProject) -> Result<()> {
    let active = active_workflows(&project.config);
    let completion_models = referenced_completion_models(&project.config, &active);
    for model_id in completion_models {
        let model = require_model(&project.config.completion_models, &model_id)?;
        validate_model(&model_id, model)?;
    }
    if active.contains(GENERATE_TEXT_EMBEDDINGS_WORKFLOW) {
        project
            .config
            .validate_embed_text()
            .map_err(|message| GraphLoomError::InvalidModel {
                model_id: project.config.embed_text.embedding_model_id.clone(),
                message,
            })?;
        let embedding_model_id = project.config.embed_text.embedding_model_id.clone();
        let embedding_model = require_model(&project.config.embedding_models, &embedding_model_id)?;
        validate_model(&embedding_model_id, embedding_model)?;
    }
    validate_chunking_if_needed(project, &active)?;
    for path in project.paths.active_prompt_paths(&project.config, &active) {
        if !tokio::fs::try_exists(&path)
            .await
            .map_err(|source| GraphLoomError::Io {
                operation: "check prompt",
                path: path.clone(),
                source,
            })?
        {
            return Err(GraphLoomError::MissingPrompt { path });
        }
    }
    if active.contains(LOAD_INPUT_DOCUMENTS_WORKFLOW)
        && !tokio::fs::try_exists(&project.paths.input_dir)
            .await
            .map_err(|source| GraphLoomError::Io {
                operation: "check input directory",
                path: project.paths.input_dir.clone(),
                source,
            })?
    {
        return Err(GraphLoomError::MissingInput {
            message: format!(
                "input directory {} does not exist",
                project.paths.input_dir.display()
            ),
        });
    }
    if active.contains(LOAD_INPUT_DOCUMENTS_WORKFLOW) {
        let file_pattern = Regex::new(&project.config.input.file_pattern).map_err(|source| {
            GraphLoomError::InvalidModel {
                model_id: "input.file_pattern".to_owned(),
                message: source.to_string(),
            }
        })?;
        if input_file_count(&project.paths.input_dir, &file_pattern).await? == 0 {
            return Err(GraphLoomError::MissingInput {
                message: "no matching input files found".to_owned(),
            });
        }
    }
    Ok(())
}

fn require_model<'a>(
    models: &'a BTreeMap<String, graphloom_llm::ModelConfig>,
    model_id: &str,
) -> Result<&'a graphloom_llm::ModelConfig> {
    models
        .get(model_id)
        .ok_or_else(|| GraphLoomError::InvalidModel {
            model_id: model_id.to_owned(),
            message: "model is not configured".to_owned(),
        })
}

fn referenced_completion_models(
    config: &GraphRagConfig,
    active: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut models = BTreeSet::new();
    if active.contains(EXTRACT_GRAPH_WORKFLOW) {
        models.insert(config.extract_graph.completion_model_id.clone());
        models.insert(config.summarize_descriptions.completion_model_id.clone());
    }
    if active.contains(EXTRACT_COVARIATES_WORKFLOW) && config.extract_claims.enabled {
        models.insert(config.extract_claims.completion_model_id.clone());
    }
    if active.contains(CREATE_COMMUNITY_REPORTS_WORKFLOW) {
        models.insert(config.community_reports.completion_model_id.clone());
    }
    models
}

fn active_workflows(config: &GraphRagConfig) -> BTreeSet<String> {
    config.workflow_order().into_iter().collect()
}

fn validate_chunking_if_needed(project: &LoadedProject, active: &BTreeSet<String>) -> Result<()> {
    let needs_chunking = [
        CREATE_BASE_TEXT_UNITS_WORKFLOW,
        EXTRACT_GRAPH_WORKFLOW,
        CREATE_COMMUNITY_REPORTS_WORKFLOW,
        GENERATE_TEXT_EMBEDDINGS_WORKFLOW,
    ]
    .iter()
    .any(|workflow| active.contains(*workflow));
    if !needs_chunking {
        return Ok(());
    }
    if project.config.chunking.overlap >= project.config.chunking.size.get() {
        return Err(GraphLoomError::InvalidModel {
            model_id: "chunking".to_owned(),
            message: format!(
                "overlap {} must be smaller than size {}",
                project.config.chunking.overlap, project.config.chunking.size,
            ),
        });
    }
    let mut encodings = BTreeSet::new();
    if active.contains(CREATE_BASE_TEXT_UNITS_WORKFLOW) || active.contains(EXTRACT_GRAPH_WORKFLOW) {
        encodings.insert((
            "chunking.encoding_model".to_owned(),
            project.config.chunking.encoding_model.as_str(),
        ));
    }
    if active.contains(CREATE_COMMUNITY_REPORTS_WORKFLOW) {
        encodings.insert((
            format!(
                "completion_models.{}.encoding_model",
                project.config.community_reports.completion_model_id
            ),
            effective_completion_encoding(
                &project.config,
                &project.config.community_reports.completion_model_id,
            ),
        ));
    }
    if active.contains(GENERATE_TEXT_EMBEDDINGS_WORKFLOW) {
        encodings.insert((
            format!(
                "embedding_models.{}.encoding_model",
                project.config.embed_text.embedding_model_id
            ),
            effective_embedding_encoding(
                &project.config,
                &project.config.embed_text.embedding_model_id,
            ),
        ));
    }
    for (model_id, encoding) in encodings {
        TiktokenTokenizer::new(encoding).map_err(|source| GraphLoomError::InvalidModel {
            model_id,
            message: source.to_string(),
        })?;
    }
    Ok(())
}

fn validate_model(model_id: &str, model: &graphloom_llm::ModelConfig) -> Result<()> {
    model
        .validate_openai_compatible(model_id)
        .map_err(|source| GraphLoomError::InvalidModel {
            model_id: model_id.to_owned(),
            message: source.to_string(),
        })
}

fn validate_storage(kind: &'static str, storage_type: &str) -> Result<()> {
    if storage_type.eq_ignore_ascii_case("file") {
        Ok(())
    } else {
        Err(GraphLoomError::UnsupportedStorage {
            kind,
            storage_type: storage_type.to_owned(),
        })
    }
}

fn validate_reporting(reporting_type: &str) -> Result<()> {
    validate_storage("reporting", reporting_type)
}

fn validate_input(input_type: &str) -> Result<()> {
    if input_type.eq_ignore_ascii_case("text") || input_type.eq_ignore_ascii_case("file") {
        Ok(())
    } else {
        Err(GraphLoomError::UnsupportedInput {
            input_type: input_type.to_owned(),
        })
    }
}

fn validate_cache(cache_type: &str) -> Result<()> {
    if cache_type.eq_ignore_ascii_case("json") || cache_type.eq_ignore_ascii_case("none") {
        Ok(())
    } else {
        Err(GraphLoomError::UnsupportedStorage {
            kind: "cache",
            storage_type: cache_type.to_owned(),
        })
    }
}

async fn input_file_count(root: &Path, file_pattern: &Regex) -> Result<usize> {
    let storage = FileStorage::existing(root)?;
    let files = storage.list("").await?;
    Ok(files
        .iter()
        .filter(|name| file_pattern.is_match(name))
        .count())
}

/// Return a redacted summary of a config.
///
/// # Errors
///
/// Returns an error if serialization fails.
pub fn redacted_config_summary(config: &GraphRagConfig) -> Result<Value> {
    let mut value = serde_json::to_value(config).map_err(|source| GraphLoomError::ConfigParse {
        path: PathBuf::from("<config>"),
        source: Box::new(source),
    })?;
    crate::error::redact_json(&mut value);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::TempDir;

    use super::*;
    use crate::cli::{InitArgs, init_project};

    #[test]
    fn test_should_parse_supported_dotenv_syntax() {
        let values = parse_dotenv(
            "\n# comment\nPLAIN=value\nDOUBLE=\"quoted value\"\nSINGLE='single \
             value'\nWITH_EQUALS=left=right\n",
        )
        .expect("dotenv syntax should parse");

        assert_eq!(values.get("PLAIN").map(String::as_str), Some("value"));
        assert_eq!(
            values.get("DOUBLE").map(String::as_str),
            Some("quoted value")
        );
        assert_eq!(
            values.get("SINGLE").map(String::as_str),
            Some("single value")
        );
        assert_eq!(
            values.get("WITH_EQUALS").map(String::as_str),
            Some("left=right")
        );
    }

    #[tokio::test]
    async fn test_should_load_initialized_yaml_without_changing_cwd() {
        let tempdir = TempDir::new().expect("tempdir");
        init_project(&InitArgs {
            root: tempdir.path().to_path_buf(),
            model: "custom-chat".to_owned(),
            embedding: "custom-embedding".to_owned(),
            force: false,
        })
        .await
        .expect("init");
        tokio::fs::write(tempdir.path().join("input").join("doc.txt"), "Alice")
            .await
            .expect("input write");
        tokio::fs::write(
            tempdir.path().join(".env"),
            "GRAPHRAG_API_KEY=super-secret-key\n",
        )
        .await
        .expect("dotenv write");

        let cwd = std::env::current_dir().expect("cwd");
        let project = load_project_config(tempdir.path()).await.expect("load");
        assert_eq!(std::env::current_dir().expect("cwd after"), cwd);

        assert_eq!(
            project.config.completion_models["default_completion_model"].model,
            "custom-chat"
        );
        assert_eq!(
            project.config.embedding_models["default_embedding_model"].model,
            "custom-embedding"
        );
        assert_eq!(
            project.config.completion_models["default_completion_model"].provider_type(),
            "openai"
        );
        assert_eq!(
            project.config.completion_models["default_completion_model"].auth_method,
            "api_key"
        );
        assert_eq!(project.config.input.input_type, "text");
        assert_eq!(project.config.input_storage.base_dir, "input");
        assert_eq!(project.config.output_storage.base_dir, "output");
        assert_eq!(project.config.cache.storage.base_dir, "cache");
        assert_eq!(project.config.reporting.base_dir, "logs");
        assert_eq!(project.config.vector_store.vector_size, 3_072);
        assert!(project.config.sections.contains_key("local_search"));
        assert!(project.config.sections.contains_key("global_search"));
        assert!(project.config.sections.contains_key("drift_search"));
        assert!(project.config.sections.contains_key("basic_search"));
        validate_project(&project, false).await.expect("validate");

        let summary = redacted_config_summary(&project.config).expect("summary");
        let text = serde_json::to_string(&summary).expect("summary json");
        assert!(!text.contains("super-secret-key"));
        assert!(text.contains("<redacted>"));
    }

    #[tokio::test]
    async fn test_should_support_yml_json_and_env_precedence() {
        let tempdir = TempDir::new().expect("tempdir");
        tokio::fs::write(
            tempdir.path().join(".env"),
            "GRAPHRAG_API_KEY=from-dotenv\n",
        )
        .await
        .expect("dotenv");
        tokio::fs::write(
            tempdir.path().join("settings.yml"),
            r"
completion_models:
  default_completion_model:
    model_provider: openai
    model: gpt
    auth_method: api_key
    api_key: $GRAPHRAG_API_KEY
embedding_models:
  default_embedding_model:
    model_provider: openai
    model: emb
    auth_method: api_key
    api_key: ${GRAPHRAG_API_KEY}
",
        )
        .await
        .expect("settings");
        let mut env = BTreeMap::new();
        env.insert("GRAPHRAG_API_KEY".to_owned(), "from-env".to_owned());
        let yml = load_project_config_with_env(tempdir.path(), &env)
            .await
            .expect("load yml");
        assert_eq!(
            yml.config.completion_models["default_completion_model"]
                .api_key
                .as_deref(),
            Some("from-env")
        );

        tokio::fs::remove_file(tempdir.path().join("settings.yml"))
            .await
            .expect("remove yml");
        tokio::fs::write(
            tempdir.path().join("settings.json"),
            r#"{
  "completion_models": {
    "default_completion_model": {
      "model_provider": "openai",
      "model": "gpt",
      "auth_method": "api_key",
      "api_key": "${GRAPHRAG_API_KEY}"
    }
  },
  "embedding_models": {
    "default_embedding_model": {
      "model_provider": "openai",
      "model": "emb",
      "auth_method": "api_key",
      "api_key": "$GRAPHRAG_API_KEY"
    }
  }
}"#,
        )
        .await
        .expect("json");
        let json = load_project_config_with_env(tempdir.path(), &BTreeMap::new())
            .await
            .expect("load json");
        assert_eq!(
            json.config.embedding_models["default_embedding_model"]
                .api_key
                .as_deref(),
            Some("from-dotenv")
        );
    }

    #[tokio::test]
    async fn test_should_fail_on_missing_env_and_malformed_dotenv() {
        let tempdir = TempDir::new().expect("tempdir");
        tokio::fs::write(
            tempdir.path().join("settings.yaml"),
            "completion_models:\n  default_completion_model:\n    model: gpt\n    api_key: \
             ${MISSING}\n",
        )
        .await
        .expect("settings");
        let error = load_project_config(tempdir.path())
            .await
            .expect_err("missing env");
        assert!(error.to_string().contains("MISSING"));

        tokio::fs::write(tempdir.path().join(".env"), "not-valid\n")
            .await
            .expect("dotenv");
        let error = load_project_config(tempdir.path())
            .await
            .expect_err("bad dotenv");
        assert!(error.to_string().contains("line 1"));
    }

    #[tokio::test]
    async fn test_should_validate_dry_run_preflight_requirements() {
        let tempdir = TempDir::new().expect("tempdir");
        init_project(&InitArgs {
            root: tempdir.path().to_path_buf(),
            model: "gpt-test".to_owned(),
            embedding: "embed-test".to_owned(),
            force: false,
        })
        .await
        .expect("init");
        tokio::fs::write(tempdir.path().join("input").join("doc.txt"), "Alice")
            .await
            .expect("input");

        let project = load_project_config(tempdir.path()).await.expect("load");
        let error = validate_project(&project, false)
            .await
            .expect_err("placeholder api key should fail");
        assert!(error.to_string().contains("api_key is required"));
        assert!(!error.to_string().contains("<API_KEY>"));
    }

    #[tokio::test]
    async fn test_should_validate_input_file_pattern_with_reader_regex() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut project = initialized_project(tempdir.path()).await;
        tokio::fs::write(tempdir.path().join("input").join("doc.md"), "Alice")
            .await
            .expect("md input");

        project.config.input.file_pattern = ".*\\.md$".to_owned();
        validate_project(&project, false)
            .await
            .expect("md pattern should match");

        project.config.input.file_pattern = ".*\\.txt$".to_owned();
        let error = validate_project(&project, false)
            .await
            .expect_err("txt pattern should not match");
        assert!(error.to_string().contains("no matching input files"));
    }

    #[tokio::test]
    async fn test_should_match_input_pattern_against_storage_logical_path() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut project = initialized_project(tempdir.path()).await;
        tokio::fs::create_dir_all(tempdir.path().join("input").join("subdir"))
            .await
            .expect("subdir");
        tokio::fs::write(
            tempdir
                .path()
                .join("input")
                .join("subdir")
                .join("document.txt"),
            "Alice",
        )
        .await
        .expect("nested input");

        project.config.input.file_pattern = "^subdir/.*\\.txt$".to_owned();
        validate_project(&project, false)
            .await
            .expect("logical path pattern should match");
    }

    #[tokio::test]
    async fn test_should_validate_only_active_workflow_dependencies() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut project = initialized_project(tempdir.path()).await;
        tokio::fs::write(tempdir.path().join("input").join("doc.txt"), "Alice")
            .await
            .expect("input");
        project.config.workflows = vec![
            "load_input_documents".to_owned(),
            "create_base_text_units".to_owned(),
            "create_final_documents".to_owned(),
        ];
        project.config.completion_models.clear();
        project.config.embedding_models.clear();

        validate_project(&project, false)
            .await
            .expect("prefix workflows should not require LLM models");

        project.config.workflows.push("extract_graph".to_owned());
        let error = validate_project(&project, false)
            .await
            .expect_err("extract_graph should require its model");
        assert!(error.to_string().contains("default_completion_model"));
    }

    #[tokio::test]
    async fn test_should_require_embedding_model_only_when_embeddings_are_active() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut project = initialized_project(tempdir.path()).await;
        tokio::fs::write(tempdir.path().join("input").join("doc.txt"), "Alice")
            .await
            .expect("input");
        project.config.workflows = crate::workflows::STEP8_WORKFLOWS
            .iter()
            .map(|workflow| (*workflow).to_owned())
            .collect();
        project.config.embedding_models.clear();
        validate_project(&project, false)
            .await
            .expect("step8 should not require embedding model");

        project
            .config
            .workflows
            .push("generate_text_embeddings".to_owned());
        let error = validate_project(&project, false)
            .await
            .expect_err("step9 should require embedding model");
        assert!(error.to_string().contains("default_embedding_model"));
    }

    #[tokio::test]
    async fn test_should_ignore_unused_unsupported_completion_models() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut project = initialized_project(tempdir.path()).await;
        tokio::fs::write(tempdir.path().join("input").join("doc.txt"), "Alice")
            .await
            .expect("input");
        let mut unused = project.config.completion_models["default_completion_model"].clone();
        unused.provider_type = "azure".to_owned();
        project
            .config
            .completion_models
            .insert("unused_query_model".to_owned(), unused);

        validate_project(&project, false)
            .await
            .expect("unused unsupported model should not block standard index");
    }

    #[tokio::test]
    async fn test_should_require_claims_model_when_claims_enabled() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut project = initialized_project(tempdir.path()).await;
        tokio::fs::write(tempdir.path().join("input").join("doc.txt"), "Alice")
            .await
            .expect("input");
        project.config.extract_claims.enabled = true;
        project.config.extract_claims.completion_model_id = "missing_claim_model".to_owned();

        let error = validate_project(&project, false)
            .await
            .expect_err("missing claims model should fail");
        assert!(error.to_string().contains("missing_claim_model"));
    }

    #[tokio::test]
    async fn test_should_not_require_summarize_dependencies_for_finalize_graph_only() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut project = initialized_project(tempdir.path()).await;
        project.config.workflows = vec!["finalize_graph".to_owned()];
        project.config.completion_models.clear();
        project.config.summarize_descriptions.prompt = Some("prompts/missing.txt".to_owned());

        validate_project(&project, false)
            .await
            .expect("finalize_graph should not require summarizer model or prompt");
    }

    #[tokio::test]
    async fn test_should_require_summarize_dependencies_for_extract_graph() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut project = initialized_project(tempdir.path()).await;
        tokio::fs::write(tempdir.path().join("input").join("doc.txt"), "Alice")
            .await
            .expect("input");
        project.config.workflows = vec!["extract_graph".to_owned()];
        project.config.summarize_descriptions.completion_model_id = "missing_summarizer".to_owned();

        let error = validate_project(&project, false)
            .await
            .expect_err("extract_graph should require summarizer model");

        assert!(error.to_string().contains("missing_summarizer"));
    }

    #[tokio::test]
    async fn test_should_validate_effective_completion_encoding_for_community_reports() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut project = initialized_project(tempdir.path()).await;
        project.config.workflows = vec!["create_community_reports".to_owned()];
        project
            .config
            .completion_models
            .get_mut("default_completion_model")
            .expect("model")
            .encoding_model = Some("definitely-not-an-encoding".to_owned());

        let error = validate_project(&project, false)
            .await
            .expect_err("invalid community report encoding should fail");

        assert!(error.to_string().contains("default_completion_model"));
        assert!(error.to_string().contains("definitely-not-an-encoding"));
    }

    #[tokio::test]
    async fn test_should_validate_effective_embedding_encoding_for_text_embeddings() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut project = initialized_project(tempdir.path()).await;
        project.config.workflows = vec!["generate_text_embeddings".to_owned()];
        project
            .config
            .embedding_models
            .get_mut("default_embedding_model")
            .expect("model")
            .encoding_model = Some("definitely-not-an-encoding".to_owned());

        let error = validate_project(&project, false)
            .await
            .expect_err("invalid embedding encoding should fail");

        assert!(error.to_string().contains("default_embedding_model"));
        assert!(error.to_string().contains("definitely-not-an-encoding"));
    }

    async fn initialized_project(root: &std::path::Path) -> LoadedProject {
        init_project(&InitArgs {
            root: root.to_path_buf(),
            model: "gpt-test".to_owned(),
            embedding: "embed-test".to_owned(),
            force: false,
        })
        .await
        .expect("init");
        tokio::fs::write(root.join(".env"), "GRAPHRAG_API_KEY=test-key\n")
            .await
            .expect("dotenv");
        load_project_config(root).await.expect("load")
    }
}
