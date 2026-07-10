//! Recoverable publication of initialized project files.

use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use super::{InitPlan, ManagedFilePlan};
use crate::cli::error::{CliError, Result};

#[derive(Debug)]
struct StagedManagedFile {
    target: PathBuf,
    staged: PathBuf,
}

#[derive(Debug)]
struct PublishedManagedFile {
    target: PathBuf,
    backup: Option<PathBuf>,
}

pub(super) async fn execute_plan(plan: &InitPlan) -> Result<()> {
    execute_plan_with_hook(plan, |_| Ok(())).await
}

pub(super) async fn execute_plan_with_hook<F>(plan: &InitPlan, mut before_publish: F) -> Result<()>
where
    F: FnMut(usize) -> Result<()>,
{
    let created_directories = missing_directories(&plan.directories).await?;
    if let Err(error) = create_directories(&plan.directories).await {
        remove_empty_directories(&created_directories).await;
        return Err(error);
    }

    let staged = match stage_managed_files(&plan.files).await {
        Ok(staged) => staged,
        Err(error) => {
            remove_empty_directories(&created_directories).await;
            return Err(error);
        }
    };
    let mut published = Vec::with_capacity(staged.len());

    for (index, file) in staged.iter().enumerate() {
        let backup = match move_target_to_backup(&file.target).await {
            Ok(backup) => backup,
            Err(error) => {
                let rollback = rollback_published(&published).await;
                cleanup_staged(&staged).await;
                remove_empty_directories(&created_directories).await;
                return Err(with_rollback(
                    "publish initialized project",
                    error,
                    rollback,
                ));
            }
        };
        if let Err(error) = before_publish(index) {
            let current_rollback = restore_backup(&file.target, backup.as_deref()).await;
            let previous_rollback = rollback_published(&published).await;
            cleanup_staged(&staged).await;
            remove_empty_directories(&created_directories).await;
            return Err(with_rollback(
                "publish initialized project",
                error,
                current_rollback.and(previous_rollback),
            ));
        }
        if let Err(source) = tokio::fs::rename(&file.staged, &file.target).await {
            let error = CliError::Io {
                operation: "publish managed file",
                path: file.target.clone(),
                source,
            };
            let current_rollback = restore_backup(&file.target, backup.as_deref()).await;
            let previous_rollback = rollback_published(&published).await;
            cleanup_staged(&staged).await;
            remove_empty_directories(&created_directories).await;
            return Err(with_rollback(
                "publish initialized project",
                error,
                current_rollback.and(previous_rollback),
            ));
        }
        published.push(PublishedManagedFile {
            target: file.target.clone(),
            backup,
        });
    }

    remove_backups(&published).await;
    Ok(())
}

async fn missing_directories(directories: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut missing = Vec::new();
    for directory in directories {
        if !tokio::fs::try_exists(directory)
            .await
            .map_err(|source| CliError::Io {
                operation: "check initialization directory",
                path: directory.clone(),
                source,
            })?
        {
            missing.push(directory.clone());
        }
    }
    Ok(missing)
}

async fn create_directories(directories: &[PathBuf]) -> Result<()> {
    for directory in directories {
        tokio::fs::create_dir_all(directory)
            .await
            .map_err(|source| CliError::Io {
                operation: "create directory",
                path: directory.clone(),
                source,
            })?;
    }
    Ok(())
}

async fn stage_managed_files(files: &[ManagedFilePlan]) -> Result<Vec<StagedManagedFile>> {
    let mut staged = Vec::new();
    for file in files {
        let exists = tokio::fs::try_exists(&file.path)
            .await
            .map_err(|source| CliError::Io {
                operation: "check managed file",
                path: file.path.clone(),
                source,
            })?;
        if exists && !file.overwrite {
            continue;
        }
        let staged_path = temporary_sibling(&file.path, "tmp")?;
        if let Err(error) = write_staged_file(&staged_path, &file.content).await {
            cleanup_staged(&staged).await;
            return Err(error);
        }
        staged.push(StagedManagedFile {
            target: file.path.clone(),
            staged: staged_path,
        });
    }
    Ok(staged)
}

async fn write_staged_file(path: &Path, content: &str) -> Result<()> {
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .await
        .map_err(|source| CliError::Io {
            operation: "create staged managed file",
            path: path.to_path_buf(),
            source,
        })?;
    if let Err(source) = file.write_all(content.as_bytes()).await {
        cleanup_tmp(path).await;
        return Err(CliError::Io {
            operation: "write staged managed file",
            path: path.to_path_buf(),
            source,
        });
    }
    if let Err(source) = file.sync_all().await {
        cleanup_tmp(path).await;
        return Err(CliError::Io {
            operation: "sync staged managed file",
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

async fn move_target_to_backup(target: &Path) -> Result<Option<PathBuf>> {
    if !tokio::fs::try_exists(target)
        .await
        .map_err(|source| CliError::Io {
            operation: "check managed file before publish",
            path: target.to_path_buf(),
            source,
        })?
    {
        return Ok(None);
    }
    let backup = temporary_sibling(target, "backup")?;
    tokio::fs::rename(target, &backup)
        .await
        .map_err(|source| CliError::Io {
            operation: "backup managed file",
            path: target.to_path_buf(),
            source,
        })?;
    Ok(Some(backup))
}

async fn restore_backup(target: &Path, backup: Option<&Path>) -> Result<()> {
    if let Some(backup) = backup {
        tokio::fs::rename(backup, target)
            .await
            .map_err(|source| CliError::Io {
                operation: "restore managed file backup",
                path: target.to_path_buf(),
                source,
            })?;
    }
    Ok(())
}

async fn rollback_published(files: &[PublishedManagedFile]) -> Result<()> {
    let mut first_error = None;
    for file in files.iter().rev() {
        if let Err(source) = tokio::fs::remove_file(&file.target).await
            && source.kind() != std::io::ErrorKind::NotFound
            && first_error.is_none()
        {
            first_error = Some(CliError::Io {
                operation: "remove published managed file during rollback",
                path: file.target.clone(),
                source,
            });
        }
        if let Err(error) = restore_backup(&file.target, file.backup.as_deref()).await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

async fn remove_backups(files: &[PublishedManagedFile]) {
    for file in files {
        if let Some(backup) = &file.backup
            && let Err(source) = tokio::fs::remove_file(backup).await
        {
            tracing::warn!(path = %backup.display(), error = %source, "failed to remove obsolete initialization backup");
        }
    }
}

async fn cleanup_staged(files: &[StagedManagedFile]) {
    for file in files {
        cleanup_tmp(&file.staged).await;
    }
}

async fn remove_empty_directories(directories: &[PathBuf]) {
    for directory in directories.iter().rev() {
        if let Err(source) = tokio::fs::remove_dir(directory).await
            && source.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %directory.display(), error = %source, "failed to remove empty initialization directory");
        }
    }
}

fn temporary_sibling(path: &Path, kind: &str) -> Result<PathBuf> {
    let parent = path.parent().ok_or_else(|| CliError::InvalidRoot {
        path: path.to_path_buf(),
        message: "managed path has no parent".to_owned(),
    })?;
    let name = path.file_name().ok_or_else(|| CliError::InvalidRoot {
        path: path.to_path_buf(),
        message: "managed path has no file name".to_owned(),
    })?;
    Ok(parent.join(format!(
        ".{}.{}.{}",
        name.to_string_lossy(),
        Uuid::new_v4(),
        kind,
    )))
}

async fn cleanup_tmp(path: &Path) {
    if let Err(source) = tokio::fs::remove_file(path).await
        && source.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(path = %path.display(), error = %source, "failed to remove staged initialization file");
    }
}

fn with_rollback(operation: &'static str, source: CliError, rollback: Result<()>) -> CliError {
    match rollback {
        Ok(()) => source,
        Err(rollback) => CliError::RollbackFailed {
            operation,
            source: Box::new(source),
            rollback: Box::new(rollback),
        },
    }
}
