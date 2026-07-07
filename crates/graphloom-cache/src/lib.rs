//! Cache providers for `GraphLoom`.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

mod error;
mod json;
mod memory;

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use bytes::Bytes;
pub use error::{CacheError, Result};
pub use json::JsonCache;
pub use memory::MemoryCache;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

/// Cache storage contract.
#[async_trait]
pub trait Cache: Send + Sync + std::fmt::Debug {
    /// Read a cached value.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid or the provider cannot read.
    async fn get(&self, key: &str) -> Result<Option<Bytes>>;

    /// Store a cached value.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid, serialization fails, or the
    /// provider cannot write.
    async fn set(&self, key: &str, value: Bytes) -> Result<()> {
        self.set_with_debug(key, value, BTreeMap::new()).await
    }

    /// Store a cached value plus optional debug metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid, serialization fails, or the
    /// provider cannot write.
    async fn set_with_debug(
        &self,
        key: &str,
        value: Bytes,
        debug_data: BTreeMap<String, Value>,
    ) -> Result<()>;

    /// Return whether a key exists.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid or the provider cannot inspect.
    async fn has(&self, key: &str) -> Result<bool>;

    /// Delete one cached value.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid or deletion fails.
    async fn delete(&self, key: &str) -> Result<()>;

    /// Clear this namespace.
    ///
    /// # Errors
    ///
    /// Returns an error when provider cleanup fails.
    async fn clear(&self) -> Result<()>;

    /// Create a namespace view rooted at `namespace`.
    ///
    /// # Errors
    ///
    /// Returns an error when the namespace is invalid.
    fn child(&self, namespace: &str) -> Result<Arc<dyn Cache>>;
}

/// JSON serialization helpers for cache payloads.
#[async_trait]
pub trait JsonCacheExt: Cache {
    /// Read and deserialize a cached JSON value.
    ///
    /// # Errors
    ///
    /// Returns an error when the cache provider cannot read or JSON decoding
    /// fails.
    async fn get_json<T>(&self, key: &str) -> Result<Option<T>>
    where
        T: DeserializeOwned + Send,
    {
        let Some(bytes) = self.get(key).await? else {
            return Ok(None);
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|source| CacheError::Json {
                key: key.to_owned(),
                source,
            })
    }

    /// Serialize and store a JSON value.
    ///
    /// # Errors
    ///
    /// Returns an error when JSON encoding fails or the cache provider cannot
    /// write.
    async fn set_json<T>(&self, key: &str, value: &T) -> Result<()>
    where
        T: Serialize + Sync,
    {
        let bytes = serde_json::to_vec(value).map_err(|source| CacheError::Json {
            key: key.to_owned(),
            source,
        })?;
        self.set(key, Bytes::from(bytes)).await
    }

    /// Serialize and store a JSON value plus optional debug metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when JSON encoding fails or the cache provider cannot
    /// write.
    async fn set_json_with_debug<T>(
        &self,
        key: &str,
        value: &T,
        debug_data: BTreeMap<String, Value>,
    ) -> Result<()>
    where
        T: Serialize + Sync,
    {
        let bytes = serde_json::to_vec(value).map_err(|source| CacheError::Json {
            key: key.to_owned(),
            source,
        })?;
        self.set_with_debug(key, Bytes::from(bytes), debug_data)
            .await
    }
}

#[async_trait]
impl<T> JsonCacheExt for T where T: Cache + ?Sized {}
