//! Project initialization.

mod transaction;

use std::path::{Path, PathBuf};

use serde_yaml::Value as YamlValue;
use transaction::execute_plan;
#[cfg(test)]
use transaction::execute_plan_with_hook;

use crate::{
    cli::{
        args::InitArgs,
        error::{CliError, Result},
    },
    path_safety::{
        absolute_unresolved, is_symlink_or_reparse, normalize_path, reject_symlink_ancestors,
    },
    prompts::PromptKind,
};

const SETTINGS: &str = include_str!("../assets/settings.yaml");
const DOTENV: &str = include_str!("../assets/dotenv");

/// Managed prompt assets.
pub const PROMPT_ASSETS: &[(&str, &str)] = &[
    (
        PromptKind::ExtractGraph.filename(),
        PromptKind::ExtractGraph.default_template(),
    ),
    (
        PromptKind::SummarizeDescriptions.filename(),
        PromptKind::SummarizeDescriptions.default_template(),
    ),
    (
        PromptKind::ExtractClaims.filename(),
        PromptKind::ExtractClaims.default_template(),
    ),
    (
        PromptKind::CommunityReport.filename(),
        PromptKind::CommunityReport.default_template(),
    ),
    (
        "community_report_text.txt",
        include_str!("../assets/prompts/community_report_text.txt"),
    ),
    (
        "drift_search_system_prompt.txt",
        include_str!("../assets/prompts/drift_search_system_prompt.txt"),
    ),
    (
        "drift_reduce_prompt.txt",
        include_str!("../assets/prompts/drift_reduce_prompt.txt"),
    ),
    (
        "global_search_map_system_prompt.txt",
        include_str!("../assets/prompts/global_search_map_system_prompt.txt"),
    ),
    (
        "global_search_reduce_system_prompt.txt",
        include_str!("../assets/prompts/global_search_reduce_system_prompt.txt"),
    ),
    (
        "global_search_knowledge_system_prompt.txt",
        include_str!("../assets/prompts/global_search_knowledge_system_prompt.txt"),
    ),
    (
        "local_search_system_prompt.txt",
        include_str!("../assets/prompts/local_search_system_prompt.txt"),
    ),
    (
        "basic_search_system_prompt.txt",
        include_str!("../assets/prompts/basic_search_system_prompt.txt"),
    ),
    (
        "question_gen_system_prompt.txt",
        include_str!("../assets/prompts/question_gen_system_prompt.txt"),
    ),
];

/// Initialize a `GraphLoom` project.
///
/// # Errors
///
/// Returns an error when the root cannot be created or managed files cannot be written.
pub async fn init_project(args: &InitArgs) -> Result<()> {
    let plan = InitPlan::build(args).await?;
    execute_plan(&plan).await?;

    println!("Initialized GraphLoom project at {}", plan.root.display());
    Ok(())
}

#[derive(Debug)]
struct InitPlan {
    root: PathBuf,
    directories: Vec<PathBuf>,
    files: Vec<ManagedFilePlan>,
}

#[derive(Debug)]
struct ManagedFilePlan {
    path: PathBuf,
    content: String,
    overwrite: bool,
}

impl InitPlan {
    async fn build(args: &InitArgs) -> Result<Self> {
        validate_model_argument("model", &args.model)?;
        validate_model_argument("embedding", &args.embedding)?;
        let raw_root = absolute_unresolved(&args.root)?;
        reject_symlink_ancestors(&raw_root).await?;
        reject_symlink(&raw_root).await?;
        let root = normalize_path(&raw_root);
        let settings = root.join("settings.yaml");
        if tokio::fs::try_exists(&settings)
            .await
            .map_err(|source| CliError::Io {
                operation: "check settings",
                path: settings.clone(),
                source,
            })?
            && !args.force
        {
            return Err(CliError::AlreadyInitialized { root });
        }

        let settings_content = render_settings(&args.model, &args.embedding)?;
        let mut plan = Self {
            root: root.clone(),
            directories: vec![root.clone(), root.join("input"), root.join("prompts")],
            files: vec![
                ManagedFilePlan {
                    path: settings,
                    content: settings_content,
                    overwrite: true,
                },
                ManagedFilePlan {
                    path: root.join(".env"),
                    content: DOTENV.to_owned(),
                    overwrite: args.force,
                },
            ],
        };
        plan.files
            .extend(PROMPT_ASSETS.iter().map(|(name, content)| ManagedFilePlan {
                path: root.join("prompts").join(name),
                content: (*content).to_owned(),
                overwrite: args.force,
            }));
        plan.preflight().await?;
        Ok(plan)
    }

    async fn preflight(&self) -> Result<()> {
        for directory in &self.directories {
            preflight_directory(directory).await?;
        }
        for file in &self.files {
            preflight_managed_file(file).await?;
        }
        Ok(())
    }
}

fn validate_model_argument(name: &str, value: &str) -> Result<()> {
    if value.contains('\0') || value.chars().any(char::is_control) {
        return Err(CliError::InvalidModel {
            model_id: name.to_owned(),
            message: "model names must not contain NUL or control characters".to_owned(),
        });
    }
    Ok(())
}

fn render_settings(model: &str, embedding: &str) -> Result<String> {
    let mut value: YamlValue =
        serde_yaml::from_str(SETTINGS).map_err(|source| CliError::ConfigParse {
            path: PathBuf::from("<built-in settings.yaml>"),
            source: Box::new(source),
        })?;
    set_yaml_path(
        &mut value,
        &["completion_models", "default_completion_model", "model"],
        model,
    )?;
    set_yaml_path(
        &mut value,
        &["embedding_models", "default_embedding_model", "model"],
        embedding,
    )?;
    serde_yaml::to_string(&value).map_err(|source| CliError::ConfigParse {
        path: PathBuf::from("<built-in settings.yaml>"),
        source: Box::new(source),
    })
}

fn set_yaml_path(value: &mut YamlValue, path: &[&str], replacement: &str) -> Result<()> {
    let mut current = value;
    for segment in &path[..path.len().saturating_sub(1)] {
        current = current
            .get_mut(*segment)
            .ok_or_else(|| CliError::InvalidRoot {
                path: PathBuf::from("<built-in settings.yaml>"),
                message: format!("missing settings key {segment}"),
            })?;
    }
    let leaf = path.last().copied().ok_or_else(|| CliError::InvalidRoot {
        path: PathBuf::from("<built-in settings.yaml>"),
        message: "empty settings key path".to_owned(),
    })?;
    let Some(slot) = current.get_mut(leaf) else {
        return Err(CliError::InvalidRoot {
            path: PathBuf::from("<built-in settings.yaml>"),
            message: format!("missing settings key {leaf}"),
        });
    };
    *slot = YamlValue::String(replacement.to_owned());
    Ok(())
}

async fn preflight_directory(path: &Path) -> Result<()> {
    reject_symlink_ancestors(path).await?;
    reject_symlink(path).await?;
    match tokio::fs::metadata(path).await {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(CliError::InvalidRoot {
            path: path.to_path_buf(),
            message: "expected directory path".to_owned(),
        }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(CliError::Io {
            operation: "stat directory",
            path: path.to_path_buf(),
            source,
        }),
    }
}

async fn preflight_managed_file(file: &ManagedFilePlan) -> Result<()> {
    if let Some(parent) = file.path.parent() {
        reject_symlink_ancestors(parent).await?;
        reject_symlink(parent).await?;
    }
    reject_symlink(&file.path).await?;
    match tokio::fs::metadata(&file.path).await {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(CliError::InvalidRoot {
            path: file.path.clone(),
            message: "refusing to overwrite non-file managed path".to_owned(),
        }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(CliError::Io {
            operation: "stat managed file",
            path: file.path.clone(),
            source,
        }),
    }
}

async fn reject_symlink(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        reject_symlink_ancestors(parent).await?;
    }
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if is_symlink_or_reparse(&metadata) => Err(CliError::InvalidRoot {
            path: path.to_path_buf(),
            message: "refusing to overwrite symlink".to_owned(),
        }),
        Ok(_) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(CliError::Io {
            operation: "check symlink",
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn args(root: &Path, force: bool) -> InitArgs {
        InitArgs {
            root: root.to_path_buf(),
            model: "gpt-4.1".to_owned(),
            embedding: "text-embedding-3-large".to_owned(),
            force,
        }
    }

    #[tokio::test]
    async fn test_should_create_project_with_all_assets() {
        let tempdir = TempDir::new().expect("tempdir");
        init_project(&args(tempdir.path(), false))
            .await
            .expect("init");

        assert!(tempdir.path().join("settings.yaml").is_file());
        assert!(tempdir.path().join(".env").is_file());
        assert!(tempdir.path().join("input").is_dir());
        assert!(tempdir.path().join("prompts").is_dir());
        for (name, content) in PROMPT_ASSETS {
            let path = tempdir.path().join("prompts").join(name);
            assert!(path.is_file());
            assert_eq!(
                tokio::fs::read_to_string(path).await.expect("prompt asset"),
                *content,
            );
        }
    }

    #[tokio::test]
    async fn test_should_initialize_tera_prompt_templates() {
        let tempdir = TempDir::new().expect("tempdir");
        init_project(&args(tempdir.path(), false))
            .await
            .expect("init");

        let extract_graph =
            tokio::fs::read_to_string(tempdir.path().join("prompts").join("extract_graph.txt"))
                .await
                .expect("extract graph prompt");

        assert!(extract_graph.contains("{{ input_text }}"));
        assert!(extract_graph.contains("{{ entity_types }}"));
        assert!(!extract_graph.contains("{input_text}"));
        assert!(!extract_graph.contains("{entity_types}"));
    }

    #[tokio::test]
    async fn test_should_write_model_names_with_yaml_safe_serialization() {
        let tempdir = TempDir::new().expect("tempdir");
        let completion = "vendor:model#v1 \"quoted\"";
        let embedding = "embed:model#v2 [brackets]";
        init_project(&InitArgs {
            root: tempdir.path().to_path_buf(),
            model: completion.to_owned(),
            embedding: embedding.to_owned(),
            force: false,
        })
        .await
        .expect("init");

        let settings = tokio::fs::read_to_string(tempdir.path().join("settings.yaml"))
            .await
            .expect("settings");
        let config: crate::GraphRagConfig = serde_yaml::from_str(&settings).expect("config");
        assert_eq!(
            config.completion_models["default_completion_model"].model,
            completion
        );
        assert_eq!(
            config.embedding_models["default_embedding_model"].model,
            embedding
        );
        assert_eq!(config.output_storage.base_dir, "output");
        assert_eq!(config.input_storage.base_dir, "input");
        assert_eq!(
            config.extract_graph.prompt.as_deref(),
            Some("prompts/extract_graph.txt")
        );
        assert_eq!(
            config.community_reports.graph_prompt.as_deref(),
            Some("prompts/community_report.txt")
        );
        assert!(config.sections.contains_key("local_search"));
        assert!(config.sections.contains_key("global_search"));
        assert!(config.sections.contains_key("drift_search"));
        assert!(config.sections.contains_key("basic_search"));
    }

    #[tokio::test]
    async fn test_should_reject_control_character_model_name_without_side_effects() {
        let tempdir = TempDir::new().expect("tempdir");
        let root = tempdir.path().join("project");
        let error = init_project(&InitArgs {
            root: root.clone(),
            model: "bad\nmodel".to_owned(),
            embedding: "embedding".to_owned(),
            force: false,
        })
        .await
        .expect_err("control characters should fail");

        assert!(error.to_string().contains("control"));
        assert!(!root.exists());
    }

    #[tokio::test]
    async fn test_should_fail_when_already_initialized_without_force() {
        let tempdir = TempDir::new().expect("tempdir");
        let settings = tempdir.path().join("settings.yaml");
        tokio::fs::write(&settings, "original")
            .await
            .expect("settings");

        let error = init_project(&args(tempdir.path(), false))
            .await
            .expect_err("already initialized");

        assert!(error.to_string().contains("--force"));
        assert_eq!(
            tokio::fs::read_to_string(settings).await.expect("settings"),
            "original"
        );
    }

    #[tokio::test]
    async fn test_should_force_overwrite_managed_files_and_preserve_user_files() {
        let tempdir = TempDir::new().expect("tempdir");
        init_project(&args(tempdir.path(), false))
            .await
            .expect("init");
        tokio::fs::write(tempdir.path().join(".env"), "OLD=1")
            .await
            .expect("dotenv");
        tokio::fs::write(
            tempdir.path().join("prompts").join("extract_graph.txt"),
            "old prompt",
        )
        .await
        .expect("prompt");
        tokio::fs::write(tempdir.path().join("prompts").join("custom.txt"), "custom")
            .await
            .expect("custom prompt");
        tokio::fs::write(tempdir.path().join("input").join("doc.txt"), "user input")
            .await
            .expect("input");
        tokio::fs::write(tempdir.path().join("notes.txt"), "keep")
            .await
            .expect("unknown");

        init_project(&args(tempdir.path(), true))
            .await
            .expect("force");

        assert_eq!(
            tokio::fs::read_to_string(tempdir.path().join(".env"))
                .await
                .expect("dotenv"),
            DOTENV
        );
        assert_ne!(
            tokio::fs::read_to_string(tempdir.path().join("prompts").join("extract_graph.txt"))
                .await
                .expect("prompt"),
            "old prompt"
        );
        assert_eq!(
            tokio::fs::read_to_string(tempdir.path().join("prompts").join("custom.txt"))
                .await
                .expect("custom"),
            "custom"
        );
        assert_eq!(
            tokio::fs::read_to_string(tempdir.path().join("input").join("doc.txt"))
                .await
                .expect("input"),
            "user input"
        );
        assert_eq!(
            tokio::fs::read_to_string(tempdir.path().join("notes.txt"))
                .await
                .expect("unknown"),
            "keep"
        );
    }

    #[tokio::test]
    async fn test_should_roll_back_every_managed_file_when_force_publish_fails() {
        let tempdir = TempDir::new().expect("tempdir");
        init_project(&args(tempdir.path(), false))
            .await
            .expect("initial init");
        tokio::fs::write(tempdir.path().join(".env"), "ORIGINAL=1")
            .await
            .expect("custom dotenv");
        let settings_before = tokio::fs::read(tempdir.path().join("settings.yaml"))
            .await
            .expect("settings before");
        let prompt_before =
            tokio::fs::read(tempdir.path().join("prompts").join("extract_graph.txt"))
                .await
                .expect("prompt before");
        let plan = InitPlan::build(&InitArgs {
            root: tempdir.path().to_path_buf(),
            model: "replacement-model".to_owned(),
            embedding: "replacement-embedding".to_owned(),
            force: true,
        })
        .await
        .expect("plan");

        let error = execute_plan_with_hook(&plan, |index| {
            if index == 1 {
                return Err(CliError::Io {
                    operation: "inject publish failure",
                    path: tempdir.path().join(".env"),
                    source: std::io::Error::other("injected failure"),
                });
            }
            Ok(())
        })
        .await
        .expect_err("publish should fail");

        assert!(error.to_string().contains("injected failure"));
        assert_eq!(
            tokio::fs::read(tempdir.path().join("settings.yaml"))
                .await
                .expect("settings after"),
            settings_before,
        );
        assert_eq!(
            tokio::fs::read_to_string(tempdir.path().join(".env"))
                .await
                .expect("dotenv after"),
            "ORIGINAL=1",
        );
        assert_eq!(
            tokio::fs::read(tempdir.path().join("prompts").join("extract_graph.txt"),)
                .await
                .expect("prompt after"),
            prompt_before,
        );
        assert_no_transaction_files(tempdir.path()).await;
    }

    #[tokio::test]
    async fn test_should_remove_new_scaffold_when_initial_publish_fails() {
        let tempdir = TempDir::new().expect("tempdir");
        let root = tempdir.path().join("project");
        let plan = InitPlan::build(&args(&root, false)).await.expect("plan");

        execute_plan_with_hook(&plan, |index| {
            if index == 1 {
                return Err(CliError::Io {
                    operation: "inject initial publish failure",
                    path: root.join(".env"),
                    source: std::io::Error::other("injected failure"),
                });
            }
            Ok(())
        })
        .await
        .expect_err("publication should fail");

        assert!(!root.exists(), "incomplete scaffold must be removed");
    }

    async fn assert_no_transaction_files(root: &Path) {
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            let mut entries = tokio::fs::read_dir(&directory)
                .await
                .expect("read directory");
            while let Some(entry) = entries.next_entry().await.expect("directory entry") {
                let path = entry.path();
                if entry.file_type().await.expect("file type").is_dir() {
                    pending.push(path);
                    continue;
                }
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .expect("UTF-8 test file name");
                let extension = path.extension().and_then(|extension| extension.to_str());
                assert_ne!(extension, Some("tmp"), "staged file leaked: {name}");
                assert_ne!(extension, Some("backup"), "backup leaked: {name}");
            }
        }
    }

    #[tokio::test]
    async fn test_should_not_overwrite_partial_project_without_settings() {
        let tempdir = TempDir::new().expect("tempdir");
        tokio::fs::create_dir(tempdir.path().join("prompts"))
            .await
            .expect("prompts");
        tokio::fs::write(tempdir.path().join(".env"), "EXISTING=1")
            .await
            .expect("dotenv");
        tokio::fs::write(
            tempdir.path().join("prompts").join("extract_graph.txt"),
            "existing prompt",
        )
        .await
        .expect("prompt");

        init_project(&args(tempdir.path(), false))
            .await
            .expect("partial init");

        assert_eq!(
            tokio::fs::read_to_string(tempdir.path().join(".env"))
                .await
                .expect("dotenv"),
            "EXISTING=1"
        );
        assert_eq!(
            tokio::fs::read_to_string(tempdir.path().join("prompts").join("extract_graph.txt"))
                .await
                .expect("prompt"),
            "existing prompt"
        );
        assert!(
            tempdir
                .path()
                .join("prompts")
                .join("summarize_descriptions.txt")
                .is_file()
        );
    }

    #[tokio::test]
    async fn test_should_ignore_stale_temporary_file_names() {
        let tempdir = TempDir::new().expect("tempdir");
        tokio::fs::write(tempdir.path().join(".settings.yaml.tmp-graphloom"), "stale")
            .await
            .expect("stale temp");

        init_project(&args(tempdir.path(), false))
            .await
            .expect("init");

        assert!(tempdir.path().join("settings.yaml").is_file());
        assert_eq!(
            tokio::fs::read_to_string(tempdir.path().join(".settings.yaml.tmp-graphloom"))
                .await
                .expect("stale temp"),
            "stale"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_should_reject_symlink_managed_file_target() {
        let tempdir = TempDir::new().expect("tempdir");
        let external = tempdir.path().join("external-settings.yaml");
        tokio::fs::write(&external, "external")
            .await
            .expect("external");
        std::os::unix::fs::symlink(&external, tempdir.path().join("settings.yaml"))
            .expect("symlink");

        let error = init_project(&args(tempdir.path(), true))
            .await
            .expect_err("symlink target should fail");
        assert!(error.to_string().contains("symlink"));
        assert_eq!(
            tokio::fs::read_to_string(&external)
                .await
                .expect("external"),
            "external"
        );
        assert!(!tempdir.path().join(".env").exists());
        assert!(!tempdir.path().join("input").exists());
        assert!(!tempdir.path().join("prompts").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_should_reject_symlink_prompt_parent() {
        let tempdir = TempDir::new().expect("tempdir");
        let external = TempDir::new().expect("external");
        std::os::unix::fs::symlink(external.path(), tempdir.path().join("prompts"))
            .expect("symlink");

        let error = init_project(&args(tempdir.path(), false))
            .await
            .expect_err("symlink parent should fail");
        assert!(error.to_string().contains("symlink"));
        assert!(!tempdir.path().join("settings.yaml").exists());
        assert!(!tempdir.path().join(".env").exists());
        assert!(
            tokio::fs::read_dir(external.path())
                .await
                .expect("external prompts")
                .next_entry()
                .await
                .expect("read external")
                .is_none()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_should_reject_symlink_root_without_side_effects() {
        let tempdir = TempDir::new().expect("tempdir");
        let external = TempDir::new().expect("external");
        let project = external.path().join("project");
        tokio::fs::create_dir(&project).await.expect("project");
        let link = tempdir.path().join("project-link");
        std::os::unix::fs::symlink(&project, &link).expect("symlink");

        let error = init_project(&args(&link, false))
            .await
            .expect_err("symlink root should fail");
        assert!(error.to_string().contains("symlink"));
        assert!(!project.join("input").exists());
        assert!(!project.join("prompts").exists());
        assert!(!project.join("settings.yaml").exists());
        assert!(!project.join(".env").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_should_reject_symlink_ancestor_before_normalization_without_side_effects() {
        let tempdir = TempDir::new().expect("tempdir");
        let external = TempDir::new().expect("external");
        let link = tempdir.path().join("ancestor-link");
        std::os::unix::fs::symlink(external.path(), &link).expect("symlink");
        let root = link.join("project");

        let error = init_project(&args(&root, false))
            .await
            .expect_err("symlink ancestor should fail");

        assert!(error.to_string().contains("symlink"));
        assert!(!external.path().join("project").exists());
        assert!(!external.path().join("input").exists());
        assert!(!external.path().join("prompts").exists());
        assert!(!external.path().join("settings.yaml").exists());
        assert!(!external.path().join(".env").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_should_reject_symlink_ancestor_hidden_by_parent_component() {
        let tempdir = TempDir::new().expect("tempdir");
        let external = TempDir::new().expect("external");
        let link = tempdir.path().join("ancestor-link");
        std::os::unix::fs::symlink(external.path(), &link).expect("symlink");
        let root = link.join("..").join("project");

        let error = init_project(&args(&root, false))
            .await
            .expect_err("symlink ancestor hidden by .. should fail");

        assert!(error.to_string().contains("symlink"));
        assert!(!tempdir.path().join("project").exists());
        assert!(!external.path().join("project").exists());
    }

    #[tokio::test]
    async fn test_should_reject_non_directory_parent_before_target_check() {
        let tempdir = TempDir::new().expect("tempdir");
        let file = tempdir.path().join("file");
        tokio::fs::write(&file, "not a directory")
            .await
            .expect("file");
        let child = file.join("child");

        let error = reject_symlink(&child)
            .await
            .expect_err("not a directory must not be swallowed");

        assert!(error.to_string().contains("not a directory"));
    }

    #[tokio::test]
    async fn test_should_reject_non_directory_ancestor_cross_platform() {
        let tempdir = TempDir::new().expect("tempdir");
        let file = tempdir.path().join("file");
        tokio::fs::write(&file, "not a directory")
            .await
            .expect("file");
        let child = file.join("child").join("project");

        let error = reject_symlink_ancestors(&child)
            .await
            .expect_err("not a directory must not be swallowed");

        assert!(error.to_string().contains("not a directory"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn test_should_check_verbatim_path_ancestors_without_querying_prefix_only_path() {
        let tempdir = TempDir::new().expect("tempdir");
        let canonical = tempdir.path().canonicalize().expect("canonical tempdir");
        crate::path_safety::tests::windows::assert_windows_verbatim_path(&canonical);

        reject_symlink_ancestors(&canonical.join("missing").join("child"))
            .await
            .expect("verbatim path ancestor check");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_should_reject_symlink_input_directory_without_side_effects() {
        let tempdir = TempDir::new().expect("tempdir");
        let external = TempDir::new().expect("external");
        std::os::unix::fs::symlink(external.path(), tempdir.path().join("input")).expect("symlink");

        let error = init_project(&args(tempdir.path(), false))
            .await
            .expect_err("symlink input should fail");
        assert!(error.to_string().contains("symlink"));
        assert!(!tempdir.path().join("settings.yaml").exists());
        assert!(!tempdir.path().join(".env").exists());
        assert!(!tempdir.path().join("prompts").exists());
        assert!(
            tokio::fs::read_dir(external.path())
                .await
                .expect("external input")
                .next_entry()
                .await
                .expect("read external")
                .is_none()
        );
    }
}
