//! Cache middleware for canonical model calls.

mod cached;

pub use cached::{CacheMetrics, CachedCompletionModel, CachedEmbeddingModel, CachedModelResult};
