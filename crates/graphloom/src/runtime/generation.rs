//! Isolated index generations and recoverable publication.

use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

use uuid::Uuid;

use super::{VectorLocation, io_error, vector_location};
use crate::{
    GraphLoomError, Result,
    path_safety::{
        reject_descendant_link_components, relative_descendant,
        validate_existing_publication_target, validate_publication_directory_root,
        validate_publication_target_metadata,
    },
    project::LoadedProject,
};

/// Isolated output generation and the publication transaction that owns it.
#[derive(Debug)]
pub(crate) struct StagedIndexGeneration {
    project: LoadedProject,
    publication: IndexPublication,
}

impl StagedIndexGeneration {
    /// Build paths for a new generation without touching the active index.
    pub(crate) async fn new(active: &LoadedProject, vector_store_enabled: bool) -> Result<Self> {
        active.paths.validate_destructive_paths()?;
        validate_existing_publication_target(&active.paths.output_dir, "output publication")
            .await?;

        let staged_output = transaction_sibling(&active.paths.output_dir, "staging")?;
        let (staged_vector, external_vector, preserved_vectors) = if vector_store_enabled {
            active.paths.validate_vector_path_safety()?;
            match vector_location(&active.paths)? {
                VectorLocation::InsideOutput(relative) => {
                    (staged_output.join(relative), None, Vec::new())
                }
                VectorLocation::OutsideOutput => {
                    validate_existing_publication_target(
                        &active.paths.vector_db_uri,
                        "vector DB publication",
                    )
                    .await?;
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
        self.publish_with_hooks(
            &mut before_publish,
            |_, _, _| Ok(()),
            |_, _, _| Ok(()),
            |_, _, _| Ok(()),
        )
        .await
    }

    async fn publish_with_hooks<F, G, H, I>(
        &self,
        mut before_publish: F,
        mut before_preserved_move: G,
        mut before_preserved_restore: H,
        mut before_backup_restore: I,
    ) -> Result<()>
    where
        F: FnMut(usize) -> Result<()>,
        G: FnMut(usize, &Path, &Path) -> Result<()>,
        H: FnMut(usize, &Path, &Path) -> Result<()>,
        I: FnMut(usize, &Path, Option<&Path>) -> Result<()>,
    {
        let mut published = Vec::with_capacity(self.targets.len());
        for (index, target) in self.targets.iter().enumerate() {
            let backup =
                match backup_active_path(&target.live, "validate publication target before backup")
                    .await
                {
                    Ok(backup) => backup,
                    Err(error) => {
                        let rollback =
                            rollback_publication(&published, &mut before_preserved_restore).await;
                        self.cleanup().await;
                        return Err(with_rollback("publish index generation", error, rollback));
                    }
                };
            if let Err(error) = before_publish(index) {
                let error = rollback_failed_publication_target(
                    index,
                    &target.live,
                    backup.as_deref(),
                    &published,
                    error,
                    &mut before_backup_restore,
                    &mut before_preserved_restore,
                )
                .await;
                self.cleanup().await;
                return Err(error);
            }
            // Recheck immediately before publishing the staged directory. This prevents normal
            // empty-directory replacement but cannot fully eliminate TOCTOU races without
            // platform-specific no-replace rename operations.
            if let Err(error) = validate_live_absent_before_staged_publish(&target.live).await {
                let error = rollback_failed_publication_target(
                    index,
                    &target.live,
                    backup.as_deref(),
                    &published,
                    error,
                    &mut before_backup_restore,
                    &mut before_preserved_restore,
                )
                .await;
                self.cleanup().await;
                return Err(error);
            }
            if let Err(source) = tokio::fs::rename(&target.staged, &target.live).await {
                let error = io_error("publish staged index generation", &target.live, source);
                let error = rollback_failed_publication_target(
                    index,
                    &target.live,
                    backup.as_deref(),
                    &published,
                    error,
                    &mut before_backup_restore,
                    &mut before_preserved_restore,
                )
                .await;
                self.cleanup().await;
                return Err(error);
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
                let rollback = rollback_published_then_previous(
                    index,
                    &current,
                    &published,
                    &mut before_preserved_restore,
                )
                .await;
                self.cleanup().await;
                return Err(with_rollback(
                    "publish preserved index descendants",
                    error,
                    rollback,
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

async fn validate_live_absent_before_staged_publish(live: &Path) -> Result<()> {
    match tokio::fs::symlink_metadata(live).await {
        Ok(_) => Err(GraphLoomError::Io {
            operation: "publish staged index generation",
            path: live.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "publication live root unexpectedly exists before staged publish",
            ),
        }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(
            "inspect publication live root before staged publish",
            live,
            source,
        )),
    }
}

async fn rollback_failed_publication_target(
    current_index: usize,
    live: &Path,
    backup: Option<&Path>,
    previous: &[PublishedTarget],
    error: GraphLoomError,
    before_backup_restore: &mut impl FnMut(usize, &Path, Option<&Path>) -> Result<()>,
    before_restore: &mut impl FnMut(usize, &Path, &Path) -> Result<()>,
) -> GraphLoomError {
    let rollback = rollback_backup_then_previous(
        current_index,
        live,
        backup,
        previous,
        before_backup_restore,
        before_restore,
    )
    .await;
    with_rollback("publish index generation", error, rollback)
}

async fn rollback_backup_then_previous(
    current_index: usize,
    live: &Path,
    backup: Option<&Path>,
    previous: &[PublishedTarget],
    before_backup_restore: &mut impl FnMut(usize, &Path, Option<&Path>) -> Result<()>,
    before_restore: &mut impl FnMut(usize, &Path, &Path) -> Result<()>,
) -> Result<()> {
    let current_rollback =
        rollback_current_backup_only(current_index, live, backup, before_backup_restore).await;
    let previous_rollback = rollback_publication(previous, before_restore).await;
    combine_rollback_results(current_rollback, previous_rollback)
}

async fn rollback_current_backup_only(
    current_index: usize,
    live: &Path,
    backup: Option<&Path>,
    before_backup_restore: &mut impl FnMut(usize, &Path, Option<&Path>) -> Result<()>,
) -> Result<()> {
    before_backup_restore(current_index, live, backup)?;
    let Some(backup) = backup else {
        // The target did not exist before publication, so there is no previous state to restore.
        // Preserve any newly appeared live target; the original publication conflict describes it.
        return Ok(());
    };
    validate_publication_directory_root(backup, "validate backup-only rollback root").await?;
    if path_exists_without_following_links(live).await? {
        return Err(GraphLoomError::Io {
            operation: "restore backup-only rollback root",
            path: live.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "publication live root unexpectedly exists before backup restore",
            ),
        });
    }
    restore_active_path(live, Some(backup)).await
}

fn combine_rollback_results(current: Result<()>, previous: Result<()>) -> Result<()> {
    match (current, previous) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(current), Ok(())) => Err(current),
        (Ok(()), Err(previous)) => Err(previous),
        (Err(current), Err(previous)) => Err(GraphLoomError::RollbackFailed {
            operation: "rollback current and previous publication targets",
            source: Box::new(current),
            rollback: Box::new(previous),
        }),
    }
}

async fn rollback_published_then_previous(
    current_index: usize,
    current: &PublishedTarget,
    previous: &[PublishedTarget],
    before_restore: &mut impl FnMut(usize, &Path, &Path) -> Result<()>,
) -> Result<()> {
    rollback_target(current_index, current, before_restore).await?;
    rollback_publication(previous, before_restore).await
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
        reject_descendant_link_components(backup, relative, "move preserved descendant source")
            .await?;
        if !path_exists_without_following_links(&source).await? {
            continue;
        }
        let destination = target.live.join(relative);
        reject_descendant_link_components(
            &target.live,
            relative,
            "create preserved descendant destination",
        )
        .await?;
        if path_exists_without_following_links(&destination).await? {
            return Err(GraphLoomError::PreservedDescendantConflict { path: destination });
        }
    }
    for relative in &target.preserved_descendants {
        let source = backup.join(relative);
        reject_descendant_link_components(backup, relative, "move preserved descendant source")
            .await?;
        if !path_exists_without_following_links(&source).await? {
            continue;
        }
        let destination = target.live.join(relative);
        reject_descendant_link_components(
            &target.live,
            relative,
            "create preserved descendant destination",
        )
        .await?;
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|source_error| {
                    io_error("create preserved destination parent", parent, source_error)
                })?;
        }
        before_move(target_index, &source, &destination)?;
        // Rechecking both sides after parent creation and test hooks narrows, but cannot fully
        // eliminate, filesystem TOCTOU races. Handle-based crash-safe publication is out of scope.
        reject_descendant_link_components(backup, relative, "move preserved descendant source")
            .await?;
        reject_descendant_link_components(
            &target.live,
            relative,
            "create preserved descendant destination",
        )
        .await?;
        if path_exists_without_following_links(&destination).await? {
            return Err(GraphLoomError::PreservedDescendantConflict { path: destination });
        }
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

async fn path_exists_without_following_links(path: &Path) -> Result<bool> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(io_error("inspect preserved descendant path", path, source)),
    }
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

async fn backup_active_path(path: &Path, operation: &'static str) -> Result<Option<PathBuf>> {
    // Revalidate immediately before the destructive rename. This narrows, but cannot eliminate,
    // filesystem TOCTOU races without handle-based operations.
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => validate_publication_target_metadata(&metadata, path, operation)?,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(io_error(operation, path, source)),
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
    validate_rollback_roots(target).await?;
    restore_preserved_descendants(target_index, target, before_restore).await?;
    // Descendant restoration and hooks can race with external root replacement. Revalidate
    // immediately before destructive removal; this narrows but cannot eliminate TOCTOU races.
    validate_rollback_roots(target).await?;
    remove_path_if_exists(&target.live).await?;
    restore_active_path(&target.live, target.backup.as_deref()).await
}

async fn validate_rollback_roots(target: &PublishedTarget) -> Result<()> {
    validate_publication_directory_root(&target.live, "validate rollback live root").await?;
    if let Some(backup) = target.backup.as_deref() {
        validate_publication_directory_root(backup, "validate rollback backup root").await?;
    }
    Ok(())
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
            reject_descendant_link_components(
                &target.live,
                relative,
                "restore preserved descendant source",
            )
            .await?;
            reject_descendant_link_components(
                backup,
                relative,
                "restore preserved descendant destination",
            )
            .await?;
            if let Some(parent) = destination.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|source_error| {
                        io_error("restore preserved descendant parent", parent, source_error)
                    })?;
            }
            before_restore(target_index, &source, &destination)?;
            // Recheck after creating the destination parent and immediately before rename. This
            // reduces, but does not eliminate, filesystem TOCTOU races.
            reject_descendant_link_components(
                &target.live,
                relative,
                "restore preserved descendant source",
            )
            .await?;
            reject_descendant_link_components(
                backup,
                relative,
                "restore preserved descendant destination",
            )
            .await?;
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
        if let Some(backup) = &target.backup {
            if let Err(error) =
                validate_publication_directory_root(backup, "validate obsolete publication backup")
                    .await
            {
                tracing::warn!(path = %backup.display(), error = %error, "obsolete index backup violated the directory invariant");
                continue;
            }
            if let Err(source) = tokio::fs::remove_dir_all(backup).await {
                tracing::warn!(path = %backup.display(), error = %source, "failed to remove obsolete index backup");
            }
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
    async fn test_should_defensively_reject_file_publication_targets() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut config = crate::GraphRagConfig::default();
        config.vector_store.db_uri = "vector-db".to_owned();
        let active = LoadedProject::from_config(tempdir.path(), config).expect("active project");

        tokio::fs::write(&active.paths.output_dir, "output file")
            .await
            .expect("output file");
        let error = StagedIndexGeneration::new(&active, true)
            .await
            .expect_err("output file must be rejected");
        assert!(error.to_string().contains("output publication"));
        assert!(error.to_string().contains("not a directory"));
        assert_eq!(
            tokio::fs::read_to_string(&active.paths.output_dir)
                .await
                .expect("output file contents"),
            "output file"
        );

        tokio::fs::remove_file(&active.paths.output_dir)
            .await
            .expect("remove output file");
        tokio::fs::create_dir(&active.paths.output_dir)
            .await
            .expect("output directory");
        tokio::fs::write(&active.paths.vector_db_uri, "vector file")
            .await
            .expect("vector file");
        let error = StagedIndexGeneration::new(&active, true)
            .await
            .expect_err("external vector file must be rejected");
        assert!(error.to_string().contains("vector DB publication"));
        assert!(error.to_string().contains("not a directory"));
        assert_eq!(
            tokio::fs::read_to_string(&active.paths.vector_db_uri)
                .await
                .expect("vector file contents"),
            "vector file"
        );
    }

    #[tokio::test]
    async fn test_should_reject_output_file_at_backup_boundary() {
        let tempdir = TempDir::new().expect("tempdir");
        let active = LoadedProject::from_config(tempdir.path(), crate::GraphRagConfig::default())
            .expect("active project");
        write_marker(&active.paths.output_dir, "old-output").await;
        let generation = StagedIndexGeneration::new(&active, false)
            .await
            .expect("generation");
        let (staged, publication) = generation.into_parts();
        let staged_output = staged.paths.output_dir.clone();
        write_marker(&staged_output, "new-output").await;
        tokio::fs::remove_dir_all(&active.paths.output_dir)
            .await
            .expect("replace output directory");
        tokio::fs::write(&active.paths.output_dir, "external output file")
            .await
            .expect("replacement output file");

        let error = publication
            .publish()
            .await
            .expect_err("replacement output file must be rejected");

        assert!(error.to_string().contains("publication target"));
        assert!(error.to_string().contains("not a directory"));
        assert!(active.paths.output_dir.is_file());
        assert_eq!(
            tokio::fs::read_to_string(&active.paths.output_dir)
                .await
                .expect("replacement output contents"),
            "external output file"
        );
        assert!(!staged_output.exists());
        assert_no_transaction_residue(tempdir.path()).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_should_reject_output_symlink_at_backup_boundary() {
        use std::os::unix::fs::symlink;

        let tempdir = TempDir::new().expect("tempdir");
        let external = TempDir::new().expect("external");
        let active = LoadedProject::from_config(tempdir.path(), crate::GraphRagConfig::default())
            .expect("active project");
        write_marker(&active.paths.output_dir, "old-output").await;
        write_marker(external.path(), "external-output").await;
        let generation = StagedIndexGeneration::new(&active, false)
            .await
            .expect("generation");
        let (staged, publication) = generation.into_parts();
        let staged_output = staged.paths.output_dir.clone();
        write_marker(&staged_output, "new-output").await;
        tokio::fs::remove_dir_all(&active.paths.output_dir)
            .await
            .expect("replace output directory");
        symlink(external.path(), &active.paths.output_dir).expect("replacement output symlink");

        let error = publication
            .publish()
            .await
            .expect_err("replacement output symlink must be rejected");

        assert!(error.to_string().contains("symlink or reparse point"));
        assert!(
            tokio::fs::symlink_metadata(&active.paths.output_dir)
                .await
                .expect("output symlink metadata")
                .file_type()
                .is_symlink()
        );
        assert_eq!(read_marker(external.path()).await, "external-output");
        assert!(!staged_output.exists());
        assert_no_transaction_residue(tempdir.path()).await;
        assert_no_transaction_residue(external.path()).await;
    }

    #[tokio::test]
    async fn test_should_rollback_output_when_external_vector_becomes_file_before_backup() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut config = crate::GraphRagConfig::default();
        config.vector_store.db_uri = "vector-db".to_owned();
        let active = LoadedProject::from_config(tempdir.path(), config).expect("active project");
        write_marker(&active.paths.output_dir, "old-output").await;
        let generation = StagedIndexGeneration::new(&active, true)
            .await
            .expect("generation");
        let (staged, publication) = generation.into_parts();
        let staged_output = staged.paths.output_dir.clone();
        let staged_vector = staged.paths.vector_db_uri.clone();
        write_marker(&staged_output, "new-output").await;
        write_marker(&staged_vector, "new-vector").await;
        tokio::fs::write(&active.paths.vector_db_uri, "external vector file")
            .await
            .expect("replacement vector file");

        let error = publication
            .publish()
            .await
            .expect_err("replacement vector file must be rejected");

        assert!(error.to_string().contains("publication target"));
        assert!(error.to_string().contains("not a directory"));
        assert_eq!(read_marker(&active.paths.output_dir).await, "old-output");
        assert_eq!(
            tokio::fs::read_to_string(&active.paths.vector_db_uri)
                .await
                .expect("replacement vector contents"),
            "external vector file"
        );
        assert!(!staged_output.exists());
        assert!(!staged_vector.exists());
        assert_no_transaction_residue(tempdir.path()).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_should_preserve_empty_live_directory_and_rollback_previous_target() {
        let tempdir = TempDir::new().expect("tempdir");
        let output = tempdir.path().join("output");
        let staged_output = tempdir.path().join("output-staged");
        let vector = tempdir.path().join("vector");
        let staged_vector = tempdir.path().join("vector-staged");
        write_marker(&output, "old-output").await;
        write_marker(&staged_output, "new-output").await;
        write_marker(&staged_vector, "new-vector").await;
        let publication = IndexPublication {
            targets: vec![
                PublicationTarget::replace(output.clone(), staged_output.clone()),
                PublicationTarget::replace(vector.clone(), staged_vector.clone()),
            ],
        };

        let error = publication
            .publish_with_hook(|index| {
                if index == 1 {
                    create_dir_in_hook(&vector, "create empty vector conflict")?;
                }
                Ok(())
            })
            .await
            .expect_err("empty vector directory must conflict with publication");

        assert!(error.to_string().contains("unexpectedly exists"));
        assert!(error.to_string().contains("before staged publish"));
        assert!(matches!(
            &error,
            GraphLoomError::Io {
                operation: "publish staged index generation",
                ..
            }
        ));
        assert_eq!(read_marker(&output).await, "old-output");
        assert!(vector.is_dir());
        let mut vector_entries = tokio::fs::read_dir(&vector)
            .await
            .expect("vector directory");
        assert!(
            vector_entries
                .next_entry()
                .await
                .expect("vector entry")
                .is_none(),
            "unexpected live vector directory must remain empty",
        );
        assert!(!staged_output.exists());
        assert!(!staged_vector.exists());
        assert_no_transaction_residue(tempdir.path()).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_should_preserve_empty_live_directory_for_single_target_conflict() {
        let tempdir = TempDir::new().expect("tempdir");
        let live = tempdir.path().join("output");
        let staged = tempdir.path().join("output-staged");
        write_marker(&staged, "new-output").await;
        let publication = IndexPublication {
            targets: vec![PublicationTarget::replace(live.clone(), staged.clone())],
        };

        let error = publication
            .publish_with_hook(|_| create_dir_in_hook(&live, "create empty output conflict"))
            .await
            .expect_err("empty output directory must conflict with publication");

        assert!(error.to_string().contains("unexpectedly exists"));
        assert!(matches!(
            &error,
            GraphLoomError::Io {
                operation: "publish staged index generation",
                ..
            }
        ));
        assert!(live.is_dir());
        let mut live_entries = tokio::fs::read_dir(&live).await.expect("live directory");
        assert!(
            live_entries
                .next_entry()
                .await
                .expect("live entry")
                .is_none(),
            "unexpected single live directory must remain empty",
        );
        assert!(!staged.exists());
        assert_no_transaction_residue(tempdir.path()).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_should_rollback_output_when_missing_vector_appears_as_file_before_publish() {
        let tempdir = TempDir::new().expect("tempdir");
        let output = tempdir.path().join("output");
        let staged_output = tempdir.path().join("output-staged");
        let vector = tempdir.path().join("vector");
        let staged_vector = tempdir.path().join("vector-staged");
        write_marker(&output, "old-output").await;
        write_marker(&staged_output, "new-output").await;
        write_marker(&staged_vector, "new-vector").await;
        let publication = IndexPublication {
            targets: vec![
                PublicationTarget::replace(output.clone(), staged_output.clone()),
                PublicationTarget::replace(vector.clone(), staged_vector.clone()),
            ],
        };

        let error = publication
            .publish_with_hook(|index| {
                if index == 1 {
                    write_in_hook(&vector, "unexpected vector file", "create vector conflict")?;
                }
                Ok(())
            })
            .await
            .expect_err("unexpected vector file must fail publication");

        assert!(error.to_string().contains("unexpectedly exists"));
        assert!(matches!(
            &error,
            GraphLoomError::Io {
                operation: "publish staged index generation",
                ..
            }
        ));
        assert_eq!(read_marker(&output).await, "old-output");
        assert_eq!(
            tokio::fs::read_to_string(&vector)
                .await
                .expect("vector conflict contents"),
            "unexpected vector file"
        );
        assert!(!staged_output.exists());
        assert!(!staged_vector.exists());
        assert_no_transaction_residue(tempdir.path()).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_should_rollback_output_when_missing_vector_appears_as_directory_before_publish() {
        let tempdir = TempDir::new().expect("tempdir");
        let output = tempdir.path().join("output");
        let staged_output = tempdir.path().join("output-staged");
        let vector = tempdir.path().join("vector");
        let staged_vector = tempdir.path().join("vector-staged");
        write_marker(&output, "old-output").await;
        write_marker(&staged_output, "new-output").await;
        write_marker(&staged_vector, "new-vector").await;
        let publication = IndexPublication {
            targets: vec![
                PublicationTarget::replace(output.clone(), staged_output.clone()),
                PublicationTarget::replace(vector.clone(), staged_vector.clone()),
            ],
        };

        let error = publication
            .publish_with_hook(|index| {
                if index == 1 {
                    write_marker_in_hook(
                        &vector,
                        "unexpected vector directory",
                        "create vector directory conflict",
                    )?;
                }
                Ok(())
            })
            .await
            .expect_err("unexpected vector directory must fail publication");

        assert!(error.to_string().contains("unexpectedly exists"));
        assert!(matches!(
            &error,
            GraphLoomError::Io {
                operation: "publish staged index generation",
                ..
            }
        ));
        assert_eq!(read_marker(&output).await, "old-output");
        assert_eq!(read_marker(&vector).await, "unexpected vector directory");
        assert!(!staged_output.exists());
        assert!(!staged_vector.exists());
        assert_no_transaction_residue(tempdir.path()).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_should_rollback_previous_and_preserve_current_backup_on_live_conflict() {
        let tempdir = TempDir::new().expect("tempdir");
        let output = tempdir.path().join("output");
        let staged_output = tempdir.path().join("output-staged");
        let vector = tempdir.path().join("vector");
        let staged_vector = tempdir.path().join("vector-staged");
        write_marker(&output, "old-output").await;
        write_marker(&staged_output, "new-output").await;
        write_marker(&vector, "old-vector").await;
        write_marker(&staged_vector, "new-vector").await;
        let publication = IndexPublication {
            targets: vec![
                PublicationTarget::replace(output.clone(), staged_output.clone()),
                PublicationTarget::replace(vector.clone(), staged_vector.clone()),
            ],
        };

        let error = publication
            .publish_with_hook(|index| {
                if index == 1 {
                    write_marker_in_hook(
                        &vector,
                        "unexpected vector directory",
                        "recreate vector live conflict",
                    )?;
                }
                Ok(())
            })
            .await
            .expect_err("recreated vector live must fail rollback");

        assert!(error.to_string().contains("unexpectedly exists"));
        assert!(matches!(&error, GraphLoomError::RollbackFailed { .. }));
        assert_eq!(read_marker(&output).await, "old-output");
        assert_eq!(read_marker(&vector).await, "unexpected vector directory");
        let vector_backup = find_single_backup(tempdir.path(), "vector").await;
        assert_eq!(read_marker(&vector_backup).await, "old-vector");
        assert!(!staged_output.exists());
        assert!(!staged_vector.exists());
        assert_no_transaction_residue_except(tempdir.path(), &vector_backup).await;
    }

    #[test]
    fn test_should_combine_current_and_previous_rollback_results() {
        let error = || GraphLoomError::Io {
            operation: "inject rollback failure",
            path: PathBuf::from("target"),
            source: std::io::Error::other("rollback failure"),
        };

        assert!(combine_rollback_results(Ok(()), Ok(())).is_ok());
        assert!(matches!(
            combine_rollback_results(Err(error()), Ok(())),
            Err(GraphLoomError::Io { .. })
        ));
        assert!(matches!(
            combine_rollback_results(Ok(()), Err(error())),
            Err(GraphLoomError::Io { .. })
        ));
        let combined = combine_rollback_results(Err(error()), Err(error()))
            .expect_err("both rollback failures must be retained");
        assert!(matches!(combined, GraphLoomError::RollbackFailed { .. }));
    }

    #[tokio::test]
    async fn test_should_treat_existing_live_as_successful_rollback_without_backup() {
        let tempdir = TempDir::new().expect("tempdir");
        let live = tempdir.path().join("live");
        tokio::fs::write(&live, "unexpected live")
            .await
            .expect("live file");
        let mut hook_calls = 0;

        rollback_current_backup_only(3, &live, None, &mut |index, actual_live, backup| {
            hook_calls += 1;
            assert_eq!(index, 3);
            assert_eq!(actual_live, live);
            assert!(backup.is_none());
            Ok(())
        })
        .await
        .expect("no backup means no previous state needs restoration");

        assert_eq!(hook_calls, 1);
        assert_eq!(
            tokio::fs::read_to_string(&live)
                .await
                .expect("live contents"),
            "unexpected live"
        );
    }

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
        let generation = StagedIndexGeneration::new(&active, false)
            .await
            .expect("generation");
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

    #[cfg(unix)]
    #[tokio::test]
    async fn test_should_reject_preserved_descendant_with_symlink_ancestor() {
        use std::os::unix::fs::symlink;

        let tempdir = TempDir::new().expect("tempdir");
        let live = tempdir.path().join("output");
        let staged = tempdir.path().join("output-staged");
        let external = tempdir.path().join("external");
        write_marker(&live, "old-output").await;
        write_marker(&external.join("lancedb"), "external-vector").await;
        symlink("../external", live.join("vectors")).expect("symlink");
        write_marker(&staged, "new-output").await;
        let publication = IndexPublication {
            targets: vec![
                PublicationTarget::replace_preserving(
                    live.clone(),
                    staged,
                    vec![PathBuf::from("vectors/lancedb")],
                )
                .expect("preserved target"),
            ],
        };

        let error = publication
            .publish()
            .await
            .expect_err("linked ancestor must fail");

        assert!(error.to_string().contains("symlink or reparse point"));
        assert_eq!(read_marker(&live).await, "old-output");
        assert!(
            tokio::fs::symlink_metadata(live.join("vectors"))
                .await
                .expect("symlink metadata")
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            read_marker(&external.join("lancedb")).await,
            "external-vector"
        );
        assert_only_active_directories(tempdir.path(), &["external", "output"]).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_should_reject_preserved_descendant_that_is_symlink() {
        use std::os::unix::fs::symlink;

        let tempdir = TempDir::new().expect("tempdir");
        let live = tempdir.path().join("output");
        let staged = tempdir.path().join("output-staged");
        let external = tempdir.path().join("external-lancedb");
        write_marker(&live, "old-output").await;
        write_marker(&external, "external-vector").await;
        symlink("../external-lancedb", live.join("lancedb")).expect("symlink");
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
            .publish()
            .await
            .expect_err("linked descendant must fail");

        assert!(error.to_string().contains("symlink or reparse point"));
        assert_eq!(read_marker(&live).await, "old-output");
        assert!(
            tokio::fs::symlink_metadata(live.join("lancedb"))
                .await
                .expect("symlink metadata")
                .file_type()
                .is_symlink()
        );
        assert_eq!(read_marker(&external).await, "external-vector");
        assert_only_active_directories(tempdir.path(), &["external-lancedb", "output"]).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_should_stop_rollback_when_preserved_restore_path_becomes_symlink() {
        use std::os::unix::fs::symlink;

        let tempdir = TempDir::new().expect("tempdir");
        let live = tempdir.path().join("output");
        let backup = tempdir.path().join("output-backup");
        let external = tempdir.path().join("external-destination");
        write_marker(&live, "new-output").await;
        write_marker(&live.join("lancedb"), "old-vector").await;
        write_marker(&backup, "old-output").await;
        tokio::fs::create_dir(&external).await.expect("external");
        let target = PublishedTarget {
            live: live.clone(),
            backup: Some(backup.clone()),
            moved_descendants: vec![PathBuf::from("lancedb")],
        };

        let error = rollback_target(0, &target, &mut |_, _, destination| {
            symlink(&external, destination).map_err(|source| GraphLoomError::Io {
                operation: "inject restore destination symlink",
                path: destination.to_path_buf(),
                source,
            })?;
            Ok(())
        })
        .await
        .expect_err("reverse link validation must fail");

        assert!(error.to_string().contains("symlink or reparse point"));
        assert_eq!(read_marker(&live).await, "new-output");
        assert_eq!(read_marker(&live.join("lancedb")).await, "old-vector");
        assert_eq!(read_marker(&backup).await, "old-output");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_should_stop_restore_when_backup_root_becomes_symlink() {
        use std::os::unix::fs::symlink;

        let tempdir = TempDir::new().expect("tempdir");
        let live = tempdir.path().join("output");
        let backup = tempdir.path().join("output-backup");
        let backup_original = tempdir.path().join("output-backup-original");
        let external = tempdir.path().join("external");
        write_marker(&live, "new-output").await;
        write_marker(&live.join("lancedb"), "old-vector").await;
        write_marker(&backup_original, "old-output").await;
        tokio::fs::create_dir(&external).await.expect("external");
        symlink(&external, &backup).expect("backup root symlink");
        let target = PublishedTarget {
            live: live.clone(),
            backup: Some(backup.clone()),
            moved_descendants: vec![PathBuf::from("lancedb")],
        };

        let error = rollback_target(0, &target, &mut |_, _, _| Ok(()))
            .await
            .expect_err("linked backup root must stop rollback");

        assert!(matches!(
            error,
            GraphLoomError::UnsafePublicationRoot { ref path, .. } if path == &backup
        ));
        assert_eq!(read_marker(&live).await, "new-output");
        assert_eq!(read_marker(&live.join("lancedb")).await, "old-vector");
        assert_eq!(read_marker(&backup_original).await, "old-output");
        assert!(
            tokio::fs::symlink_metadata(&backup)
                .await
                .expect("backup symlink")
                .file_type()
                .is_symlink()
        );
        assert!(
            !tokio::fs::try_exists(external.join("lancedb"))
                .await
                .expect("external descendant existence")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_should_not_rollback_previous_target_after_current_root_safety_failure() {
        use std::os::unix::fs::symlink;

        let tempdir = TempDir::new().expect("tempdir");
        let first_live = tempdir.path().join("first-live");
        let first_backup = tempdir.path().join("first-backup");
        let current_live = tempdir.path().join("current-live");
        let current_backup = tempdir.path().join("current-backup");
        let external = tempdir.path().join("external");
        write_marker(&first_live, "first-new").await;
        write_marker(&first_backup, "first-old").await;
        write_marker(&current_live, "current-new").await;
        write_marker(&current_live.join("lancedb"), "current-vector").await;
        tokio::fs::create_dir(&external).await.expect("external");
        symlink(&external, &current_backup).expect("current backup symlink");
        let targets = vec![
            PublishedTarget {
                live: first_live.clone(),
                backup: Some(first_backup.clone()),
                moved_descendants: Vec::new(),
            },
            PublishedTarget {
                live: current_live.clone(),
                backup: Some(current_backup),
                moved_descendants: vec![PathBuf::from("lancedb")],
            },
        ];
        let mut restore_indices = Vec::new();

        let error = rollback_publication(&targets, &mut |index, _, _| {
            restore_indices.push(index);
            Ok(())
        })
        .await
        .expect_err("current linked root must stop rollback");

        assert!(matches!(
            error,
            GraphLoomError::UnsafePublicationRoot { .. }
        ));
        assert!(restore_indices.is_empty());
        assert_eq!(read_marker(&current_live).await, "current-new");
        assert_eq!(
            read_marker(&current_live.join("lancedb")).await,
            "current-vector"
        );
        assert_eq!(read_marker(&first_live).await, "first-new");
        assert_eq!(read_marker(&first_backup).await, "first-old");
    }

    #[tokio::test]
    async fn test_should_stop_previous_target_rollback_when_current_rollback_fails() {
        let tempdir = TempDir::new().expect("tempdir");
        let first_live = tempdir.path().join("first-live");
        let first_backup = tempdir.path().join("first-backup");
        let second_live = tempdir.path().join("second-live");
        let second_backup = tempdir.path().join("second-backup");
        write_marker(&first_live, "first-new").await;
        write_marker(&first_backup, "first-old").await;
        write_marker(&second_live, "second-new").await;
        write_marker(&second_live.join("lancedb"), "second-vector").await;
        write_marker(&second_backup, "second-old").await;
        let targets = vec![
            PublishedTarget {
                live: first_live.clone(),
                backup: Some(first_backup.clone()),
                moved_descendants: Vec::new(),
            },
            PublishedTarget {
                live: second_live.clone(),
                backup: Some(second_backup.clone()),
                moved_descendants: vec![PathBuf::from("lancedb")],
            },
        ];
        let mut restore_indices = Vec::new();

        let error = rollback_publication(&targets, &mut |index, _, destination| {
            restore_indices.push(index);
            Err(GraphLoomError::Io {
                operation: "inject current rollback failure",
                path: destination.to_path_buf(),
                source: std::io::Error::other("injected current rollback failure"),
            })
        })
        .await
        .expect_err("current rollback must fail");

        assert!(
            error
                .to_string()
                .contains("injected current rollback failure")
        );
        assert_eq!(restore_indices, vec![1]);
        assert_eq!(read_marker(&second_live).await, "second-new");
        assert_eq!(
            read_marker(&second_live.join("lancedb")).await,
            "second-vector"
        );
        assert_eq!(read_marker(&second_backup).await, "second-old");
        assert_eq!(read_marker(&first_live).await, "first-new");
        assert_eq!(read_marker(&first_backup).await, "first-old");
    }

    #[tokio::test]
    async fn test_should_rollback_previous_target_when_current_backup_restore_fails() {
        let tempdir = TempDir::new().expect("tempdir");
        let first_live = tempdir.path().join("first");
        let first_staged = tempdir.path().join("first-staged");
        let second_live = tempdir.path().join("second");
        let second_staged = tempdir.path().join("second-staged");
        write_marker(&first_live, "first-old").await;
        write_marker(&first_staged, "first-new").await;
        write_marker(&second_live, "second-old").await;
        write_marker(&second_staged, "second-new").await;
        let publication = IndexPublication {
            targets: vec![
                PublicationTarget::replace(first_live.clone(), first_staged.clone()),
                PublicationTarget::replace(second_live.clone(), second_staged),
            ],
        };

        let error = publication
            .publish_with_hooks(
                |index| {
                    if index == 1 {
                        return Err(GraphLoomError::Io {
                            operation: "inject before publish failure",
                            path: second_live.clone(),
                            source: std::io::Error::other("injected before publish failure"),
                        });
                    }
                    Ok(())
                },
                |_, _, _| Ok(()),
                |_, _, _| Ok(()),
                |index, live, _| {
                    if index == 1 {
                        return Err(GraphLoomError::Io {
                            operation: "inject current backup restore failure",
                            path: live.to_path_buf(),
                            source: std::io::Error::other(
                                "injected current backup restore failure",
                            ),
                        });
                    }
                    Ok(())
                },
            )
            .await
            .expect_err("current restore must fail");

        assert!(matches!(error, GraphLoomError::RollbackFailed { .. }));
        assert_eq!(read_marker(&first_live).await, "first-old");
        assert!(!first_staged.exists());
        assert!(!second_live.exists());
        assert!(find_single_backup(tempdir.path(), "second").await.is_dir());
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
                |_, _, _| Ok(()),
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

    #[cfg(unix)]
    #[tokio::test]
    async fn test_should_validate_rollback_roots_without_moved_descendants() {
        use std::os::unix::fs::symlink;

        let tempdir = TempDir::new().expect("tempdir");
        let live = tempdir.path().join("output");
        let backup = tempdir.path().join("output-backup");
        let external = tempdir.path().join("external");
        write_marker(&live, "new-output").await;
        tokio::fs::create_dir(&external).await.expect("external");
        symlink(&external, &backup).expect("backup symlink");
        let target = PublishedTarget {
            live: live.clone(),
            backup: Some(backup.clone()),
            moved_descendants: Vec::new(),
        };
        let mut hook_called = false;

        let error = rollback_target(0, &target, &mut |_, _, _| {
            hook_called = true;
            Ok(())
        })
        .await
        .expect_err("empty descendant rollback must validate roots");

        assert!(matches!(
            error,
            GraphLoomError::UnsafePublicationRoot { ref path, .. } if path == &backup
        ));
        assert!(!hook_called);
        assert_eq!(read_marker(&live).await, "new-output");
        assert!(
            tokio::fs::symlink_metadata(&backup)
                .await
                .expect("backup metadata")
                .file_type()
                .is_symlink()
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn test_should_keep_live_when_backup_root_changes_before_preserved_move() {
        use std::os::unix::fs::symlink;

        let tempdir = TempDir::new().expect("tempdir");
        let live = tempdir.path().join("output");
        let staged = tempdir.path().join("output-staged");
        let external = tempdir.path().join("external");
        write_marker(&live, "old-output").await;
        write_marker(&live.join("lancedb"), "old-vector").await;
        write_marker(&staged, "new-output").await;
        tokio::fs::create_dir(&external).await.expect("external");
        let publication = preserving_publication(live.clone(), staged);
        let mut backup_original = None;

        let error = publication
            .publish_with_hooks(
                |_| Ok(()),
                |_, source, _| {
                    let backup = source.parent().ok_or_else(|| GraphLoomError::Io {
                        operation: "locate injected backup root",
                        path: source.to_path_buf(),
                        source: std::io::Error::other("preserved source has no parent"),
                    })?;
                    let original = backup.with_extension("backup-original");
                    rename_in_hook(backup, &original, "replace backup root")?;
                    symlink(&external, backup)
                        .map_err(|source| io_error("link backup root", backup, source))?;
                    backup_original = Some(original);
                    Ok(())
                },
                |_, _, _| Ok(()),
                |_, _, _| Ok(()),
            )
            .await
            .expect_err("linked backup root must fail publication and rollback");

        let original = backup_original.expect("hook backup original");
        assert_unsafe_root_rollback(&error);
        assert_eq!(read_marker(&live).await, "new-output");
        assert_eq!(read_marker(&original).await, "old-output");
        assert_eq!(read_marker(&original.join("lancedb")).await, "old-vector");
        let backup = original.with_extension("backup");
        assert!(
            tokio::fs::symlink_metadata(&backup)
                .await
                .expect("backup symlink")
                .file_type()
                .is_symlink()
        );
        assert!(!external.join("lancedb").exists());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn test_should_keep_live_when_backup_root_disappears_before_rollback() {
        let tempdir = TempDir::new().expect("tempdir");
        let live = tempdir.path().join("output");
        let staged = tempdir.path().join("output-staged");
        write_marker(&live, "old-output").await;
        write_marker(&live.join("lancedb"), "old-vector").await;
        write_marker(&staged, "new-output").await;
        let publication = preserving_publication(live.clone(), staged);
        let mut backup_original = None;

        let error = publication
            .publish_with_hooks(
                |_| Ok(()),
                |_, source, _| {
                    let backup = source.parent().ok_or_else(|| GraphLoomError::Io {
                        operation: "locate injected backup root",
                        path: source.to_path_buf(),
                        source: std::io::Error::other("preserved source has no parent"),
                    })?;
                    let original = backup.with_extension("backup-original");
                    rename_in_hook(backup, &original, "remove backup root")?;
                    backup_original = Some(original);
                    Ok(())
                },
                |_, _, _| Ok(()),
                |_, _, _| Ok(()),
            )
            .await
            .expect_err("missing backup root must fail publication and rollback");

        let original = backup_original.expect("hook backup original");
        assert!(matches!(error, GraphLoomError::RollbackFailed { .. }));
        assert_eq!(read_marker(&live).await, "new-output");
        assert_eq!(read_marker(&original).await, "old-output");
        assert_eq!(read_marker(&original.join("lancedb")).await, "old-vector");
        assert!(!original.with_extension("backup").exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_should_keep_live_when_backup_root_becomes_file_before_rollback() {
        let tempdir = TempDir::new().expect("tempdir");
        let live = tempdir.path().join("output");
        let staged = tempdir.path().join("output-staged");
        write_marker(&live, "old-output").await;
        write_marker(&live.join("lancedb"), "old-vector").await;
        write_marker(&staged, "new-output").await;
        let publication = preserving_publication(live.clone(), staged);
        let mut backup_original = None;

        let error = publication
            .publish_with_hooks(
                |_| Ok(()),
                |_, source, _| {
                    let backup = source.parent().ok_or_else(|| GraphLoomError::Io {
                        operation: "locate injected backup root",
                        path: source.to_path_buf(),
                        source: std::io::Error::other("preserved source has no parent"),
                    })?;
                    let original = backup.with_extension("backup-original");
                    rename_in_hook(backup, &original, "replace backup root")?;
                    write_in_hook(backup, "unsafe replacement", "write backup replacement")?;
                    backup_original = Some(original);
                    Ok(())
                },
                |_, _, _| Ok(()),
                |_, _, _| Ok(()),
            )
            .await
            .expect_err("file backup root must fail publication and rollback");

        let original = backup_original.expect("hook backup original");
        assert!(matches!(error, GraphLoomError::RollbackFailed { .. }));
        assert!(error.to_string().contains("not a directory"));
        assert_eq!(read_marker(&live).await, "new-output");
        assert_eq!(read_marker(&original).await, "old-output");
        assert_eq!(read_marker(&original.join("lancedb")).await, "old-vector");
        assert!(original.with_extension("backup").is_file());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn test_should_not_rollback_previous_target_after_forward_root_failure() {
        use std::os::unix::fs::symlink;

        let tempdir = TempDir::new().expect("tempdir");
        let first_live = tempdir.path().join("first");
        let first_staged = tempdir.path().join("first-staged");
        let current_live = tempdir.path().join("output");
        let current_staged = tempdir.path().join("output-staged");
        let external = tempdir.path().join("external");
        write_marker(&first_live, "first-old").await;
        write_marker(&first_staged, "first-new").await;
        write_marker(&current_live, "current-old").await;
        write_marker(&current_live.join("lancedb"), "current-vector").await;
        write_marker(&current_staged, "current-new").await;
        tokio::fs::create_dir(&external).await.expect("external");
        let publication = IndexPublication {
            targets: vec![
                PublicationTarget::replace(first_live.clone(), first_staged),
                PublicationTarget::replace_preserving(
                    current_live.clone(),
                    current_staged,
                    vec![PathBuf::from("lancedb")],
                )
                .expect("preserved target"),
            ],
        };
        let mut restore_indices = Vec::new();

        let error = publication
            .publish_with_hooks(
                |_| Ok(()),
                |index, source, _| {
                    if index == 1 {
                        let backup = source.parent().ok_or_else(|| GraphLoomError::Io {
                            operation: "locate injected backup root",
                            path: source.to_path_buf(),
                            source: std::io::Error::other("preserved source has no parent"),
                        })?;
                        let original = backup.with_extension("backup-original");
                        rename_in_hook(backup, &original, "replace backup root")?;
                        symlink(&external, backup)
                            .map_err(|source| io_error("link backup root", backup, source))?;
                    }
                    Ok(())
                },
                |index, _, _| {
                    restore_indices.push(index);
                    Ok(())
                },
                |_, _, _| Ok(()),
            )
            .await
            .expect_err("current root replacement must stop previous rollback");

        assert_unsafe_root_rollback(&error);
        assert!(restore_indices.is_empty());
        assert_eq!(read_marker(&first_live).await, "first-new");
        assert_eq!(
            read_marker(&find_single_backup(tempdir.path(), "first").await).await,
            "first-old"
        );
        assert_eq!(read_marker(&current_live).await, "current-new");
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
        tokio::fs::create_dir_all(directory)
            .await
            .expect("directory");
        tokio::fs::write(directory.join("marker"), value)
            .await
            .expect("marker");
    }

    fn preserving_publication(live: PathBuf, staged: PathBuf) -> IndexPublication {
        IndexPublication {
            targets: vec![
                PublicationTarget::replace_preserving(live, staged, vec![PathBuf::from("lancedb")])
                    .expect("preserved target"),
            ],
        }
    }

    fn rename_in_hook(source: &Path, destination: &Path, operation: &'static str) -> Result<()> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                tokio::fs::rename(source, destination)
                    .await
                    .map_err(|error| io_error(operation, source, error))
            })
        })
    }

    fn write_in_hook(path: &Path, contents: &str, operation: &'static str) -> Result<()> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                tokio::fs::write(path, contents)
                    .await
                    .map_err(|error| io_error(operation, path, error))
            })
        })
    }

    fn write_marker_in_hook(
        directory: &Path,
        contents: &str,
        operation: &'static str,
    ) -> Result<()> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                tokio::fs::create_dir_all(directory)
                    .await
                    .map_err(|error| io_error(operation, directory, error))?;
                let marker = directory.join("marker");
                tokio::fs::write(&marker, contents)
                    .await
                    .map_err(|error| io_error(operation, &marker, error))
            })
        })
    }

    fn create_dir_in_hook(directory: &Path, operation: &'static str) -> Result<()> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                tokio::fs::create_dir(directory)
                    .await
                    .map_err(|error| io_error(operation, directory, error))
            })
        })
    }

    #[cfg(unix)]
    fn assert_unsafe_root_rollback(error: &GraphLoomError) {
        let GraphLoomError::RollbackFailed {
            source, rollback, ..
        } = error
        else {
            panic!("expected rollback failure, got {error}");
        };
        assert!(matches!(
            source.as_ref(),
            GraphLoomError::UnsafePublicationRoot {
                operation: "move preserved descendant source",
                ..
            }
        ));
        assert!(matches!(
            rollback.as_ref(),
            GraphLoomError::UnsafePublicationRoot {
                operation: "validate rollback backup root",
                ..
            }
        ));
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

    async fn assert_no_transaction_residue(root: &Path) {
        let mut entries = tokio::fs::read_dir(root).await.expect("root entries");
        while let Some(entry) = entries.next_entry().await.expect("entry") {
            let name = entry.file_name().to_string_lossy().into_owned();
            assert!(
                !name.ends_with(".staging") && !name.ends_with(".backup"),
                "publication transaction residue should not remain: {name}",
            );
        }
    }

    async fn assert_no_transaction_residue_except(root: &Path, retained: &Path) {
        let mut entries = tokio::fs::read_dir(root).await.expect("root entries");
        while let Some(entry) = entries.next_entry().await.expect("entry") {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".staging") || name.ends_with(".backup") {
                assert_eq!(path, retained, "unexpected transaction residue: {name}");
            }
        }
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
