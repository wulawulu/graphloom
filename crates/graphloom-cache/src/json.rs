use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use bytes::Bytes;
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
    async fn get(&self, key: &str) -> Result<Option<Bytes>> {
        if !self.has(key).await? {
            return Ok(None);
        }

        let Some(bytes) = self.storage.get(key).await? else {
            return Ok(None);
        };
        let data = match serde_json::from_slice::<Value>(&bytes) {
            Ok(data) => data,
            Err(_source) => {
                self.storage.delete(key).await?;
                return Ok(None);
            }
        };

        data.get("result")
            .map(serde_json::to_vec)
            .transpose()
            .map(|value| value.map(Bytes::from))
            .map_err(|source| CacheError::Json {
                key: key.to_owned(),
                source,
            })
    }

    async fn set_with_debug(
        &self,
        key: &str,
        value: Bytes,
        debug_data: BTreeMap<String, Value>,
    ) -> Result<()> {
        let value = serde_json::from_slice::<Value>(&value).map_err(|source| CacheError::Json {
            key: key.to_owned(),
            source,
        })?;
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
            storage: self.storage.child(Some(namespace))?,
        }))
    }
}
