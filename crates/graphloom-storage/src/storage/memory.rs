use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;

use super::Storage;
use crate::{
    Result, StorageError,
    path::{path_to_logical, strip_namespace, validate_logical_path},
};

/// In-memory [`Storage`] for tests and deterministic local execution.
#[derive(Debug, Clone, Default)]
pub struct MemoryStorage {
    objects: Arc<DashMap<String, Vec<u8>>>,
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
    async fn get(&self, name: &str) -> Result<Vec<u8>> {
        let key = self.key(name)?;
        self.objects
            .get(&key)
            .map(|bytes| bytes.value().clone())
            .ok_or(StorageError::InvalidPath {
                path: name.to_owned(),
                reason: "object does not exist",
            })
    }

    async fn set(&self, name: &str, bytes: &[u8]) -> Result<()> {
        self.objects.insert(self.key(name)?, bytes.to_vec());
        Ok(())
    }

    async fn delete(&self, name: &str) -> Result<()> {
        self.objects.remove(&self.key(name)?);
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        if self.namespace.is_empty() {
            self.objects.clear();
        } else {
            let prefix = format!("{}/", self.namespace);
            let keys = self
                .objects
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
                self.objects.remove(&key);
            }
        }
        Ok(())
    }

    async fn has(&self, name: &str) -> Result<bool> {
        Ok(self.objects.contains_key(&self.key(name)?))
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let prefix = self.key(prefix)?;
        let mut names = self
            .objects
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

    async fn get_creation_date(&self, _name: &str) -> Result<Option<String>> {
        Ok(None)
    }

    fn child(&self, namespace: &str) -> Result<Arc<dyn Storage>> {
        let child = path_to_logical(&validate_logical_path(namespace)?);
        let namespace = if self.namespace.is_empty() {
            child
        } else {
            format!("{}/{}", self.namespace, child)
        };

        Ok(Arc::new(Self {
            objects: Arc::clone(&self.objects),
            namespace,
        }))
    }
}
