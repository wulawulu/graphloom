//! Local model-call metadata excluded from provider and cache JSON.

/// Cache disposition for one model call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CacheStatus {
    /// No cache middleware participated in the call.
    #[default]
    NotUsed,
    /// The response came from cache.
    Hit,
    /// The response came from the provider after a cache miss.
    Miss,
}

/// Local metadata associated with a canonical response.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelCallMetadata {
    /// Cache disposition for this call.
    pub cache_status: CacheStatus,
}
