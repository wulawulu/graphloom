//! Composable Studio Query and Explainability Run HTTP APIs.

mod graph;
mod query;
mod query_result;
mod runs;

use std::{fmt, num::NonZeroUsize, path::PathBuf, sync::Arc};

use axum::{Router, routing::get};
use graphloom::{
    GraphRagConfig,
    explainability::{ExplainabilityLiveHub, ExplainabilityStore},
};
use tokio::sync::Semaphore;

pub use self::query_result::{StudioQueryResult, StudioQueryUsage, StudioQueryUsageCategory};
use self::{
    graph::{
        get_community, get_community_report, get_entity, get_relationship, get_summary,
        list_communities, list_entities, list_relationships,
    },
    query::{GraphLoomQueryRunner, QueryRunner, start_query},
    query_result::{QueryResultRegistry, get_query_result},
    runs::{get_run, list_runs},
};
use crate::{
    explainability::ExplainabilitySseService,
    graph::{GraphDataSource, ParquetGraphDataSource},
};

const DEFAULT_MAX_CONCURRENT_QUERIES: usize = 4;
const DEFAULT_MAX_RETAINED_QUERY_RESULTS: usize = 128;

/// Bounded Query-job admission and current-process result retention options for
/// [`StudioApiService`].
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct StudioApiOptions {
    max_concurrent_queries: NonZeroUsize,
    max_retained_query_results: NonZeroUsize,
}

impl StudioApiOptions {
    /// Create options allowing four concurrent Query jobs and retaining the
    /// latest 128 successful results in FIFO insertion order.
    #[must_use]
    pub fn new() -> Self {
        Self {
            max_concurrent_queries: NonZeroUsize::new(DEFAULT_MAX_CONCURRENT_QUERIES)
                .unwrap_or(NonZeroUsize::MIN),
            max_retained_query_results: NonZeroUsize::new(DEFAULT_MAX_RETAINED_QUERY_RESULTS)
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

    /// Override the maximum number of successful Query results retained in this process.
    #[must_use]
    pub const fn with_max_retained_query_results(
        mut self,
        max_retained_query_results: NonZeroUsize,
    ) -> Self {
        self.max_retained_query_results = max_retained_query_results;
        self
    }

    /// Return the maximum number of successful Query results retained in this process.
    #[must_use]
    pub const fn max_retained_query_results(&self) -> NonZeroUsize {
        self.max_retained_query_results
    }
}

impl Default for StudioApiOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Host-side Studio API bound to one project, graph, and Explainability namespace.
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
    /// Bind Studio APIs to a `GraphLoom` project, its configured output graph,
    /// Store, and matching Live Hub namespace.
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
        let table_root = resolve_table_root(&project_root, &config.output_storage.base_dir);
        Self::with_runner_and_graph_data_source(
            project_root,
            store,
            live_hub,
            options,
            Arc::new(GraphLoomQueryRunner::new(config)),
            Arc::new(ParquetGraphDataSource::new(table_root)),
        )
    }

    /// Bind Studio APIs to a custom graph datasource.
    ///
    /// The datasource must represent the same project graph used by the supplied
    /// Query configuration. This constructor is useful for alternate graph
    /// backends while preserving every HTTP contract.
    #[must_use]
    pub fn new_with_graph_data_source(
        config: GraphRagConfig,
        project_root: PathBuf,
        store: Arc<dyn ExplainabilityStore>,
        live_hub: Arc<ExplainabilityLiveHub>,
        graph_data_source: Arc<dyn GraphDataSource>,
        options: StudioApiOptions,
    ) -> Self {
        Self::with_runner_and_graph_data_source(
            project_root,
            store,
            live_hub,
            options,
            Arc::new(GraphLoomQueryRunner::new(config)),
            graph_data_source,
        )
    }

    #[cfg(test)]
    fn with_runner(
        project_root: PathBuf,
        store: Arc<dyn ExplainabilityStore>,
        live_hub: Arc<ExplainabilityLiveHub>,
        options: StudioApiOptions,
        query_runner: Arc<dyn QueryRunner>,
    ) -> Self {
        let graph_data_source = Arc::new(ParquetGraphDataSource::new(project_root.join("output")));
        Self::with_runner_and_graph_data_source(
            project_root,
            store,
            live_hub,
            options,
            query_runner,
            graph_data_source,
        )
    }

    fn with_runner_and_graph_data_source(
        project_root: PathBuf,
        store: Arc<dyn ExplainabilityStore>,
        live_hub: Arc<ExplainabilityLiveHub>,
        options: StudioApiOptions,
        query_runner: Arc<dyn QueryRunner>,
        graph_data_source: Arc<dyn GraphDataSource>,
    ) -> Self {
        Self {
            state: Arc::new(StudioApiState {
                project_root,
                store,
                live_hub,
                query_runner,
                query_permits: Arc::new(Semaphore::new(options.max_concurrent_queries().get())),
                query_results: Arc::new(QueryResultRegistry::new(
                    options.max_retained_query_results(),
                )),
                graph_data_source,
            }),
        }
    }

    /// Build a Router containing Query, Run metadata/history, SSE, and Graph Explorer routes.
    pub fn router(&self) -> Router {
        let api = Router::new()
            .route("/api/query", axum::routing::post(start_query))
            .route("/api/query/{run_id}/result", get(get_query_result))
            .route("/api/explainability/runs", get(list_runs))
            .route("/api/explainability/runs/{run_id}", get(get_run))
            .route("/api/graph/summary", get(get_summary))
            .route("/api/graph/entities", get(list_entities))
            .route("/api/graph/entities/{entity_id}", get(get_entity))
            .route("/api/graph/relationships", get(list_relationships))
            .route(
                "/api/graph/relationships/{relationship_id}",
                get(get_relationship),
            )
            .route("/api/graph/communities", get(list_communities))
            .route("/api/graph/communities/{community_id}", get(get_community))
            .route(
                "/api/graph/communities/{community_id}/report",
                get(get_community_report),
            )
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
    query_results: Arc<QueryResultRegistry>,
    graph_data_source: Arc<dyn GraphDataSource>,
}

fn resolve_table_root(project_root: &std::path::Path, base_dir: &str) -> PathBuf {
    let configured = std::path::Path::new(base_dir);
    if configured.is_absolute() {
        configured.to_path_buf()
    } else {
        project_root.join(configured)
    }
}

impl fmt::Debug for StudioApiState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StudioApiState { .. }")
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod graph_tests;
