use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use chrono::{DateTime, Local};
use regex::Regex;

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
    /// Creates the root directory when it is absent.
    #[allow(
        clippy::disallowed_methods,
        reason = "FileStorage::new is a synchronous constructor and must create the root before \
                  returning"
    )]
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root).map_err(|source| StorageError::Filesystem {
            path: root.clone(),
            source,
        })?;
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
}

#[async_trait]
impl Storage for FileStorage {
    async fn get(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let path = self.logical_path(name)?;
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(StorageError::Filesystem { path, source }),
        }
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

    async fn clear(&self) -> Result<()> {
        let root = self.root.join(&self.namespace);
        if !tokio::fs::try_exists(&root)
            .await
            .map_err(|source| StorageError::Filesystem {
                path: root.clone(),
                source,
            })?
        {
            tokio::fs::create_dir_all(&root)
                .await
                .map_err(|source| StorageError::Filesystem { path: root, source })?;
            return Ok(());
        }

        let mut entries =
            tokio::fs::read_dir(&root)
                .await
                .map_err(|source| StorageError::Filesystem {
                    path: root.clone(),
                    source,
                })?;
        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|source| StorageError::Filesystem {
                    path: root.clone(),
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
                tokio::fs::remove_dir_all(&path)
                    .await
                    .map_err(|source| StorageError::Filesystem { path, source })?;
            } else {
                tokio::fs::remove_file(&path)
                    .await
                    .map_err(|source| StorageError::Filesystem { path, source })?;
            }
        }
        Ok(())
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
        let mut names = Vec::new();

        if tokio::fs::try_exists(&root)
            .await
            .map_err(|source| StorageError::Filesystem {
                path: root.clone(),
                source,
            })?
        {
            collect_files(&root, &prefix, None, &mut names).await?;
        }

        names.sort();
        Ok(names)
    }

    async fn keys(&self) -> Result<Vec<String>> {
        let root = self.root.join(&self.namespace);
        let mut names = Vec::new();
        if !tokio::fs::try_exists(&root)
            .await
            .map_err(|source| StorageError::Filesystem {
                path: root.clone(),
                source,
            })?
        {
            return Ok(names);
        }

        let mut entries =
            tokio::fs::read_dir(&root)
                .await
                .map_err(|source| StorageError::Filesystem {
                    path: root.clone(),
                    source,
                })?;
        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|source| StorageError::Filesystem {
                    path: root.clone(),
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
            if file_type.is_file()
                && let Some(name) = path.file_name().and_then(|name| name.to_str())
            {
                names.push(name.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }

    async fn find(&self, pattern: &str) -> Result<Vec<String>> {
        let regex = Regex::new(pattern).map_err(|source| StorageError::Regex {
            pattern: pattern.to_owned(),
            source,
        })?;
        let root = self.root.join(&self.namespace);
        let mut names = Vec::new();

        if tokio::fs::try_exists(&root)
            .await
            .map_err(|source| StorageError::Filesystem {
                path: root.clone(),
                source,
            })?
        {
            collect_files(&root, "", Some(&regex), &mut names).await?;
        }

        names.sort();
        Ok(names)
    }

    async fn get_creation_date(&self, name: &str) -> Result<Option<String>> {
        let path = self.logical_path(name)?;
        let metadata =
            tokio::fs::metadata(&path)
                .await
                .map_err(|source| StorageError::Filesystem {
                    path: path.clone(),
                    source,
                })?;
        let timestamp = metadata
            .created()
            .or_else(|_| metadata.modified())
            .map_err(|source| StorageError::Filesystem { path, source })?;
        let datetime = DateTime::<Local>::from(timestamp);

        Ok(Some(datetime.format("%Y-%m-%d %H:%M:%S %z").to_string()))
    }

    fn child(&self, namespace: Option<&str>) -> Result<Arc<dyn Storage>> {
        let Some(namespace) = namespace else {
            return Ok(Arc::new(self.clone()));
        };
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
    prefix: &str,
    regex: Option<&Regex>,
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
                if logical.starts_with(prefix) && regex.is_none_or(|regex| regex.is_match(&logical))
                {
                    output.push(logical);
                }
            }
        }
    }

    Ok(())
}
