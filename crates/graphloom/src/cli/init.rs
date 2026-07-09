//! Project initialization.

use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::cli::{
    args::InitArgs,
    error::{CliError, Result},
};

const SETTINGS: &str = include_str!("../assets/settings.yaml");
const DOTENV: &str = include_str!("../assets/dotenv");

/// Managed prompt assets.
pub const PROMPT_ASSETS: &[(&str, &str)] = &[
    (
        "extract_graph.txt",
        include_str!("../assets/prompts/extract_graph.txt"),
    ),
    (
        "summarize_descriptions.txt",
        include_str!("../assets/prompts/summarize_descriptions.txt"),
    ),
    (
        "extract_claims.txt",
        include_str!("../assets/prompts/extract_claims.txt"),
    ),
    (
        "community_report_graph.txt",
        include_str!("../assets/prompts/community_report_graph.txt"),
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
    let root = normalize_existing_parent(&args.root)?;
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

    create_dir(&root).await?;
    create_dir(&root.join("input")).await?;
    create_dir(&root.join("prompts")).await?;

    let settings_content = SETTINGS
        .replace("__COMPLETION_MODEL__", &args.model)
        .replace("__EMBEDDING_MODEL__", &args.embedding);
    write_managed_file(&settings, &settings_content, true).await?;
    write_managed_file(&root.join(".env"), DOTENV, args.force).await?;

    for (name, content) in PROMPT_ASSETS {
        write_managed_file(&root.join("prompts").join(name), content, args.force).await?;
    }

    println!("Initialized GraphLoom project at {}", root.display());
    Ok(())
}

async fn create_dir(path: &Path) -> Result<()> {
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|source| CliError::Io {
            operation: "create directory",
            path: path.to_path_buf(),
            source,
        })
}

async fn write_managed_file(path: &Path, content: &str, overwrite: bool) -> Result<()> {
    let exists = tokio::fs::try_exists(path)
        .await
        .map_err(|source| CliError::Io {
            operation: "check file",
            path: path.to_path_buf(),
            source,
        })?;
    if exists && !overwrite {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        reject_symlink_ancestors(parent).await?;
        create_dir(parent).await?;
    }
    reject_symlink(path).await?;
    if exists {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|source| CliError::Io {
                operation: "stat existing managed file",
                path: path.to_path_buf(),
                source,
            })?;
        if !metadata.is_file() {
            return Err(CliError::InvalidRoot {
                path: path.to_path_buf(),
                message: "refusing to overwrite non-file managed path".to_owned(),
            });
        }
    }

    let tmp = temporary_sibling(path)?;
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&tmp)
        .await
        .map_err(|source| CliError::Io {
            operation: "create temporary file",
            path: tmp.clone(),
            source,
        })?;
    if let Err(source) = file.write_all(content.as_bytes()).await {
        cleanup_tmp(&tmp).await;
        return Err(CliError::Io {
            operation: "write temporary file",
            path: tmp,
            source,
        });
    }
    if let Err(source) = file.flush().await {
        cleanup_tmp(&tmp).await;
        return Err(CliError::Io {
            operation: "flush temporary file",
            path: tmp,
            source,
        });
    }
    drop(file);
    if overwrite
        && exists
        && let Err(source) = tokio::fs::remove_file(path).await
    {
        cleanup_tmp(&tmp).await;
        return Err(CliError::Io {
            operation: "remove existing managed file",
            path: path.to_path_buf(),
            source,
        });
    }
    if let Err(source) = tokio::fs::rename(&tmp, path).await {
        cleanup_tmp(&tmp).await;
        return Err(CliError::Io {
            operation: "rename temporary file",
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

async fn reject_symlink(path: &Path) -> Result<()> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(CliError::InvalidRoot {
            path: path.to_path_buf(),
            message: "refusing to overwrite symlink".to_owned(),
        }),
        Ok(_) | Err(_) => Ok(()),
    }
}

async fn reject_symlink_ancestors(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        match tokio::fs::symlink_metadata(&current).await {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(CliError::InvalidRoot {
                    path: current,
                    message: "refusing to write through symlink parent".to_owned(),
                });
            }
            Ok(_) => {}
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(CliError::Io {
                    operation: "check parent symlink",
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(())
}

fn temporary_sibling(path: &Path) -> Result<PathBuf> {
    let parent = path.parent().ok_or_else(|| CliError::InvalidRoot {
        path: path.to_path_buf(),
        message: "managed path has no parent".to_owned(),
    })?;
    let name = path.file_name().ok_or_else(|| CliError::InvalidRoot {
        path: path.to_path_buf(),
        message: "managed path has no file name".to_owned(),
    })?;
    Ok(parent.join(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        Uuid::new_v4()
    )))
}

async fn cleanup_tmp(path: &Path) {
    let _ = tokio::fs::remove_file(path).await;
}

fn normalize_existing_parent(path: &Path) -> Result<PathBuf> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        && parent.exists()
    {
        let parent = parent.canonicalize().map_err(|source| CliError::Io {
            operation: "canonicalize parent",
            path: parent.to_path_buf(),
            source,
        })?;
        let name = path.file_name().ok_or_else(|| CliError::InvalidRoot {
            path: path.to_path_buf(),
            message: "root path has no final component".to_owned(),
        })?;
        return Ok(parent.join(name));
    }
    Ok(path.to_path_buf())
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
        for (name, _) in PROMPT_ASSETS {
            assert!(tempdir.path().join("prompts").join(name).is_file());
        }
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
    }
}
