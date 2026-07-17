//! Project config loading and validation.

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use graphloom_llm::TiktokenTokenizer;
use graphloom_storage::{FileStorage, Storage};
use regex::Regex;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::{
    GraphLoomError, GraphRagConfig, IndexWorkflowRegistry, LOAD_INPUT_DOCUMENTS_WORKFLOW, Result,
    project::{LoadedProject, ProjectPaths},
    prompts::PromptRepository,
    register_standard_index_workflows,
    runtime::{DefaultModelFactory, ModelFactory, validate_model_connectivity},
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
#[cfg(test)]
async fn validate_project(project: &LoadedProject, skip_optional: bool) -> Result<()> {
    validate_required(project)?;
    let pipeline = build_index_pipeline(&project.config)?;
    let requirements = pipeline.requirements(&project.config)?;
    if requirements.requires_vector_store() {
        project.paths.validate_vector_path_safety()?;
    }
    if skip_optional {
        return Ok(());
    }
    validate_optional(project, &requirements, true).await
}

/// Validate an index project before building or dry-running an index.
///
/// # Errors
///
/// Returns an error for unsupported, unsafe, or incomplete indexing settings.
pub async fn validate_index_project(project: &LoadedProject, mode: ValidationMode) -> Result<()> {
    validate_index_project_with_factory(project, mode, &DefaultModelFactory).await
}

pub(crate) async fn validate_index_project_with_factory(
    project: &LoadedProject,
    mode: ValidationMode,
    model_factory: &dyn ModelFactory,
) -> Result<()> {
    validate_required(project)?;
    let pipeline = build_index_pipeline(&project.config)?;
    let requirements = pipeline.requirements(&project.config)?;
    if requirements.requires_vector_store() {
        project.paths.validate_vector_path_safety()?;
    }
    if matches!(mode, ValidationMode::SkipOptional) {
        return Ok(());
    }
    let ValidationMode::Full { cache_enabled } = mode else {
        return Ok(());
    };
    validate_optional(project, &requirements, cache_enabled).await?;
    validate_model_connectivity(&project.config, &requirements, model_factory).await
}

/// Validation depth for index requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationMode {
    /// Validate all preflight checks, including uncached required-model connectivity.
    Full {
        /// Whether this run would construct configured cache storage.
        cache_enabled: bool,
    },
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
    project.paths.validate_output_path_safety()?;
    Ok(())
}

async fn validate_optional(
    project: &LoadedProject,
    requirements: &crate::IndexWorkflowRequirements,
    cache_enabled: bool,
) -> Result<()> {
    let active = active_workflows(&project.config);
    if requirements.embedding_models().next().is_some() {
        project
            .config
            .validate_embed_text()
            .map_err(|message| GraphLoomError::InvalidModel {
                model_id: project.config.embed_text.embedding_model_id.clone(),
                message,
            })?;
    }
    if requirements.requires_vector_store() {
        validate_active_vector_schemas(&project.config)?;
    }
    validate_chunking_requirements(&project.config, requirements)?;
    validate_prompt_requirements(project, requirements).await?;
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
    validate_required_models(&project.config, requirements)?;
    validate_runtime_path_writability(project, requirements, cache_enabled).await?;
    Ok(())
}

fn validate_active_vector_schemas(config: &crate::GraphRagConfig) -> Result<()> {
    config
        .vector_store
        .validate()
        .map_err(|source| GraphLoomError::RuntimeBuild {
            source: Box::new(source),
        })?;
    for embedding_name in &config.embed_text.names {
        config
            .vector_store
            .schema_for(embedding_name)
            .validate()
            .map_err(|source| GraphLoomError::RuntimeBuild {
                source: Box::new(source),
            })?;
    }
    Ok(())
}

async fn validate_prompt_requirements(
    project: &LoadedProject,
    requirements: &crate::IndexWorkflowRequirements,
) -> Result<()> {
    let repository = PromptRepository::new(&project.root);
    for requirement in requirements.prompt_requirements() {
        repository
            .load(
                requirement.kind,
                requirement.configured_path.as_deref().map(Path::new),
            )
            .await?;
    }
    Ok(())
}

async fn validate_runtime_path_writability(
    project: &LoadedProject,
    requirements: &crate::IndexWorkflowRequirements,
    cache_enabled: bool,
) -> Result<()> {
    probe_directory_writable(&project.paths.output_dir, "output").await?;
    probe_directory_writable(&project.paths.reporting_dir, "logs").await?;
    if cache_enabled && project.config.cache.cache_type.eq_ignore_ascii_case("json") {
        probe_directory_writable(&project.paths.cache_dir, "cache").await?;
    }
    if requirements.requires_vector_store() {
        probe_directory_writable(&project.paths.vector_db_uri, "vector DB").await?;
    }
    Ok(())
}

async fn probe_directory_writable(directory: &Path, label: &'static str) -> Result<()> {
    let probe_root = writable_probe_root(directory, label).await?;
    let probe = probe_root.join(format!(".graphloom-write-probe-{}", Uuid::new_v4()));
    let write_result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .await
            .map_err(|source| probe_io_error("create write probe", &probe, source))?;
        file.write_all(b"graphloom")
            .await
            .map_err(|source| probe_io_error("write probe", &probe, source))?;
        file.flush()
            .await
            .map_err(|source| probe_io_error("flush write probe", &probe, source))?;
        drop(file);
        tokio::fs::remove_file(&probe)
            .await
            .map_err(|source| probe_io_error("remove write probe", &probe, source))?;
        Ok(())
    }
    .await;

    if write_result.is_err() {
        let _ = tokio::fs::remove_file(&probe).await;
    }
    write_result
}

async fn writable_probe_root(directory: &Path, label: &'static str) -> Result<PathBuf> {
    match tokio::fs::metadata(directory).await {
        Ok(metadata) if metadata.is_dir() => Ok(directory.to_path_buf()),
        Ok(_) => Err(GraphLoomError::RuntimeBuild {
            source: Box::new(std::io::Error::new(
                ErrorKind::AlreadyExists,
                format!("{label} path {} is not a directory", directory.display()),
            )),
        }),
        Err(source) if source.kind() == ErrorKind::NotFound => {
            existing_ancestor(directory, label).await
        }
        Err(source) => Err(probe_io_error(
            "inspect writable directory",
            directory,
            source,
        )),
    }
}

async fn existing_ancestor(path: &Path, label: &'static str) -> Result<PathBuf> {
    let mut current = path.to_path_buf();
    while let Some(parent) = current.parent() {
        match tokio::fs::metadata(parent).await {
            Ok(metadata) if metadata.is_dir() => return Ok(parent.to_path_buf()),
            Ok(_) => {
                return Err(GraphLoomError::RuntimeBuild {
                    source: Box::new(std::io::Error::new(
                        ErrorKind::AlreadyExists,
                        format!("{label} ancestor {} is not a directory", parent.display()),
                    )),
                });
            }
            Err(source) if source.kind() == ErrorKind::NotFound => {
                current = parent.to_path_buf();
            }
            Err(source) => {
                return Err(probe_io_error("inspect writable ancestor", parent, source));
            }
        }
    }
    Err(GraphLoomError::RuntimeBuild {
        source: Box::new(std::io::Error::new(
            ErrorKind::NotFound,
            format!(
                "no writable ancestor found for {label} path {}",
                path.display()
            ),
        )),
    })
}

fn probe_io_error(operation: &'static str, path: &Path, source: std::io::Error) -> GraphLoomError {
    GraphLoomError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
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

pub(crate) fn build_index_pipeline(config: &GraphRagConfig) -> Result<crate::IndexPipeline> {
    let mut registry = IndexWorkflowRegistry::new();
    register_standard_index_workflows(&mut registry)?;
    crate::IndexPipelineFactory::new(registry).standard(config)
}

pub(crate) fn validate_required_models(
    config: &GraphRagConfig,
    requirements: &crate::IndexWorkflowRequirements,
) -> Result<()> {
    for model_id in requirements.completion_models() {
        validate_model(
            model_id,
            require_model(&config.completion_models, model_id)?,
        )?;
    }
    for model_id in requirements.embedding_models() {
        validate_model(model_id, require_model(&config.embedding_models, model_id)?)?;
    }
    Ok(())
}

fn active_workflows(config: &GraphRagConfig) -> BTreeSet<String> {
    config.workflow_order().into_iter().collect()
}

fn validate_chunking_requirements(
    config: &GraphRagConfig,
    requirements: &crate::IndexWorkflowRequirements,
) -> Result<()> {
    if requirements.requires_chunking_config()
        && config.chunking.overlap >= config.chunking.size.get()
    {
        return Err(GraphLoomError::InvalidModel {
            model_id: "chunking".to_owned(),
            message: format!(
                "overlap {} must be smaller than size {}",
                config.chunking.overlap, config.chunking.size,
            ),
        });
    }
    for requirement in requirements.tokenizer_requirements() {
        TiktokenTokenizer::new(&requirement.encoding).map_err(|source| {
            GraphLoomError::InvalidModel {
                model_id: requirement.source.clone(),
                message: source.to_string(),
            }
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

    use secrecy::ExposeSecret;
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
        assert_eq!(
            project.config.local_search.completion_model_id,
            "default_completion_model"
        );
        assert_eq!(
            project.config.global_search.completion_model_id,
            "default_completion_model"
        );
        assert_eq!(
            project.config.drift_search.embedding_model_id,
            "default_embedding_model"
        );
        assert_eq!(project.config.basic_search.k, 10);
        assert!(!project.config.sections.contains_key("local_search"));
        assert!(!project.config.sections.contains_key("global_search"));
        assert!(!project.config.sections.contains_key("drift_search"));
        assert!(!project.config.sections.contains_key("basic_search"));
        validate_project(&project, false).await.expect("validate");

        let mut project = project;
        project.config.sections.insert(
            "custom_extension".to_owned(),
            serde_json::json!({
                "access_token": "dynamic-token-secret",
                "nested": {"password": "dynamic-password-secret"}
            }),
        );
        let summary = redacted_config_summary(&project.config).expect("summary");
        let text = serde_json::to_string(&summary).expect("summary json");
        assert!(!text.contains("super-secret-key"));
        assert!(!text.contains("dynamic-token-secret"));
        assert!(!text.contains("dynamic-password-secret"));
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
    model_provider: deepseek
    model: deepseek-v4-flash
    auth_method: api_key
    api_key: $GRAPHRAG_API_KEY
embedding_models:
  default_embedding_model:
    model_provider: ollama
    model: bge-m3
    auth_method: api_key
    api_key: ${GRAPHRAG_API_KEY}
    api_base: http://localhost:11434
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
                .as_ref()
                .map(ExposeSecret::expose_secret),
            Some("from-env")
        );
        assert_eq!(
            yml.config.completion_models["default_completion_model"].effective_api_base(),
            "https://api.deepseek.com"
        );
        assert_eq!(
            yml.config.embedding_models["default_embedding_model"].effective_api_base(),
            "http://localhost:11434/v1"
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
                .as_ref()
                .map(ExposeSecret::expose_secret),
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
    async fn test_should_use_graphrag_litellm_fallback_for_model_tokenizers() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut project = initialized_project(tempdir.path()).await;
        project.config.chunking.encoding_model = "o200k_base".to_owned();
        project
            .config
            .completion_models
            .get_mut("default_completion_model")
            .expect("completion model")
            .encoding_model = None;
        project
            .config
            .embedding_models
            .get_mut("default_embedding_model")
            .expect("embedding model")
            .encoding_model = None;

        assert_eq!(
            crate::config::effective_completion_encoding(
                &project.config,
                "default_completion_model"
            ),
            "cl100k_base"
        );
        assert_eq!(
            crate::config::effective_embedding_encoding(&project.config, "default_embedding_model"),
            "cl100k_base"
        );
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

    #[tokio::test]
    async fn test_should_validate_chunking_only_for_workflows_that_require_it() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut project = initialized_project(tempdir.path()).await;
        tokio::fs::write(tempdir.path().join("input").join("doc.txt"), "Alice")
            .await
            .expect("input");
        project.config.chunking.overlap = project.config.chunking.size.get();
        project.config.workflows = vec!["create_final_documents".to_owned()];
        validate_project(&project, false)
            .await
            .expect("inactive chunking config should be ignored");

        project.config.workflows = vec!["create_base_text_units".to_owned()];
        let error = validate_project(&project, false)
            .await
            .expect_err("active chunking config should fail");
        assert!(error.to_string().contains("overlap"));
    }

    #[tokio::test]
    async fn test_should_validate_only_active_tokenizer_requirements() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut project = initialized_project(tempdir.path()).await;
        tokio::fs::write(tempdir.path().join("input").join("doc.txt"), "Alice")
            .await
            .expect("input");
        project.config.chunking.encoding_model = "definitely-not-an-encoding".to_owned();
        project.config.workflows = vec!["create_final_documents".to_owned()];
        validate_project(&project, false)
            .await
            .expect("inactive tokenizer should be ignored");

        project.config.workflows = vec!["create_base_text_units".to_owned()];
        let error = validate_project(&project, false)
            .await
            .expect_err("active tokenizer should fail");
        assert!(error.to_string().contains("chunking.encoding_model"));
        assert!(error.to_string().contains("definitely-not-an-encoding"));
    }

    #[tokio::test]
    async fn test_should_validate_only_runtime_required_community_report_prompt() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut project = initialized_project(tempdir.path()).await;
        project.config.workflows = vec!["create_community_reports".to_owned()];
        tokio::fs::remove_file(
            tempdir
                .path()
                .join("prompts")
                .join("community_report_text.txt"),
        )
        .await
        .expect("remove unused text prompt");

        validate_project(&project, false)
            .await
            .expect("unused text prompt should not be required");

        tokio::fs::remove_file(
            tempdir
                .path()
                .join("prompts")
                .join("community_report_graph.txt"),
        )
        .await
        .expect("remove required graph prompt");
        let error = validate_project(&project, false)
            .await
            .expect_err("runtime graph prompt should be required");
        assert!(error.to_string().contains("CommunityReportGraph"));
        assert!(error.to_string().contains("community_report_graph.txt"));
    }

    #[tokio::test]
    async fn test_should_probe_only_paths_needed_by_the_active_run() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut project = initialized_project(tempdir.path()).await;
        let blocked = tempdir.path().join("ordinary-file");
        tokio::fs::write(&blocked, "not a directory")
            .await
            .expect("blocked path");
        let no_vectors = crate::IndexWorkflowRequirements::default();

        project.paths.output_dir = blocked.clone();
        let error = validate_runtime_path_writability(&project, &no_vectors, false)
            .await
            .expect_err("output file must be rejected");
        assert!(error.to_string().contains("output path"));
        assert!(error.to_string().contains("not a directory"));
        project.paths.output_dir = tempdir.path().join("output");

        project.paths.vector_db_uri = blocked.clone();
        validate_runtime_path_writability(&project, &no_vectors, false)
            .await
            .expect("inactive vector path should not be probed");
        let mut vectors = crate::IndexWorkflowRequirements::default();
        vectors.require_vector_store();
        let error = validate_runtime_path_writability(&project, &vectors, false)
            .await
            .expect_err("active vector path must be probed");
        assert!(error.to_string().contains("vector DB path"));
    }

    #[tokio::test]
    async fn test_should_clean_writability_probes_without_creating_runtime_paths() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut project = initialized_project(tempdir.path()).await;
        let output = tempdir.path().join("existing-output");
        let sentinel = output.join("sentinel");
        tokio::fs::create_dir_all(&output)
            .await
            .expect("output directory");
        tokio::fs::write(&sentinel, "preserve")
            .await
            .expect("sentinel");
        project.paths.output_dir = output.clone();
        project.paths.reporting_dir = tempdir.path().join("future").join("logs");
        project.paths.cache_dir = tempdir.path().join("future").join("cache");
        project.paths.vector_db_uri = tempdir.path().join("future").join("lancedb");
        let mut requirements = crate::IndexWorkflowRequirements::default();
        requirements.require_vector_store();

        validate_runtime_path_writability(&project, &requirements, true)
            .await
            .expect("writability probes");

        assert_eq!(
            tokio::fs::read_to_string(&sentinel)
                .await
                .expect("sentinel should remain"),
            "preserve"
        );
        assert!(!tempdir.path().join("future").exists());
        let mut entries = tokio::fs::read_dir(output).await.expect("output listing");
        let mut entry_count = 0;
        while entries
            .next_entry()
            .await
            .expect("read output entry")
            .is_some()
        {
            entry_count += 1;
        }
        assert_eq!(entry_count, 1);
        assert_no_validation_probes(tempdir.path()).await;
    }

    #[tokio::test]
    async fn test_should_skip_optional_writability_probes() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut project = initialized_project(tempdir.path()).await;
        let blocked = tempdir.path().join("cache-is-a-file");
        tokio::fs::write(&blocked, "not a directory")
            .await
            .expect("blocked cache path");
        project.paths.cache_dir = blocked;

        validate_index_project_with_factory(
            &project,
            ValidationMode::SkipOptional,
            &DefaultModelFactory,
        )
        .await
        .expect("skip mode should not probe optional paths");
        assert_no_validation_probes(tempdir.path()).await;
    }

    async fn assert_no_validation_probes(root: &Path) {
        let mut entries = tokio::fs::read_dir(root).await.expect("project entries");
        while let Some(entry) = entries.next_entry().await.expect("project entry") {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            assert!(!name.starts_with(".graphloom-write-probe-"));
        }
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
