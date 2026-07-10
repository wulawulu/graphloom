//! Isolated index generations and recoverable publication.

use std::path::{Path, PathBuf};

use uuid::Uuid;

use super::{VectorLocation, io_error, vector_location};
use crate::{GraphLoomError, Result, project::LoadedProject};

/// Isolated output generation and the publication transaction that owns it.
#[derive(Debug)]
pub(crate) struct StagedIndexGeneration {
    project: LoadedProject,
    publication: IndexPublication,
}

impl StagedIndexGeneration {
    /// Build paths for a new generation without touching the active index.
    pub(crate) fn new(active: &LoadedProject) -> Result<Self> {
        active.paths.validate_destructive_paths()?;
        active.paths.validate_vector_path_safety()?;

        let staged_output = transaction_sibling(&active.paths.output_dir, "staging")?;
        let vector_location = vector_location(&active.paths)?;
        let (staged_vector, external_vector) = match vector_location {
            VectorLocation::InsideOutput => {
                let relative =
                    relative_descendant(&active.paths.vector_db_uri, &active.paths.output_dir)?;
                (staged_output.join(relative), None)
            }
            VectorLocation::OutsideOutput => {
                let staged = transaction_sibling(&active.paths.vector_db_uri, "staging")?;
                (
                    staged.clone(),
                    Some((active.paths.vector_db_uri.clone(), staged)),
                )
            }
        };

        let mut config = active.config.clone();
        config.output_storage.base_dir = staged_output.to_string_lossy().into_owned();
        config.vector_store.db_uri = staged_vector.to_string_lossy().into_owned();
        let project = LoadedProject::from_config(active.root.clone(), config)?;
        let mut targets = vec![PublicationTarget::new(
            active.paths.output_dir.clone(),
            staged_output,
        )];
        if let Some((live, staged)) = external_vector {
            targets.push(PublicationTarget::new(live, staged));
        }

        Ok(Self {
            project,
            publication: IndexPublication { targets },
        })
    }

    pub(crate) fn into_parts(self) -> (LoadedProject, IndexPublication) {
        (self.project, self.publication)
    }
}

/// Recoverable replacement of one or more active index directories.
#[derive(Debug)]
pub(crate) struct IndexPublication {
    targets: Vec<PublicationTarget>,
}

#[derive(Debug, Clone)]
struct PublicationTarget {
    live: PathBuf,
    staged: PathBuf,
}

impl PublicationTarget {
    fn new(live: PathBuf, staged: PathBuf) -> Self {
        Self { live, staged }
    }
}

#[derive(Debug)]
struct PublishedTarget {
    live: PathBuf,
    backup: Option<PathBuf>,
}

impl IndexPublication {
    /// Publish the completed generation, restoring every active path on error.
    pub(crate) async fn publish(self) -> Result<()> {
        self.publish_with_hook(|_| Ok(())).await
    }

    async fn publish_with_hook<F>(&self, mut before_publish: F) -> Result<()>
    where
        F: FnMut(usize) -> Result<()>,
    {
        let mut published = Vec::with_capacity(self.targets.len());
        for (index, target) in self.targets.iter().enumerate() {
            let backup = match backup_active_path(&target.live).await {
                Ok(backup) => backup,
                Err(error) => {
                    let rollback = rollback_publication(&published).await;
                    self.cleanup().await;
                    return Err(with_rollback("publish index generation", error, rollback));
                }
            };
            if let Err(error) = before_publish(index) {
                let current_rollback = restore_active_path(&target.live, backup.as_deref()).await;
                let previous_rollback = rollback_publication(&published).await;
                self.cleanup().await;
                return Err(with_rollback(
                    "publish index generation",
                    error,
                    current_rollback.and(previous_rollback),
                ));
            }
            if let Err(source) = tokio::fs::rename(&target.staged, &target.live).await {
                let error = io_error("publish staged index generation", &target.live, source);
                let current_rollback = restore_active_path(&target.live, backup.as_deref()).await;
                let previous_rollback = rollback_publication(&published).await;
                self.cleanup().await;
                return Err(with_rollback(
                    "publish index generation",
                    error,
                    current_rollback.and(previous_rollback),
                ));
            }
            published.push(PublishedTarget {
                live: target.live.clone(),
                backup,
            });
        }
        remove_publication_backups(&published).await;
        Ok(())
    }

    /// Remove an unpublished generation without changing the active index.
    pub(crate) async fn cleanup(&self) {
        for target in &self.targets {
            if let Err(error) = remove_path_if_exists(&target.staged).await {
                tracing::warn!(path = %target.staged.display(), error = %error, "failed to clean unpublished index generation");
            }
        }
    }
}

fn relative_descendant(path: &Path, parent: &Path) -> Result<PathBuf> {
    let path_components = path.components().collect::<Vec<_>>();
    let parent_len = parent.components().count();
    if path_components.len() <= parent_len {
        return Err(GraphLoomError::UnsafeOutputPath {
            path: path.to_path_buf(),
            message: "vector DB path must be a strict descendant of output".to_owned(),
        });
    }
    Ok(path_components
        .iter()
        .skip(parent_len)
        .map(|component| component.as_os_str())
        .collect())
}

fn transaction_sibling(path: &Path, kind: &str) -> Result<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| GraphLoomError::UnsafeOutputPath {
            path: path.to_path_buf(),
            message: "managed index path must have a parent".to_owned(),
        })?;
    let name = path
        .file_name()
        .ok_or_else(|| GraphLoomError::UnsafeOutputPath {
            path: path.to_path_buf(),
            message: "managed index path must have a file name".to_owned(),
        })?;
    Ok(parent.join(format!(
        ".{}.{}.{}",
        name.to_string_lossy(),
        Uuid::new_v4(),
        kind,
    )))
}

async fn backup_active_path(path: &Path) -> Result<Option<PathBuf>> {
    if !tokio::fs::try_exists(path)
        .await
        .map_err(|source| io_error("check active index path", path, source))?
    {
        return Ok(None);
    }
    let backup = transaction_sibling(path, "backup")?;
    tokio::fs::rename(path, &backup)
        .await
        .map_err(|source| io_error("backup active index path", path, source))?;
    Ok(Some(backup))
}

async fn restore_active_path(live: &Path, backup: Option<&Path>) -> Result<()> {
    if let Some(backup) = backup {
        tokio::fs::rename(backup, live)
            .await
            .map_err(|source| io_error("restore active index path", live, source))?;
    }
    Ok(())
}

async fn rollback_publication(targets: &[PublishedTarget]) -> Result<()> {
    let mut first_error = None;
    for target in targets.iter().rev() {
        if let Err(error) = remove_path_if_exists(&target.live).await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        if let Err(error) = restore_active_path(&target.live, target.backup.as_deref()).await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

async fn remove_publication_backups(targets: &[PublishedTarget]) {
    for target in targets {
        if let Some(backup) = &target.backup
            && let Err(source) = tokio::fs::remove_dir_all(backup).await
        {
            tracing::warn!(path = %backup.display(), error = %source, "failed to remove obsolete index backup");
        }
    }
}

async fn remove_path_if_exists(path: &Path) -> Result<()> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.is_dir() => {
            tokio::fs::remove_dir_all(path)
                .await
                .map_err(|source| io_error("remove index directory", path, source))?;
        }
        Ok(_) => {
            tokio::fs::remove_file(path)
                .await
                .map_err(|source| io_error("remove index file", path, source))?;
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(io_error("inspect index path for removal", path, source)),
    }
    Ok(())
}

fn with_rollback(
    operation: &'static str,
    source: GraphLoomError,
    rollback: Result<()>,
) -> GraphLoomError {
    match rollback {
        Ok(()) => source,
        Err(rollback) => GraphLoomError::RollbackFailed {
            operation,
            source: Box::new(source),
            rollback: Box::new(rollback),
        },
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn test_should_restore_all_active_directories_when_publication_fails() {
        let tempdir = TempDir::new().expect("tempdir");
        let active_output = tempdir.path().join("output");
        let active_vector = tempdir.path().join("vector");
        let staged_output = tempdir.path().join("output-staged");
        let staged_vector = tempdir.path().join("vector-staged");
        write_marker(&active_output, "old-output").await;
        write_marker(&active_vector, "old-vector").await;
        write_marker(&staged_output, "new-output").await;
        write_marker(&staged_vector, "new-vector").await;
        let publication = IndexPublication {
            targets: vec![
                PublicationTarget::new(active_output.clone(), staged_output),
                PublicationTarget::new(active_vector.clone(), staged_vector),
            ],
        };

        let error = publication
            .publish_with_hook(|index| {
                if index == 1 {
                    return Err(GraphLoomError::Io {
                        operation: "inject publication failure",
                        path: active_vector.clone(),
                        source: std::io::Error::other("injected failure"),
                    });
                }
                Ok(())
            })
            .await
            .expect_err("publication should fail");

        assert!(error.to_string().contains("injected failure"));
        assert_eq!(read_marker(&active_output).await, "old-output");
        assert_eq!(read_marker(&active_vector).await, "old-vector");
        assert_only_active_directories(tempdir.path(), &["output", "vector"]).await;
    }

    async fn write_marker(directory: &Path, value: &str) {
        tokio::fs::create_dir(directory).await.expect("directory");
        tokio::fs::write(directory.join("marker"), value)
            .await
            .expect("marker");
    }

    async fn read_marker(directory: &Path) -> String {
        tokio::fs::read_to_string(directory.join("marker"))
            .await
            .expect("marker")
    }

    async fn assert_only_active_directories(root: &Path, expected: &[&str]) {
        let mut names = Vec::new();
        let mut entries = tokio::fs::read_dir(root).await.expect("root entries");
        while let Some(entry) = entries.next_entry().await.expect("entry") {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        assert_eq!(names, expected);
    }
}
