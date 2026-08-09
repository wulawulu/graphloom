//! Composable Studio Query and Explainability Run HTTP APIs.

mod query;
mod runs;

use std::{fmt, num::NonZeroUsize, path::PathBuf, sync::Arc};

use axum::{Router, routing::get};
use graphloom::{
    GraphRagConfig,
    explainability::{ExplainabilityLiveHub, ExplainabilityStore},
};
use tokio::sync::Semaphore;

use self::{
    query::{GraphLoomQueryRunner, QueryRunner, start_query},
    runs::{get_run, list_runs},
};
use crate::explainability::ExplainabilitySseService;

const DEFAULT_MAX_CONCURRENT_QUERIES: usize = 4;

/// Bounded Query-job admission options for [`StudioApiService`].
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct StudioApiOptions {
    max_concurrent_queries: NonZeroUsize,
}

impl StudioApiOptions {
    /// Create options allowing four concurrent Query jobs.
    #[must_use]
    pub fn new() -> Self {
        Self {
            max_concurrent_queries: NonZeroUsize::new(DEFAULT_MAX_CONCURRENT_QUERIES)
                .unwrap_or(NonZeroUsize::MIN),
        }
    }

    /// Override the maximum number of active Query lifecycles.
    #[must_use]
    pub const fn with_max_concurrent_queries(
        mut self,
        max_concurrent_queries: NonZeroUsize,
    ) -> Self {
        self.max_concurrent_queries = max_concurrent_queries;
        self
    }

    /// Return the maximum number of active Query lifecycles.
    #[must_use]
    pub const fn max_concurrent_queries(&self) -> NonZeroUsize {
        self.max_concurrent_queries
    }
}

impl Default for StudioApiOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Host-side Studio API bound to one project and Explainability namespace.
///
/// The returned Router does not bind a socket. It is intended for trusted/local deployment;
/// production exposure requires an authorization layer outside this service.
#[derive(Clone)]
#[non_exhaustive]
pub struct StudioApiService {
    state: Arc<StudioApiState>,
}

impl fmt::Debug for StudioApiService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StudioApiService { .. }")
    }
}

impl StudioApiService {
    /// Bind Studio APIs to a `GraphLoom` project, Store, and matching Live Hub namespace.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::{path::PathBuf, sync::Arc};
    ///
    /// use graphloom::{
    ///     GraphRagConfig,
    ///     explainability::{
    ///         ExplainabilityLiveHub, ExplainabilityLiveHubOptions, ExplainabilityStore,
    ///         InMemoryExplainabilityStore,
    ///     },
    /// };
    /// use graphloom_studio::api::{StudioApiOptions, StudioApiService};
    ///
    /// let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    /// let hub = Arc::new(ExplainabilityLiveHub::new(
    ///     ExplainabilityLiveHubOptions::new(),
    /// ));
    /// let service = StudioApiService::new(
    ///     GraphRagConfig::default(),
    ///     PathBuf::from("."),
    ///     store,
    ///     hub,
    ///     StudioApiOptions::new(),
    /// );
    /// let _router = service.router();
    /// ```
    #[must_use]
    pub fn new(
        config: GraphRagConfig,
        project_root: PathBuf,
        store: Arc<dyn ExplainabilityStore>,
        live_hub: Arc<ExplainabilityLiveHub>,
        options: StudioApiOptions,
    ) -> Self {
        Self::with_runner(
            project_root,
            store,
            live_hub,
            options,
            Arc::new(GraphLoomQueryRunner::new(config)),
        )
    }

    fn with_runner(
        project_root: PathBuf,
        store: Arc<dyn ExplainabilityStore>,
        live_hub: Arc<ExplainabilityLiveHub>,
        options: StudioApiOptions,
        query_runner: Arc<dyn QueryRunner>,
    ) -> Self {
        Self {
            state: Arc::new(StudioApiState {
                project_root,
                store,
                live_hub,
                query_runner,
                query_permits: Arc::new(Semaphore::new(options.max_concurrent_queries().get())),
            }),
        }
    }

    /// Build a Router containing Query, Run metadata/history, and existing SSE routes.
    pub fn router(&self) -> Router {
        let api = Router::new()
            .route("/api/query", axum::routing::post(start_query))
            .route("/api/explainability/runs", get(list_runs))
            .route("/api/explainability/runs/{run_id}", get(get_run))
            .with_state(Arc::clone(&self.state));
        let sse = ExplainabilitySseService::new(
            Arc::clone(&self.state.store),
            Arc::clone(&self.state.live_hub),
        )
        .router();
        api.merge(sse)
    }
}

struct StudioApiState {
    project_root: PathBuf,
    store: Arc<dyn ExplainabilityStore>,
    live_hub: Arc<ExplainabilityLiveHub>,
    query_runner: Arc<dyn QueryRunner>,
    query_permits: Arc<Semaphore>,
}

impl fmt::Debug for StudioApiState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StudioApiState { .. }")
    }
}

#[cfg(test)]
mod tests;
