//! Public Query API.

use crate::{
    GraphRagConfig, Result,
    project::LoadedProject,
    query::{
        QueryError, QueryEventStream, QueryOptions, QueryResult, SearchMethod,
        basic::{basic_search as run_basic, basic_search_streaming as run_basic_streaming},
    },
};

/// Execute the selected Query method.
///
/// # Errors
///
/// Returns a typed Query error for invalid configuration, missing data, provider
/// failures, or methods scheduled after the current implementation slice.
pub async fn query(config: GraphRagConfig, options: QueryOptions) -> Result<QueryResult> {
    let project = LoadedProject::from_config(&options.project_root, config)?;
    query_loaded(project, options).await
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
    let project = LoadedProject::from_config(&options.project_root, config)?;
    query_loaded_stream(project, options).await
}

/// Execute Basic Search with the unified options structure.
///
/// # Errors
///
/// Returns a typed Query error when Basic Search cannot load or query the index.
pub async fn basic_search(
    config: GraphRagConfig,
    mut options: QueryOptions,
) -> Result<QueryResult> {
    options.method = SearchMethod::Basic;
    query(config, options).await
}

/// Stream Basic Search events with the unified options structure.
///
/// # Errors
///
/// Returns a typed Query error when Basic Search cannot start.
pub async fn basic_search_streaming(
    config: GraphRagConfig,
    mut options: QueryOptions,
) -> Result<QueryEventStream> {
    options.method = SearchMethod::Basic;
    query_stream(config, options).await
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
        method => Err(unimplemented_method(method).into()),
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
        method => Err(unimplemented_method(method).into()),
    }
}

fn unimplemented_method(method: SearchMethod) -> QueryError {
    QueryError::QueryMethod {
        method: Some(method),
        operation: "dispatch query",
        message: format!(
            "{method} search is recognized but is not provided until its later Phase 2 step"
        ),
    }
}
