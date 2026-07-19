//! Public Query API.

use crate::{
    GraphRagConfig, Result,
    project::LoadedProject,
    query::{
        QueryEngine, QueryEventStream, QueryOptions, QueryResult, SearchMethod,
        basic::{basic_search as run_basic, basic_search_streaming as run_basic_streaming},
        drift::{drift_search as run_drift, drift_search_streaming as run_drift_streaming},
        global::{global_search as run_global, global_search_streaming as run_global_streaming},
        local::{local_search as run_local, local_search_streaming as run_local_streaming},
    },
};

/// Execute the selected Query method.
///
/// # Errors
///
/// Returns a typed Query error for invalid configuration, missing data, provider
/// failures, or methods scheduled after the current implementation slice.
pub async fn query(config: GraphRagConfig, options: QueryOptions) -> Result<QueryResult> {
    match options.method {
        SearchMethod::Global => global_search(config, options).await,
        SearchMethod::Local => local_search(config, options).await,
        SearchMethod::Drift => drift_search(config, options).await,
        SearchMethod::Basic => basic_search(config, options).await,
    }
}

/// Create a streaming event sequence for the selected Query method.
///
/// This constructor is asynchronous because project/runtime validation and the
/// provider's stream handshake can fail before an event stream exists.
///
/// # Errors
///
/// Returns a typed Query error when runtime construction or stream startup fails.
pub async fn query_stream(
    config: GraphRagConfig,
    options: QueryOptions,
) -> Result<QueryEventStream> {
    match options.method {
        SearchMethod::Global => global_search_streaming(config, options).await,
        SearchMethod::Local => local_search_streaming(config, options).await,
        SearchMethod::Drift => drift_search_streaming(config, options).await,
        SearchMethod::Basic => basic_search_streaming(config, options).await,
    }
}

/// Execute Global Search with the unified options structure.
///
/// The method in `options` is always overridden with [`SearchMethod::Global`].
///
/// # Errors
///
/// Returns a typed Query error when Global Search cannot load or query the index.
pub async fn global_search(config: GraphRagConfig, options: QueryOptions) -> Result<QueryResult> {
    execute_query(config, options, SearchMethod::Global).await
}

/// Stream Global Search events with the unified options structure.
///
/// The method in `options` is always overridden with [`SearchMethod::Global`].
///
/// # Errors
///
/// Returns a typed Query error when Global Search cannot start.
pub async fn global_search_streaming(
    config: GraphRagConfig,
    options: QueryOptions,
) -> Result<QueryEventStream> {
    execute_query_stream(config, options, SearchMethod::Global).await
}

/// Execute Local Search with the unified options structure.
///
/// The method in `options` is always overridden with [`SearchMethod::Local`].
///
/// # Errors
///
/// Returns a typed Query error when Local Search cannot load or query the index.
pub async fn local_search(config: GraphRagConfig, options: QueryOptions) -> Result<QueryResult> {
    execute_query(config, options, SearchMethod::Local).await
}

/// Stream Local Search events with the unified options structure.
///
/// The method in `options` is always overridden with [`SearchMethod::Local`].
///
/// # Errors
///
/// Returns a typed Query error when Local Search cannot start.
pub async fn local_search_streaming(
    config: GraphRagConfig,
    options: QueryOptions,
) -> Result<QueryEventStream> {
    execute_query_stream(config, options, SearchMethod::Local).await
}

/// Execute Basic Search with the unified options structure.
///
/// # Errors
///
/// Returns a typed Query error when Basic Search cannot load or query the index.
pub async fn basic_search(config: GraphRagConfig, options: QueryOptions) -> Result<QueryResult> {
    execute_query(config, options, SearchMethod::Basic).await
}

/// Stream Basic Search events with the unified options structure.
///
/// # Errors
///
/// Returns a typed Query error when Basic Search cannot start.
pub async fn basic_search_streaming(
    config: GraphRagConfig,
    options: QueryOptions,
) -> Result<QueryEventStream> {
    execute_query_stream(config, options, SearchMethod::Basic).await
}

/// Execute DRIFT Search with the unified options structure.
///
/// # Errors
///
/// Returns a typed Query error when DRIFT cannot load or query the index.
pub async fn drift_search(config: GraphRagConfig, options: QueryOptions) -> Result<QueryResult> {
    execute_query(config, options, SearchMethod::Drift).await
}

/// Stream DRIFT Search events with the unified options structure.
///
/// # Errors
///
/// Returns a typed Query error when DRIFT cannot start.
pub async fn drift_search_streaming(
    config: GraphRagConfig,
    options: QueryOptions,
) -> Result<QueryEventStream> {
    execute_query_stream(config, options, SearchMethod::Drift).await
}

async fn execute_query(
    config: GraphRagConfig,
    mut options: QueryOptions,
    method: SearchMethod,
) -> Result<QueryResult> {
    options.method = method;
    let engine = QueryEngine::load(config, &options.project_root).await?;
    engine.query(options).await
}

async fn execute_query_stream(
    config: GraphRagConfig,
    mut options: QueryOptions,
    method: SearchMethod,
) -> Result<QueryEventStream> {
    options.method = method;
    let engine = QueryEngine::load(config, &options.project_root).await?;
    engine.query_stream(options).await
}

pub(crate) async fn query_loaded(
    project: LoadedProject,
    options: QueryOptions,
) -> Result<QueryResult> {
    match options.method {
        SearchMethod::Basic => {
            let runtime =
                crate::query::QueryRuntimeFactory::build_basic(&project, &options).await?;
            Ok(run_basic(runtime, &options.query, &options.response_type).await?)
        }
        SearchMethod::Local => {
            let runtime =
                crate::query::QueryRuntimeFactory::build_local(&project, &options).await?;
            Ok(run_local(
                runtime,
                &options.query,
                &options.response_type,
                options.conversation_history.as_ref(),
            )
            .await?)
        }
        SearchMethod::Global => {
            let runtime =
                crate::query::QueryRuntimeFactory::build_global(&project, &options).await?;
            Ok(run_global(runtime, &options.query, &options.response_type).await?)
        }
        SearchMethod::Drift => {
            let runtime =
                crate::query::QueryRuntimeFactory::build_drift(&project, &options).await?;
            Ok(run_drift(runtime, &options.query, &options.response_type).await?)
        }
    }
}

pub(crate) async fn query_loaded_stream(
    project: LoadedProject,
    options: QueryOptions,
) -> Result<QueryEventStream> {
    match options.method {
        SearchMethod::Basic => {
            let runtime =
                crate::query::QueryRuntimeFactory::build_basic(&project, &options).await?;
            Ok(run_basic_streaming(runtime, &options.query, &options.response_type).await?)
        }
        SearchMethod::Local => {
            let runtime =
                crate::query::QueryRuntimeFactory::build_local(&project, &options).await?;
            Ok(run_local_streaming(
                runtime,
                &options.query,
                &options.response_type,
                options.conversation_history.as_ref(),
            )
            .await?)
        }
        SearchMethod::Global => {
            let runtime =
                crate::query::QueryRuntimeFactory::build_global(&project, &options).await?;
            Ok(run_global_streaming(runtime, &options.query, &options.response_type).await?)
        }
        SearchMethod::Drift => {
            let runtime =
                crate::query::QueryRuntimeFactory::build_drift(&project, &options).await?;
            Ok(run_drift_streaming(runtime, &options.query, &options.response_type).await?)
        }
    }
}
