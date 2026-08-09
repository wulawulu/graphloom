//! Current-process Query result materialization and retrieval.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fmt,
    num::NonZeroUsize,
    sync::Arc,
};

use axum::{
    Json,
    extract::{Path, State, rejection::PathRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use graphloom::{
    explainability::{ExplainabilityRunId, ExplainabilityRunKind, ExplainabilityRunStatus},
    query::{QueryResult, QueryUsage, QueryUsageCategory},
};
use serde::{Serialize, Serializer};
use thiserror::Error;
use tokio::sync::Mutex;

use super::StudioApiState;

const INVALID_RESULT_REQUEST_BODY: &str = "invalid Studio query result request";
const QUERY_RUN_NOT_FOUND_BODY: &str = "query run not found";
const RESULT_NOT_READY_BODY: &str = "query result is not ready";
const QUERY_NOT_SUCCESSFUL_BODY: &str = "query did not complete successfully";
const RESULT_GONE_BODY: &str = "query result is no longer available";
const RESULT_SERVICE_UNAVAILABLE_BODY: &str = "Studio query result service unavailable";

/// Model usage for one semantic Query operation category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct StudioQueryUsageCategory {
    /// Model calls in this category.
    pub llm_calls: u64,
    /// Input/prompt tokens in this category.
    pub prompt_tokens: u64,
    /// Generated tokens in this category.
    pub output_tokens: u64,
}

/// Provider usage associated with a Studio Query result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct StudioQueryUsage {
    /// Total model calls.
    pub llm_calls: u64,
    /// Total input/prompt tokens.
    pub prompt_tokens: u64,
    /// Total generated tokens.
    pub output_tokens: u64,
    /// Usage by semantic operation, ordered by category name.
    pub categories: BTreeMap<String, StudioQueryUsageCategory>,
}

/// Successful final business result for one Studio Query Run.
///
/// Explainability content mode does not redact this response: it controls the
/// retained execution trace, while this type represents the answer explicitly
/// requested by the Query client. Query context records are intentionally not
/// included.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct StudioQueryResult {
    /// Explainability Run identity shared by result, metadata, and SSE routes.
    pub run_id: ExplainabilityRunId,
    /// Final generated answer.
    pub response: String,
    /// Query wall-clock duration in milliseconds.
    pub elapsed_ms: u64,
    /// Provider usage totals and categories.
    pub usage: StudioQueryUsage,
}

impl fmt::Debug for StudioQueryResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StudioQueryResult { .. }")
    }
}

struct SharedStudioQueryResult(Arc<StudioQueryResult>);

impl fmt::Debug for SharedStudioQueryResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SharedStudioQueryResult { .. }")
    }
}

impl Serialize for SharedStudioQueryResult {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.as_ref().serialize(serializer)
    }
}

#[derive(Clone)]
pub(super) struct QueryExecutionResult {
    response: String,
    elapsed_ms: u64,
    usage: StudioQueryUsage,
}

impl fmt::Debug for QueryExecutionResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("QueryExecutionResult { .. }")
    }
}

impl QueryExecutionResult {
    #[cfg(test)]
    pub(super) fn for_test(response: String, elapsed_ms: u64, usage: StudioQueryUsage) -> Self {
        Self {
            response,
            elapsed_ms,
            usage,
        }
    }

    pub(super) fn with_run_id(self, run_id: ExplainabilityRunId) -> StudioQueryResult {
        StudioQueryResult {
            run_id,
            response: self.response,
            elapsed_ms: self.elapsed_ms,
            usage: self.usage,
        }
    }
}

impl TryFrom<QueryResult> for QueryExecutionResult {
    type Error = QueryResultConversionError;

    fn try_from(result: QueryResult) -> Result<Self, Self::Error> {
        convert_result_parts(result.response, result.elapsed, result.usage)
    }
}

fn convert_result_parts(
    response: String,
    elapsed: std::time::Duration,
    usage: QueryUsage,
) -> Result<QueryExecutionResult, QueryResultConversionError> {
    Ok(QueryExecutionResult {
        response,
        elapsed_ms: checked_elapsed_ms(elapsed)?,
        usage: convert_usage(usage)?,
    })
}

fn convert_usage(usage: QueryUsage) -> Result<StudioQueryUsage, QueryResultConversionError> {
    let categories = usage
        .categories
        .into_iter()
        .map(|(name, category)| convert_usage_category(category).map(|value| (name, value)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    convert_usage_values(
        usage.llm_calls,
        usage.prompt_tokens,
        usage.output_tokens,
        categories,
    )
}

fn convert_usage_category(
    category: QueryUsageCategory,
) -> Result<StudioQueryUsageCategory, QueryResultConversionError> {
    convert_usage_category_values(
        category.llm_calls,
        category.prompt_tokens,
        category.output_tokens,
    )
}

fn convert_usage_values(
    llm_calls: usize,
    prompt_tokens: usize,
    output_tokens: usize,
    categories: BTreeMap<String, StudioQueryUsageCategory>,
) -> Result<StudioQueryUsage, QueryResultConversionError> {
    Ok(StudioQueryUsage {
        llm_calls: checked_counter(llm_calls)?,
        prompt_tokens: checked_counter(prompt_tokens)?,
        output_tokens: checked_counter(output_tokens)?,
        categories,
    })
}

fn convert_usage_category_values(
    llm_calls: usize,
    prompt_tokens: usize,
    output_tokens: usize,
) -> Result<StudioQueryUsageCategory, QueryResultConversionError> {
    Ok(StudioQueryUsageCategory {
        llm_calls: checked_counter(llm_calls)?,
        prompt_tokens: checked_counter(prompt_tokens)?,
        output_tokens: checked_counter(output_tokens)?,
    })
}

fn checked_elapsed_ms(value: std::time::Duration) -> Result<u64, QueryResultConversionError> {
    u64::try_from(value.as_millis()).map_err(|_| QueryResultConversionError::Failed)
}

fn checked_counter(value: usize) -> Result<u64, QueryResultConversionError> {
    u64::try_from(value).map_err(|_| QueryResultConversionError::Failed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum QueryResultConversionError {
    #[error("Studio Query result conversion failed")]
    Failed,
}

pub(super) struct QueryResultRegistry {
    capacity: NonZeroUsize,
    state: Mutex<QueryResultRegistryState>,
}

impl fmt::Debug for QueryResultRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("QueryResultRegistry { .. }")
    }
}

impl QueryResultRegistry {
    pub(super) fn new(capacity: NonZeroUsize) -> Self {
        Self {
            capacity,
            state: Mutex::new(QueryResultRegistryState::default()),
        }
    }

    pub(super) async fn insert(&self, result: StudioQueryResult) {
        let run_id = result.run_id.clone();
        let mut state = self.state.lock().await;
        if state
            .results
            .insert(run_id.clone(), Arc::new(result))
            .is_some()
        {
            state.insertion_order.retain(|existing| existing != &run_id);
        }
        state.insertion_order.push_back(run_id);
        while state.results.len() > self.capacity.get() {
            let Some(oldest) = state.insertion_order.pop_front() else {
                break;
            };
            state.results.remove(&oldest);
        }
    }

    pub(super) async fn get(&self, run_id: &ExplainabilityRunId) -> Option<Arc<StudioQueryResult>> {
        self.state.lock().await.results.get(run_id).cloned()
    }

    pub(super) async fn remove(&self, run_id: &ExplainabilityRunId) {
        let mut state = self.state.lock().await;
        state.results.remove(run_id);
        state.insertion_order.retain(|existing| existing != run_id);
    }
}

#[derive(Debug, Default)]
struct QueryResultRegistryState {
    results: HashMap<ExplainabilityRunId, Arc<StudioQueryResult>>,
    insertion_order: VecDeque<ExplainabilityRunId>,
}

pub(super) async fn get_query_result(
    State(state): State<Arc<StudioApiState>>,
    path: Result<Path<String>, PathRejection>,
) -> Response {
    let Ok(Path(raw_run_id)) = path else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_RESULT_REQUEST_BODY);
    };
    let Ok(run_id) = raw_run_id.parse::<ExplainabilityRunId>() else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_RESULT_REQUEST_BODY);
    };
    let run = match state.store.get_run(&run_id).await {
        Ok(Some(run)) if matches!(run.kind, ExplainabilityRunKind::Query) => run,
        Ok(Some(_) | None) => {
            return fixed_error(StatusCode::NOT_FOUND, QUERY_RUN_NOT_FOUND_BODY);
        }
        Err(_) => {
            return fixed_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                RESULT_SERVICE_UNAVAILABLE_BODY,
            );
        }
    };

    match run.status {
        ExplainabilityRunStatus::Pending | ExplainabilityRunStatus::Running => {
            fixed_error(StatusCode::ACCEPTED, RESULT_NOT_READY_BODY)
        }
        ExplainabilityRunStatus::Failed | ExplainabilityRunStatus::Cancelled => {
            fixed_error(StatusCode::CONFLICT, QUERY_NOT_SUCCESSFUL_BODY)
        }
        ExplainabilityRunStatus::Completed => match state.query_results.get(&run_id).await {
            Some(result) => Json(SharedStudioQueryResult(result)).into_response(),
            None => fixed_error(StatusCode::GONE, RESULT_GONE_BODY),
        },
        _ => fixed_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            RESULT_SERVICE_UNAVAILABLE_BODY,
        ),
    }
}

fn fixed_error(status: StatusCode, body: &'static str) -> Response {
    (status, body).into_response()
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, num::NonZeroUsize, time::Duration};

    use graphloom::{
        explainability::ExplainabilityRunId,
        query::{QueryUsage, QueryUsageCategory},
    };
    use serde_json::Value;

    use super::{
        QueryResultConversionError, QueryResultRegistry, StudioQueryResult, checked_elapsed_ms,
        convert_result_parts, convert_usage_values,
    };

    #[test]
    fn test_should_convert_result_parts_without_context_and_keep_usage_ordered() {
        // QueryResult is non-exhaustive and has no downstream constructor. This exercises the
        // exact helper used after TryFrom extracts its three intentionally exposed fields; the
        // destination type structurally has no QueryContext field.
        let mut completion = QueryUsageCategory::default();
        completion.llm_calls = 2;
        completion.prompt_tokens = 100;
        completion.output_tokens = 45;
        let mut embedding = QueryUsageCategory::default();
        embedding.llm_calls = 1;
        embedding.prompt_tokens = 20;
        let mut usage = QueryUsage::default();
        usage.llm_calls = 3;
        usage.prompt_tokens = 120;
        usage.output_tokens = 45;
        usage.categories = BTreeMap::from([
            ("completion".to_owned(), completion),
            ("embedding".to_owned(), embedding),
        ]);
        let converted = convert_result_parts(
            "RESULT_RESPONSE_SECRET_SENTINEL".to_owned(),
            Duration::from_millis(1_234),
            usage,
        )
        .expect("result conversion")
        .with_run_id("result-run".parse().expect("run id"));
        assert_eq!(converted.elapsed_ms, 1_234);
        assert_eq!(converted.usage.llm_calls, 3);
        assert_eq!(converted.usage.prompt_tokens, 120);
        assert_eq!(converted.usage.output_tokens, 45);
        assert_eq!(
            converted
                .usage
                .categories
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["completion", "embedding"]
        );
        let converted_completion = converted
            .usage
            .categories
            .get("completion")
            .expect("completion usage");
        assert_eq!(converted_completion.llm_calls, 2);
        assert_eq!(converted_completion.prompt_tokens, 100);
        assert_eq!(converted_completion.output_tokens, 45);
        let converted_embedding = converted
            .usage
            .categories
            .get("embedding")
            .expect("embedding usage");
        assert_eq!(converted_embedding.llm_calls, 1);
        assert_eq!(converted_embedding.prompt_tokens, 20);
        assert_eq!(converted_embedding.output_tokens, 0);
        let json = serde_json::to_value(&converted).expect("serialize result");
        assert_eq!(json["response"], "RESULT_RESPONSE_SECRET_SENTINEL");
        for forbidden in ["context", "records", "dataframe", "prompt"] {
            assert!(json.get(forbidden).is_none());
        }
        assert_eq!(format!("{converted:?}"), "StudioQueryResult { .. }");
        assert!(!format!("{converted:?}").contains("RESULT_RESPONSE_SECRET_SENTINEL"));
    }

    #[test]
    fn test_should_reject_elapsed_milliseconds_that_do_not_fit_u64() {
        assert_eq!(
            checked_elapsed_ms(Duration::MAX).expect_err("overflow"),
            QueryResultConversionError::Failed
        );
    }

    #[tokio::test]
    async fn test_should_retain_results_in_bounded_fifo_order() {
        let registry = QueryResultRegistry::new(NonZeroUsize::MIN);
        let first_id: ExplainabilityRunId = "first-result".parse().expect("run id");
        let second_id: ExplainabilityRunId = "second-result".parse().expect("run id");
        registry
            .insert(StudioQueryResult {
                run_id: first_id.clone(),
                response: "first".to_owned(),
                elapsed_ms: 1,
                usage: convert_usage_values(0, 0, 0, BTreeMap::new()).expect("usage"),
            })
            .await;
        registry
            .insert(StudioQueryResult {
                run_id: second_id.clone(),
                response: "second".to_owned(),
                elapsed_ms: 2,
                usage: convert_usage_values(0, 0, 0, BTreeMap::new()).expect("usage"),
            })
            .await;
        assert!(registry.get(&first_id).await.is_none());
        assert_eq!(
            registry
                .get(&second_id)
                .await
                .expect("newest retained")
                .response,
            "second"
        );

        let value: Value = serde_json::to_value(
            registry
                .get(&second_id)
                .await
                .expect("newest retained")
                .as_ref(),
        )
        .expect("serialize");
        assert!(value.get("context").is_none());
    }
}
