//! Object storage abstractions and implementations.

mod file;
mod memory;

use std::sync::Arc;

use async_trait::async_trait;
pub use file::FileStorage;
pub use memory::MemoryStorage;

use crate::Result;

/// Byte/object storage used for inputs, snapshots, cache files, and other
/// non-table artifacts.
#[async_trait]
pub trait Storage: Send + Sync + std::fmt::Debug {
    /// Read an object by logical name.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is invalid or the object cannot be read.
    async fn get(&self, name: &str) -> Result<Vec<u8>>;

    /// Write an object by logical name, replacing any existing object.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is invalid or the bytes cannot be written.
    async fn set(&self, name: &str, bytes: &[u8]) -> Result<()>;

    /// Delete an object by logical name.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is invalid or removal fails for a reason
    /// other than the object not existing.
    async fn delete(&self, name: &str) -> Result<()>;

    /// Clear this storage namespace.
    ///
    /// # Errors
    ///
    /// Returns an error when provider cleanup fails.
    async fn clear(&self) -> Result<()>;

    /// Return whether an object exists.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is invalid.
    async fn has(&self, name: &str) -> Result<bool>;

    /// List object names below `prefix`.
    ///
    /// # Errors
    ///
    /// Returns an error when the prefix is invalid or a backing store cannot be
    /// enumerated.
    async fn list(&self, prefix: &str) -> Result<Vec<String>>;

    /// Return the object's creation date using `GraphRAG`'s local-time string
    /// format when the provider exposes one.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is invalid or metadata cannot be read.
    async fn get_creation_date(&self, name: &str) -> Result<Option<String>>;

    /// Create a namespace view rooted at `namespace`.
    ///
    /// # Errors
    ///
    /// Returns an error when the namespace is invalid.
    fn child(&self, namespace: &str) -> Result<Arc<dyn Storage>>;
}
