use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use regex::Regex;

use super::Storage;
use crate::{
    Result, StorageError,
    path::{path_to_logical, strip_namespace, validate_logical_path},
};

/// In-memory [`Storage`] for tests and deterministic local execution.
#[derive(Debug, Clone, Default)]
pub struct MemoryStorage {
    storage: Arc<DashMap<String, Vec<u8>>>,
    namespace: String,
}

impl MemoryStorage {
    /// Create an empty memory storage.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn key(&self, name: &str) -> Result<String> {
        let name = path_to_logical(&validate_logical_path(name)?);
        if self.namespace.is_empty() {
            Ok(name)
        } else if name.is_empty() {
            Ok(self.namespace.clone())
        } else {
            Ok(format!("{}/{}", self.namespace, name))
        }
    }
}

#[async_trait]
impl Storage for MemoryStorage {
    async fn get(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let key = self.key(name)?;
        Ok(self.storage.get(&key).map(|bytes| bytes.value().clone()))
    }

    async fn set(&self, name: &str, bytes: &[u8]) -> Result<()> {
        self.storage.insert(self.key(name)?, bytes.to_vec());
        Ok(())
    }

    async fn delete(&self, name: &str) -> Result<()> {
        self.storage.remove(&self.key(name)?);
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        if self.namespace.is_empty() {
            self.storage.clear();
        } else {
            let prefix = format!("{}/", self.namespace);
            let keys = self
                .storage
                .iter()
                .filter_map(|entry| {
                    let key = entry.key();
                    if key == &self.namespace || key.starts_with(&prefix) {
                        Some(key.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            for key in keys {
                self.storage.remove(&key);
            }
        }
        Ok(())
    }

    async fn has(&self, name: &str) -> Result<bool> {
        Ok(self.storage.contains_key(&self.key(name)?))
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let prefix = self.key(prefix)?;
        let mut names = self
            .storage
            .iter()
            .filter_map(|entry| {
                let key = entry.key();
                if key.starts_with(&prefix) {
                    Some(strip_namespace(key, &self.namespace))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        names.sort();
        Ok(names)
    }

    async fn keys(&self) -> Result<Vec<String>> {
        let mut names = self
            .storage
            .iter()
            .filter_map(|entry| {
                let stripped = strip_namespace(entry.key(), &self.namespace);
                if stripped.contains('/') {
                    None
                } else {
                    Some(stripped)
                }
            })
            .collect::<Vec<_>>();
        names.sort();
        Ok(names)
    }

    async fn find(&self, pattern: &str) -> Result<Vec<String>> {
        let regex = Regex::new(pattern).map_err(|source| StorageError::Regex {
            pattern: pattern.to_owned(),
            source,
        })?;
        let mut names = self
            .storage
            .iter()
            .filter_map(|entry| {
                let key = entry.key();
                if self.namespace.is_empty() || key.starts_with(&format!("{}/", self.namespace)) {
                    let stripped = strip_namespace(key, &self.namespace);
                    if regex.is_match(&stripped) {
                        return Some(stripped);
                    }
                }
                None
            })
            .collect::<Vec<_>>();
        names.sort();
        Ok(names)
    }

    async fn get_creation_date(&self, _name: &str) -> Result<Option<String>> {
        Ok(None)
    }

    fn child(&self, namespace: Option<&str>) -> Result<Arc<dyn Storage>> {
        let Some(namespace) = namespace else {
            return Ok(Arc::new(self.clone()));
        };
        validate_logical_path(namespace)?;
        Ok(Arc::new(Self::new()))
    }
}
