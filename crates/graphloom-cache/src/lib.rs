//! Cache providers for `GraphLoom`.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

mod error;
mod json;
mod memory;

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
pub use error::{CacheError, Result};
pub use json::JsonCache;
pub use memory::MemoryCache;
use serde_json::Value;

/// Cache storage contract.
#[async_trait]
pub trait Cache: Send + Sync + std::fmt::Debug {
    /// Read a cached value.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid or the provider cannot read.
    async fn get(&self, key: &str) -> Result<Option<Value>>;

    /// Store a cached value.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid, serialization fails, or the
    /// provider cannot write.
    async fn set(&self, key: &str, value: Value) -> Result<()> {
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
        value: Value,
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
