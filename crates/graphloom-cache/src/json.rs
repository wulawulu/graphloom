use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use graphloom_storage::Storage;
use serde_json::{Map, Value};

use crate::{Cache, CacheError, Result};

/// JSON cache backed by a [`Storage`] provider.
///
/// This mirrors Microsoft `GraphRAG`'s `JsonCache`: cache entries are stored as a
/// JSON object containing a `result` field plus optional debug metadata, while
/// the underlying storage implementation owns all file/blob/memory details.
#[derive(Debug, Clone)]
pub struct JsonCache {
    storage: Arc<dyn Storage>,
}

impl JsonCache {
    /// Create a JSON cache over an existing storage provider.
    #[must_use]
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self { storage }
    }
}

#[async_trait]
impl Cache for JsonCache {
    async fn get(&self, key: &str) -> Result<Option<Value>> {
        if !self.has(key).await? {
            return Ok(None);
        }

        let bytes = self.storage.get(key).await?;
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_source) => {
                self.storage.delete(key).await?;
                return Ok(None);
            }
        };
        let data = match serde_json::from_str::<Value>(&text) {
            Ok(data) => data,
            Err(_source) => {
                self.storage.delete(key).await?;
                return Ok(None);
            }
        };

        Ok(data.get("result").cloned())
    }

    async fn set_with_debug(
        &self,
        key: &str,
        value: Value,
        debug_data: BTreeMap<String, Value>,
    ) -> Result<()> {
        if value.is_null() {
            return Ok(());
        }

        let mut data = Map::new();
        data.insert("result".to_owned(), value);
        data.extend(debug_data);
        let bytes =
            serde_json::to_vec(&Value::Object(data)).map_err(|source| CacheError::Json {
                key: key.to_owned(),
                source,
            })?;
        self.storage.set(key, &bytes).await?;
        Ok(())
    }

    async fn has(&self, key: &str) -> Result<bool> {
        self.storage.has(key).await.map_err(CacheError::Storage)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        if self.has(key).await? {
            self.storage.delete(key).await?;
        }
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        self.storage.clear().await.map_err(CacheError::Storage)
    }

    fn child(&self, namespace: &str) -> Result<Arc<dyn Cache>> {
        Ok(Arc::new(Self {
            storage: self.storage.child(namespace)?,
        }))
    }
}
