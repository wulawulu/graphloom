//! Isolated index generations and recoverable publication.

use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

use uuid::Uuid;

use super::{VectorLocation, io_error, vector_location};
use crate::{GraphLoomError, Result, path_safety::relative_descendant, project::LoadedProject};

/// Isolated output generation and the publication transaction that owns it.
#[derive(Debug)]
pub(crate) struct StagedIndexGeneration {
    project: LoadedProject,
    publication: IndexPublication,
}

impl StagedIndexGeneration {
    /// Build paths for a new generation without touching the active index.
    pub(crate) fn new(active: &LoadedProject, vector_store_enabled: bool) -> Result<Self> {
        active.paths.validate_destructive_paths()?;

        let staged_output = transaction_sibling(&active.paths.output_dir, "staging")?;
        let (staged_vector, external_vector, preserved_vectors) = if vector_store_enabled {
            active.paths.validate_vector_path_safety()?;
            match vector_location(&active.paths)? {
                VectorLocation::InsideOutput(relative) => {
                    (staged_output.join(relative), None, Vec::new())
                }
                VectorLocation::OutsideOutput => {
                    let staged = transaction_sibling(&active.paths.vector_db_uri, "staging")?;
                    (
                        staged.clone(),
                        Some((active.paths.vector_db_uri.clone(), staged)),
                        Vec::new(),
                    )
                }
            }
        } else {
            let preserved =
                relative_descendant(&active.paths.vector_db_uri, &active.paths.output_dir)?
                    .into_iter()
                    .collect();
            (active.paths.vector_db_uri.clone(), None, preserved)
        };

        let mut config = active.config.clone();
        config.output_storage.base_dir = staged_output.to_string_lossy().into_owned();
        config.vector_store.db_uri = staged_vector.to_string_lossy().into_owned();
        let project = LoadedProject::from_config(active.root.clone(), config)?;
        let mut targets = vec![PublicationTarget::replace_preserving(
            active.paths.output_dir.clone(),
            staged_output,
            preserved_vectors,
        )?];
        if let Some((live, staged)) = external_vector {
            targets.push(PublicationTarget::replace(live, staged));
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
    preserved_descendants: Vec<PathBuf>,
}

impl PublicationTarget {
    fn replace(live: PathBuf, staged: PathBuf) -> Self {
        Self {
            live,
            staged,
            preserved_descendants: Vec::new(),
        }
    }

    fn replace_preserving(
        live: PathBuf,
        staged: PathBuf,
        preserved_descendants: Vec<PathBuf>,
    ) -> Result<Self> {
        validate_preserved_descendants(&live, &preserved_descendants)?;
        Ok(Self {
            live,
            staged,
            preserved_descendants,
        })
    }
}

#[derive(Debug)]
struct PublishedTarget {
    live: PathBuf,
    backup: Option<PathBuf>,
    moved_descendants: Vec<PathBuf>,
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
        self.publish_with_hooks(&mut before_publish, |_, _, _| Ok(()), |_, _, _| Ok(()))
            .await
    }

    async fn publish_with_hooks<F, G, H>(
        &self,
        mut before_publish: F,
        mut before_preserved_move: G,
        mut before_preserved_restore: H,
    ) -> Result<()>
    where
        F: FnMut(usize) -> Result<()>,
        G: FnMut(usize, &Path, &Path) -> Result<()>,
        H: FnMut(usize, &Path, &Path) -> Result<()>,
    {
        let mut published = Vec::with_capacity(self.targets.len());
        for (index, target) in self.targets.iter().enumerate() {
            let backup = match backup_active_path(&target.live).await {
                Ok(backup) => backup,
                Err(error) => {
                    let rollback =
                        rollback_publication(&published, &mut before_preserved_restore).await;
                    self.cleanup().await;
                    return Err(with_rollback("publish index generation", error, rollback));
                }
            };
            if let Err(error) = before_publish(index) {
                let current_rollback = restore_active_path(&target.live, backup.as_deref()).await;
                let previous_rollback =
                    rollback_publication(&published, &mut before_preserved_restore).await;
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
                let previous_rollback =
                    rollback_publication(&published, &mut before_preserved_restore).await;
                self.cleanup().await;
                return Err(with_rollback(
                    "publish index generation",
                    error,
                    current_rollback.and(previous_rollback),
                ));
            }
            let mut current = PublishedTarget {
                live: target.live.clone(),
                backup,
                moved_descendants: Vec::new(),
            };
            if let Err(error) =
                move_preserved_descendants(index, target, &mut current, &mut before_preserved_move)
                    .await
            {
                let current_rollback =
                    rollback_target(index, &current, &mut before_preserved_restore).await;
                let previous_rollback =
                    rollback_publication(&published, &mut before_preserved_restore).await;
                self.cleanup().await;
                return Err(with_rollback(
                    "publish preserved index descendants",
                    error,
                    current_rollback.and(previous_rollback),
                ));
            }
            published.push(current);
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

async fn move_preserved_descendants(
    target_index: usize,
    target: &PublicationTarget,
    published: &mut PublishedTarget,
    before_move: &mut impl FnMut(usize, &Path, &Path) -> Result<()>,
) -> Result<()> {
    let Some(backup) = published.backup.as_deref() else {
        return Ok(());
    };
    for relative in &target.preserved_descendants {
        let source = backup.join(relative);
        if !tokio::fs::try_exists(&source)
            .await
            .map_err(|source_error| io_error("check preserved descendant", &source, source_error))?
        {
            continue;
        }
        let destination = target.live.join(relative);
        if tokio::fs::try_exists(&destination)
            .await
            .map_err(|source_error| {
                io_error("check preserved destination", &destination, source_error)
            })?
        {
            return Err(GraphLoomError::PreservedDescendantConflict { path: destination });
        }
    }
    for relative in &target.preserved_descendants {
        let source = backup.join(relative);
        if !tokio::fs::try_exists(&source)
            .await
            .map_err(|source_error| io_error("check preserved descendant", &source, source_error))?
        {
            continue;
        }
        let destination = target.live.join(relative);
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|source_error| {
                    io_error("create preserved destination parent", parent, source_error)
                })?;
        }
        before_move(target_index, &source, &destination)?;
        tokio::fs::rename(&source, &destination)
            .await
            .map_err(|source_error| GraphLoomError::PreservedDescendantMove {
                operation: "move preserved descendant",
                source_path: source,
                destination_path: destination,
                source: source_error,
            })?;
        published.moved_descendants.push(relative.clone());
    }
    Ok(())
}

fn validate_preserved_descendants(target: &Path, descendants: &[PathBuf]) -> Result<()> {
    let mut validated: BTreeSet<PathBuf> = BTreeSet::new();
    for descendant in descendants {
        let valid = !descendant.as_os_str().is_empty()
            && !descendant.is_absolute()
            && descendant
                .components()
                .all(|component| matches!(component, Component::Normal(_)));
        if !valid {
            return Err(GraphLoomError::InvalidPreservedDescendant {
                target: target.to_path_buf(),
                descendant: descendant.clone(),
                message: "path must be a non-empty relative descendant without parent, root, or \
                          prefix components"
                    .to_owned(),
            });
        }
        for existing in &validated {
            if descendant.starts_with(existing) || existing.starts_with(descendant) {
                return Err(GraphLoomError::InvalidPreservedDescendant {
                    target: target.to_path_buf(),
                    descendant: descendant.clone(),
                    message: format!("path overlaps preserved descendant {}", existing.display()),
                });
            }
        }
        validated.insert(descendant.clone());
    }
    Ok(())
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

async fn rollback_publication(
    targets: &[PublishedTarget],
    before_restore: &mut impl FnMut(usize, &Path, &Path) -> Result<()>,
) -> Result<()> {
    for (index, target) in targets.iter().enumerate().rev() {
        rollback_target(index, target, before_restore).await?;
    }
    Ok(())
}

async fn rollback_target(
    target_index: usize,
    target: &PublishedTarget,
    before_restore: &mut impl FnMut(usize, &Path, &Path) -> Result<()>,
) -> Result<()> {
    restore_preserved_descendants(target_index, target, before_restore).await?;
    remove_path_if_exists(&target.live).await?;
    restore_active_path(&target.live, target.backup.as_deref()).await
}

async fn restore_preserved_descendants(
    target_index: usize,
    target: &PublishedTarget,
    before_restore: &mut impl FnMut(usize, &Path, &Path) -> Result<()>,
) -> Result<()> {
    if let Some(backup) = target.backup.as_deref() {
        for relative in target.moved_descendants.iter().rev() {
            let source = target.live.join(relative);
            let destination = backup.join(relative);
            if let Some(parent) = destination.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|source_error| {
                        io_error("restore preserved descendant parent", parent, source_error)
                    })?;
            }
            before_restore(target_index, &source, &destination)?;
            tokio::fs::rename(&source, &destination)
                .await
                .map_err(|source_error| GraphLoomError::PreservedDescendantMove {
                    operation: "restore preserved descendant",
                    source_path: source,
                    destination_path: destination,
                    source: source_error,
                })?;
        }
    }
    Ok(())
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
        write_marker(&active_output.join("lancedb"), "old-nested-vector").await;
        write_marker(&active_vector, "old-vector").await;
        write_marker(&staged_output, "new-output").await;
        write_marker(&staged_vector, "new-vector").await;
        let publication = IndexPublication {
            targets: vec![
                PublicationTarget::replace_preserving(
                    active_output.clone(),
                    staged_output,
                    vec![PathBuf::from("lancedb")],
                )
                .expect("preserved target"),
                PublicationTarget::replace(active_vector.clone(), staged_vector),
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
        assert_eq!(
            read_marker(&active_output.join("lancedb")).await,
            "old-nested-vector"
        );
        assert_eq!(read_marker(&active_vector).await, "old-vector");
        assert_only_active_directories(tempdir.path(), &["output", "vector"]).await;
    }

    #[tokio::test]
    async fn test_should_publish_while_preserving_inactive_descendant() {
        let tempdir = TempDir::new().expect("tempdir");
        let live = tempdir.path().join("output");
        let staged = tempdir.path().join("output-staged");
        write_marker(&live, "old-output").await;
        write_marker(&live.join("lancedb"), "old-vector").await;
        write_marker(&staged, "new-output").await;
        let publication = IndexPublication {
            targets: vec![
                PublicationTarget::replace_preserving(
                    live.clone(),
                    staged,
                    vec![PathBuf::from("lancedb")],
                )
                .expect("preserved target"),
            ],
        };

        publication.publish().await.expect("publication");

        assert_eq!(read_marker(&live).await, "new-output");
        assert_eq!(read_marker(&live.join("lancedb")).await, "old-vector");
        assert_only_active_directories(tempdir.path(), &["output"]).await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn test_should_preserve_inactive_nested_vector_case_insensitively() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut config = crate::GraphRagConfig::default();
        config.output_storage.base_dir =
            tempdir.path().join("Output").to_string_lossy().into_owned();
        config.vector_store.db_uri = tempdir
            .path()
            .join("output")
            .join("lancedb")
            .to_string_lossy()
            .into_owned();
        let active = LoadedProject::from_config(tempdir.path(), config).expect("active project");
        let generation = StagedIndexGeneration::new(&active, false).expect("generation");
        let (staged, publication) = generation.into_parts();
        assert_eq!(
            publication.targets[0].preserved_descendants,
            vec![PathBuf::from("lancedb")]
        );

        write_marker(&active.paths.output_dir, "old-output").await;
        write_marker(&active.paths.output_dir.join("lancedb"), "old-vector").await;
        write_marker(&staged.paths.output_dir, "new-output").await;
        publication.publish().await.expect("publication");

        assert_eq!(read_marker(&active.paths.output_dir).await, "new-output");
        assert_eq!(
            read_marker(&active.paths.output_dir.join("lancedb")).await,
            "old-vector"
        );
        assert_only_active_directories(tempdir.path(), &["Output"]).await;
    }

    #[tokio::test]
    async fn test_should_rollback_when_preserved_destination_conflicts() {
        let tempdir = TempDir::new().expect("tempdir");
        let live = tempdir.path().join("output");
        let staged = tempdir.path().join("output-staged");
        write_marker(&live, "old-output").await;
        write_marker(&live.join("lancedb"), "old-vector").await;
        write_marker(&staged, "new-output").await;
        write_marker(&staged.join("lancedb"), "new-vector").await;
        let publication = IndexPublication {
            targets: vec![
                PublicationTarget::replace_preserving(
                    live.clone(),
                    staged,
                    vec![PathBuf::from("lancedb")],
                )
                .expect("preserved target"),
            ],
        };

        let error = publication.publish().await.expect_err("conflict must fail");

        assert!(error.to_string().contains("destination"));
        assert_eq!(read_marker(&live).await, "old-output");
        assert_eq!(read_marker(&live.join("lancedb")).await, "old-vector");
        assert_only_active_directories(tempdir.path(), &["output"]).await;
    }

    #[tokio::test]
    async fn test_should_rollback_when_preserved_move_fails() {
        let tempdir = TempDir::new().expect("tempdir");
        let live = tempdir.path().join("output");
        let staged = tempdir.path().join("output-staged");
        write_marker(&live, "old-output").await;
        write_marker(&live.join("lancedb"), "old-vector").await;
        write_marker(&staged, "new-output").await;
        let publication = IndexPublication {
            targets: vec![
                PublicationTarget::replace_preserving(
                    live.clone(),
                    staged,
                    vec![PathBuf::from("lancedb")],
                )
                .expect("preserved target"),
            ],
        };

        let error = publication
            .publish_with_hooks(
                |_| Ok(()),
                |_, _, destination| {
                    Err(GraphLoomError::Io {
                        operation: "inject preserved move failure",
                        path: destination.to_path_buf(),
                        source: std::io::Error::other("injected preserved move failure"),
                    })
                },
                |_, _, _| Ok(()),
            )
            .await
            .expect_err("preserved move should fail");

        assert!(
            error
                .to_string()
                .contains("injected preserved move failure")
        );
        assert_eq!(read_marker(&live).await, "old-output");
        assert_eq!(read_marker(&live.join("lancedb")).await, "old-vector");
        assert_only_active_directories(tempdir.path(), &["output"]).await;
    }

    #[tokio::test]
    async fn test_should_keep_live_and_backup_when_preserved_restore_fails() {
        let tempdir = TempDir::new().expect("tempdir");
        let live = tempdir.path().join("output");
        let staged = tempdir.path().join("output-staged");
        let second_live = tempdir.path().join("vector");
        let second_staged = tempdir.path().join("vector-staged");
        write_marker(&live, "old-output").await;
        write_marker(&live.join("lancedb"), "old-vector").await;
        write_marker(&staged, "new-output").await;
        write_marker(&second_live, "old-external-vector").await;
        write_marker(&second_staged, "new-external-vector").await;
        let publication = IndexPublication {
            targets: vec![
                PublicationTarget::replace_preserving(
                    live.clone(),
                    staged,
                    vec![PathBuf::from("lancedb")],
                )
                .expect("preserved target"),
                PublicationTarget::replace(second_live.clone(), second_staged),
            ],
        };

        let error = publication
            .publish_with_hooks(
                |index| {
                    if index == 1 {
                        return Err(GraphLoomError::Io {
                            operation: "inject later publication failure",
                            path: second_live.clone(),
                            source: std::io::Error::other("injected publication failure"),
                        });
                    }
                    Ok(())
                },
                |_, _, _| Ok(()),
                |index, source, destination| {
                    if index == 0 {
                        return Err(GraphLoomError::Io {
                            operation: "inject preserved restore failure",
                            path: destination.to_path_buf(),
                            source: std::io::Error::other(format!(
                                "injected restore failure moving {} to {}",
                                source.display(),
                                destination.display(),
                            )),
                        });
                    }
                    Ok(())
                },
            )
            .await
            .expect_err("rollback must report the preserved restore failure");

        assert!(matches!(error, GraphLoomError::RollbackFailed { .. }));
        assert!(error.to_string().contains("injected publication failure"));
        assert!(error.to_string().contains("injected restore failure"));
        assert_eq!(read_marker(&live).await, "new-output");
        assert_eq!(read_marker(&live.join("lancedb")).await, "old-vector");
        assert_eq!(read_marker(&second_live).await, "old-external-vector");

        let backup = find_single_backup(tempdir.path(), "output").await;
        assert_eq!(read_marker(&backup).await, "old-output");
        assert!(tokio::fs::try_exists(&live).await.expect("live existence"));
        assert!(
            tokio::fs::try_exists(&backup)
                .await
                .expect("backup existence")
        );
    }

    #[test]
    fn test_should_reject_overlapping_or_unsafe_preserved_descendants() {
        let target = PathBuf::from("output");
        let staged = PathBuf::from("staged");
        assert!(
            PublicationTarget::replace_preserving(
                target.clone(),
                staged.clone(),
                vec![PathBuf::from("../vector")],
            )
            .is_err()
        );
        assert!(
            PublicationTarget::replace_preserving(
                target,
                staged,
                vec![PathBuf::from("lancedb"), PathBuf::from("lancedb/nested")],
            )
            .is_err()
        );
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

    async fn find_single_backup(root: &Path, live_name: &str) -> PathBuf {
        let prefix = format!(".{live_name}.");
        let mut backups = Vec::new();
        let mut entries = tokio::fs::read_dir(root).await.expect("root entries");
        while let Some(entry) = entries.next_entry().await.expect("entry") {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) && name.ends_with(".backup") {
                backups.push(entry.path());
            }
        }
        assert_eq!(backups.len(), 1, "expected exactly one retained backup");
        backups.remove(0)
    }
}
