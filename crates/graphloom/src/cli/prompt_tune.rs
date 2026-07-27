//! CLI handler for `graphloom prompt-tune`.

use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;
use tracing::info;
use uuid::Uuid;

use crate::{
    api::{DocSelectionType, GenerateIndexingPromptsOptions, generate_indexing_prompts},
    cli::{
        args::{PromptTuneArgs, PromptTuneSelectionMethod},
        callbacks::ConsoleStageProgress,
        error::{CliError, Result},
    },
    path_safety::{
        absolute_unresolved, is_symlink_or_reparse, normalize_path, reject_symlink_ancestors,
    },
};

const EXTRACT_GRAPH_FILENAME: &str = "extract_graph.txt";
const ENTITY_SUMMARIZATION_FILENAME: &str = "summarize_descriptions.txt";
const COMMUNITY_SUMMARIZATION_FILENAME: &str = "community_report_graph.txt";

/// Run the `prompt-tune` command.
///
/// # Errors
///
/// Returns an error when project loading, LLM calls, or file publication fails.
pub async fn run(args: &PromptTuneArgs) -> Result<()> {
    let progress = ConsoleStageProgress::start("prompt tuning", args.verbose);
    let options = build_options(args)?;

    info!("Generating indexing prompts...");
    let prompts = generate_indexing_prompts(&options)
        .await
        .map_err(|source| CliError::PromptTune {
            source: Box::new(source),
        })?;

    progress.finish();

    let output_dir = resolve_output_dir(&options.root, &args.output).await?;
    publish_prompts(&output_dir, &prompts).await?;

    println!("Prompts written to {}", output_dir.display());
    Ok(())
}

fn build_options(args: &PromptTuneArgs) -> Result<GenerateIndexingPromptsOptions> {
    let selection_method = match args.selection_method {
        PromptTuneSelectionMethod::Top => DocSelectionType::Top,
        PromptTuneSelectionMethod::Random => DocSelectionType::Random,
        PromptTuneSelectionMethod::Auto => DocSelectionType::Auto,
    };

    Ok(GenerateIndexingPromptsOptions {
        root: args.root.clone(),
        limit: args.limit,
        selection_method,
        domain: args.domain.clone(),
        language: args.language.clone(),
        max_tokens: args.max_tokens,
        discover_entity_types: args.discover_entity_types_enabled(),
        min_examples_required: args.min_examples_required,
        n_subset_max: args.n_subset_max,
        k: args.k,
        verbose: args.verbose,
        chunk_size: Some(args.chunk_size),
        overlap: Some(args.overlap),
        cache_enabled: false,
    })
}

async fn resolve_output_dir(root: &Path, output: &Path) -> Result<PathBuf> {
    let raw = if output.is_absolute() {
        absolute_unresolved(output)?
    } else {
        let root = absolute_unresolved(root)?;
        absolute_unresolved(&root.join(output))?
    };
    reject_symlink_ancestors(&raw).await?;
    reject_symlink_files(&raw).await?;
    let dir = normalize_path(&raw);

    if tokio::fs::try_exists(&dir)
        .await
        .map_err(|source| CliError::Io {
            operation: "check output directory",
            path: dir.clone(),
            source,
        })?
    {
        let metadata = tokio::fs::metadata(&dir)
            .await
            .map_err(|source| CliError::Io {
                operation: "stat output directory",
                path: dir.clone(),
                source,
            })?;
        if !metadata.is_dir() {
            return Err(CliError::InvalidRoot {
                path: dir,
                message: "output path must be a directory".to_owned(),
            });
        }
    } else {
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|source| CliError::Io {
                operation: "create output directory",
                path: dir.clone(),
                source,
            })?;
    }
    Ok(dir)
}

async fn reject_symlink_files(path: &Path) -> Result<()> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if is_symlink_or_reparse(&metadata) => Err(CliError::InvalidRoot {
            path: path.to_path_buf(),
            message: "refusing to write to symlink".to_owned(),
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

// ---------------------------------------------------------------------------
// Transactional publication
// ---------------------------------------------------------------------------

/// Publish prompts transactionally: either all three files are written, or none.
///
/// # Transaction guarantees
///
/// 1. Stage: write to temporary files with UUID names (`.tmp` suffix)
/// 2. Preflight: validate all target paths before touching any target
/// 3. Backup: move existing targets to `.backup` files
/// 4. Publish: rename staged files into target paths
/// 5. Cleanup: remove backup files
///
/// Any failure in steps 2-5 triggers rollback:
/// - Staged `.tmp` files are removed
/// - Published targets are removed and their backups restored
/// - Remaining staged files are cleaned up
async fn publish_prompts(
    output_dir: &Path,
    prompts: &crate::api::GeneratedIndexingPrompts,
) -> Result<()> {
    let files = [
        (EXTRACT_GRAPH_FILENAME, &prompts.extract_graph),
        (
            ENTITY_SUMMARIZATION_FILENAME,
            &prompts.summarize_descriptions,
        ),
        (
            COMMUNITY_SUMMARIZATION_FILENAME,
            &prompts.community_report_graph,
        ),
    ];

    // Phase 1: Validate all targets before modifying filesystem
    for (name, _) in &files {
        let target = output_dir.join(name);
        reject_symlink_files(&target).await?;
        if let Ok(metadata) = tokio::fs::metadata(&target).await
            && !metadata.is_file()
        {
            return Err(CliError::InvalidRoot {
                path: target,
                message: format!("refusing to overwrite non-file at {name}"),
            });
        }
    }

    // Phase 2: Write all staged files
    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (name, content) in &files {
        let target = output_dir.join(name);
        let staged_path = temporary_sibling(&target, "tmp")?;
        if let Err(error) = write_staged_file(&staged_path, content).await {
            for (_, staged_p) in &staged {
                cleanup_tmp(staged_p).await;
            }
            return Err(error);
        }
        staged.push((target, staged_path));
    }

    // Phase 3: Backup → rename → rollback-on-failure
    let mut published: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();

    for (idx, (target, staged_path)) in staged.iter().enumerate() {
        let target = target.clone();
        let staged_path = staged_path.clone();

        // Backup
        let backup = match move_target_to_backup(&target).await {
            Ok(backup) => backup,
            Err(error) => {
                let _ = rollback_published(&published, true).await;
                for (_, sp) in staged.iter().skip(idx + 1) {
                    cleanup_tmp(sp).await;
                }
                cleanup_tmp(&staged_path).await;
                return Err(error);
            }
        };

        // Publish: rename staged to target
        match tokio::fs::rename(&staged_path, &target).await {
            Ok(()) => {
                published.push((target.clone(), backup));
            }
            Err(source) => {
                let _ = restore_backup(&target, backup.as_deref()).await;
                let _ = rollback_published(&published, false).await;
                for (_, sp) in staged.iter().skip(idx + 1) {
                    cleanup_tmp(sp).await;
                }
                return Err(CliError::Io {
                    operation: "publish prompt file",
                    path: target,
                    source,
                });
            }
        }
    }

    // Phase 4: Clean up backups
    for (_, backup) in &published {
        if let Some(backup) = backup
            && let Err(source) = tokio::fs::remove_file(backup).await
            && source.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                path = %backup.display(),
                error = %source,
                "failed to remove prompt-tune backup file"
            );
        }
    }

    Ok(())
}

async fn write_staged_file(path: &Path, content: &str) -> Result<()> {
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .await
        .map_err(|source| CliError::Io {
            operation: "create staged prompt file",
            path: path.to_path_buf(),
            source,
        })?;
    if let Err(source) = file.write_all(content.as_bytes()).await {
        cleanup_tmp(path).await;
        return Err(CliError::Io {
            operation: "write staged prompt file",
            path: path.to_path_buf(),
            source,
        });
    }
    if let Err(source) = file.sync_all().await {
        cleanup_tmp(path).await;
        return Err(CliError::Io {
            operation: "sync staged prompt file",
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
            operation: "check prompt file before publish",
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
            operation: "backup prompt file",
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
                operation: "restore prompt file backup",
                path: target.to_path_buf(),
                source,
            })?;
    }
    Ok(())
}

async fn rollback_published(files: &[(PathBuf, Option<PathBuf>)], keep_staged: bool) -> Result<()> {
    let _ = keep_staged; // staged are handled separately
    for (target, backup) in files.iter().rev() {
        let _ = tokio::fs::remove_file(target).await;
        let _ = restore_backup(target, backup.as_deref()).await;
    }
    Ok(())
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
        tracing::warn!(
            path = %path.display(),
            error = %source,
            "failed to remove staged prompt-tune file"
        );
    }
}
