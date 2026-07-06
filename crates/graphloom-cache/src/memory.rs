use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::Value;

use crate::{Cache, Result};

/// In-memory cache provider.
///
/// This follows Microsoft `GraphRAG`'s memory cache behavior: values are stored
/// directly by key, and child caches are independent empty caches.
#[derive(Debug, Clone, Default)]
pub struct MemoryCache {
    values: Arc<DashMap<String, Value>>,
}

impl MemoryCache {
    /// Create an empty memory cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Cache for MemoryCache {
    async fn get(&self, key: &str) -> Result<Option<Value>> {
        Ok(self.values.get(key).map(|value| value.value().clone()))
    }

    async fn set_with_debug(
        &self,
        key: &str,
        value: Value,
        _debug_data: BTreeMap<String, Value>,
    ) -> Result<()> {
        self.values.insert(key.to_owned(), value);
        Ok(())
    }

    async fn has(&self, key: &str) -> Result<bool> {
        Ok(self.values.contains_key(key))
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.values.remove(key);
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        self.values.clear();
        Ok(())
    }

    fn child(&self, _namespace: &str) -> Result<Arc<dyn Cache>> {
        Ok(Arc::new(Self::new()))
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use graphloom_storage::{MemoryStorage, Storage};
    use serde_json::json;

    use crate::{Cache, JsonCache, MemoryCache};

    #[tokio::test]
    async fn test_should_store_and_delete_memory_cache_value() {
        let cache = MemoryCache::new();
        cache
            .set("a/b", json!({"value": 1}))
            .await
            .expect("set should work");

        assert_eq!(
            cache.get("a/b").await.expect("get should work"),
            Some(json!({"value": 1}))
        );

        cache.delete("a/b").await.expect("delete should work");
        assert!(!cache.has("a/b").await.expect("has should work"));
    }

    #[tokio::test]
    async fn test_should_store_json_cache_result_via_storage() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let cache = JsonCache::new(Arc::clone(&storage));
        let mut debug = BTreeMap::new();
        debug.insert("tokens".to_owned(), json!(12));

        cache
            .set_with_debug("llm/key", json!({"answer": "yes"}), debug)
            .await
            .expect("set should work");

        assert_eq!(
            cache.get("llm/key").await.expect("get should work"),
            Some(json!({"answer": "yes"}))
        );
        let raw = storage
            .get("llm/key")
            .await
            .expect("raw storage get should work");
        let raw: serde_json::Value =
            serde_json::from_slice(&raw).expect("stored value should be JSON");
        assert_eq!(raw["result"], json!({"answer": "yes"}));
        assert_eq!(raw["tokens"], json!(12));
    }

    #[tokio::test]
    async fn test_should_namespace_json_cache_through_storage_child() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let cache = JsonCache::new(Arc::clone(&storage));
        let child = cache.child("extract_graph").expect("child should be valid");

        child
            .set("key", json!("value"))
            .await
            .expect("set should work");

        assert!(child.has("key").await.expect("child has should work"));
        assert!(!cache.has("key").await.expect("root has should work"));
    }

    #[tokio::test]
    async fn test_should_delete_corrupt_json_cache_entry() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        storage
            .set("bad", b"{")
            .await
            .expect("storage set should work");
        let cache = JsonCache::new(Arc::clone(&storage));

        assert_eq!(
            cache.get("bad").await.expect("get should degrade to miss"),
            None
        );
        assert!(!storage.has("bad").await.expect("storage has should work"));
    }
}
