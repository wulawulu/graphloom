//! Shared workflow helpers.

use std::sync::Arc;

use graphloom_llm::{CachedCompletionModel, CachedEmbeddingModel, CompletionModel, EmbeddingModel};

use crate::{GraphRagConfig, IndexPipelineContext, Result};

pub(crate) fn resolve_completion_model(
    context: &IndexPipelineContext,
    model_id: &str,
    model_instance_name: &str,
    workflow: &'static str,
) -> Result<Arc<dyn CompletionModel>> {
    let model = context
        .models()
        .completion_for_workflow(model_id, workflow)?;
    let Some(cache) = context.cache() else {
        return Ok(model);
    };
    let cache = cache.child(model_instance_name)?;
    Ok(Arc::new(CachedCompletionModel::new(model, cache)))
}

pub(crate) fn resolve_completion_encoding_model<'a>(
    config: &'a GraphRagConfig,
    model_id: &str,
) -> &'a str {
    crate::config::effective_completion_encoding(config, model_id)
}

pub(crate) fn resolve_embedding_model(
    context: &IndexPipelineContext,
    model_id: &str,
    model_instance_name: &str,
    workflow: &'static str,
) -> Result<Arc<dyn EmbeddingModel>> {
    let model = context
        .models()
        .embedding_for_workflow(model_id, workflow)?;
    let Some(cache) = context.cache() else {
        return Ok(model);
    };
    let cache = cache.child(model_instance_name)?;
    Ok(Arc::new(CachedEmbeddingModel::new(model, cache)))
}

pub(crate) fn resolve_embedding_encoding_model<'a>(
    config: &'a GraphRagConfig,
    model_id: &str,
) -> &'a str {
    crate::config::effective_embedding_encoding(config, model_id)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use bytes::Bytes;
    use graphloom_cache::{Cache, Result as CacheResult};
    use graphloom_llm::MockCompletionModel;
    use graphloom_storage::MemoryTableProvider;

    use super::resolve_completion_model;
    use crate::IndexPipelineContext;

    #[derive(Debug, Default)]
    struct RecordingCache {
        children: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Cache for RecordingCache {
        async fn get(&self, _key: &str) -> CacheResult<Option<Bytes>> {
            Ok(None)
        }

        async fn set_with_debug(
            &self,
            _key: &str,
            _value: Bytes,
            _debug_data: std::collections::BTreeMap<String, serde_json::Value>,
        ) -> CacheResult<()> {
            Ok(())
        }

        async fn has(&self, _key: &str) -> CacheResult<bool> {
            Ok(false)
        }

        async fn delete(&self, _key: &str) -> CacheResult<()> {
            Ok(())
        }

        async fn clear(&self) -> CacheResult<()> {
            Ok(())
        }

        fn child(&self, namespace: &str) -> CacheResult<Arc<dyn Cache>> {
            self.children
                .lock()
                .expect("children lock")
                .push(namespace.to_owned());
            Ok(Arc::new(Self {
                children: Arc::clone(&self.children),
            }))
        }
    }

    #[test]
    fn test_should_namespace_cached_model_by_model_instance_name() {
        let cache = Arc::new(RecordingCache::default());
        let children = Arc::clone(&cache.children);
        let context = IndexPipelineContext::for_test(Arc::new(MemoryTableProvider::new()))
            .with_cache(cache)
            .with_completion_model(
                "shared",
                Arc::new(MockCompletionModel::new(
                    "shared",
                    vec!["answer".to_owned()],
                )),
            )
            .expect("model registry");

        resolve_completion_model(&context, "shared", "extract_graph", "test")
            .expect("cached model");
        resolve_completion_model(&context, "shared", "summarize_descriptions", "test")
            .expect("cached model");

        assert_eq!(
            children.lock().expect("children lock").as_slice(),
            &["extract_graph", "summarize_descriptions"]
        );
    }
}
