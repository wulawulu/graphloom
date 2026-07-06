use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;

use super::Storage;
use crate::{
    Result, StorageError,
    path::{path_to_logical, validate_logical_path},
};

/// Filesystem-backed [`Storage`] with path traversal protection.
#[derive(Debug, Clone)]
pub struct FileStorage {
    root: PathBuf,
    namespace: PathBuf,
}

impl FileStorage {
    /// Create a storage rooted at `root`.
    ///
    /// # Errors
    ///
    /// This constructor currently only validates the root path shape and cannot
    /// fail for existing paths. Directories are created by write operations.
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        Ok(Self {
            root,
            namespace: PathBuf::new(),
        })
    }

    fn logical_path(&self, name: &str) -> Result<PathBuf> {
        Ok(self
            .root
            .join(&self.namespace)
            .join(validate_logical_path(name)?))
    }

    fn namespace_prefix(&self) -> String {
        path_to_logical(&self.namespace)
    }
}

#[async_trait]
impl Storage for FileStorage {
    async fn get(&self, name: &str) -> Result<Vec<u8>> {
        let path = self.logical_path(name)?;
        tokio::fs::read(&path)
            .await
            .map_err(|source| StorageError::Filesystem { path, source })
    }

    async fn set(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let path = self.logical_path(name)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|source| StorageError::Filesystem {
                    path: parent.to_path_buf(),
                    source,
                })?;
        }
        tokio::fs::write(&path, bytes)
            .await
            .map_err(|source| StorageError::Filesystem { path, source })
    }

    async fn delete(&self, name: &str) -> Result<()> {
        let path = self.logical_path(name)?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StorageError::Filesystem { path, source }),
        }
    }

    async fn has(&self, name: &str) -> Result<bool> {
        let path = self.logical_path(name)?;
        tokio::fs::metadata(&path)
            .await
            .map(|metadata| metadata.is_file())
            .or_else(|source| {
                if source.kind() == std::io::ErrorKind::NotFound {
                    Ok(false)
                } else {
                    Err(StorageError::Filesystem { path, source })
                }
            })
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let prefix = path_to_logical(&validate_logical_path(prefix)?);
        let root = self.root.join(&self.namespace);
        let namespace_prefix = self.namespace_prefix();
        let mut names = Vec::new();

        if tokio::fs::try_exists(&root)
            .await
            .map_err(|source| StorageError::Filesystem {
                path: root.clone(),
                source,
            })?
        {
            collect_files(&root, &namespace_prefix, &prefix, &mut names).await?;
        }

        names.sort();
        Ok(names)
    }

    fn child(&self, namespace: &str) -> Result<Arc<dyn Storage>> {
        let mut child_namespace = self.namespace.clone();
        child_namespace.push(validate_logical_path(namespace)?);
        Ok(Arc::new(Self {
            root: self.root.clone(),
            namespace: child_namespace,
        }))
    }
}

async fn collect_files(
    base: &Path,
    namespace_prefix: &str,
    prefix: &str,
    output: &mut Vec<String>,
) -> Result<()> {
    let mut stack = vec![base.to_path_buf()];
    while let Some(current) = stack.pop() {
        let mut entries =
            tokio::fs::read_dir(&current)
                .await
                .map_err(|source| StorageError::Filesystem {
                    path: current.clone(),
                    source,
                })?;

        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|source| StorageError::Filesystem {
                    path: current.clone(),
                    source,
                })?
        {
            let path = entry.path();
            let file_type = entry
                .file_type()
                .await
                .map_err(|source| StorageError::Filesystem {
                    path: path.clone(),
                    source,
                })?;

            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                let relative = path
                    .strip_prefix(base)
                    .map_err(|_| StorageError::InvalidPath {
                        path: path.display().to_string(),
                        reason: "listed file escaped storage root",
                    })?;
                let logical = path_to_logical(relative);
                if logical.starts_with(prefix) {
                    if namespace_prefix.is_empty() {
                        output.push(logical);
                    } else {
                        output.push(format!("{namespace_prefix}/{logical}"));
                    }
                }
            }
        }
    }

    Ok(())
}
