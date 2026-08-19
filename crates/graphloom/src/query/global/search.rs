//! Global map/reduce request construction and shared streaming orchestration.

use std::{cmp::Ordering, collections::BTreeMap, sync::Arc, time::Instant};

use futures_util::{StreamExt, stream};
use graphloom_llm::{ChatMessage, CompletionModel, CompletionRequest, ModelConfig, Tokenizer};
use polars_core::prelude::DataFrame;
use serde::Serialize;
use serde_json::json;

use super::{
    context::{GlobalContextResult, global_context},
    dynamic::DynamicCommunitySelection,
    parse::{MapSearchResult, parse_map_points},
};
use crate::{
    explainability::{
        ExplainabilityEvent, GlobalContextBuilt, GlobalMapBatchBuilt, GlobalMapPointDecision,
        GlobalMapPointDecisionReason, GlobalMapPointEvidence, GlobalMapPointsProduced,
        GlobalMapStarted, GlobalReduceContextBuilt, GlobalReduceSkipReason, GlobalReduceSkipped,
        LlmRequestCompleted, LlmRequestStarted,
    },
    prompts::PromptTemplate,
    query::{
        GlobalQueryRuntime, QueryContext, QueryError, QueryEvent, QueryEventStream,
        QueryInstrumentation, QueryResult, QueryUsage, QueryUsageCategory, Result, SearchMethod,
        concurrency::try_buffered_ordered,
        context::ContextTable,
        explainability::GlobalQueryExplainability,
        result::count_completion_input,
        streaming::{CompletionStreamState, completion_event_stream},
    },
};

const NO_DATA_ANSWER: &str =
    "I am sorry but I am unable to answer this question given the provided data.";

#[derive(Debug, Serialize)]
struct MapPromptContext<'a> {
    context_data: &'a str,
    max_length: usize,
}

#[derive(Debug, Serialize)]
struct ReducePromptContext<'a> {
    report_data: &'a str,
    response_type: &'a str,
    max_length: usize,
}

pub(crate) async fn global_search(
    runtime: GlobalQueryRuntime,
    query: &str,
    response_type: &str,
    instrumentation: Option<QueryInstrumentation>,
) -> Result<QueryResult> {
    let mut events =
        global_search_streaming(runtime, query, response_type, instrumentation).await?;
    while let Some(event) = events.next().await {
        if let QueryEvent::Completed(result) = event? {
            return Ok(result);
        }
    }
    Err(QueryError::QueryCompletion {
        method: SearchMethod::Global,
        operation: "aggregate Global Search stream",
        model: "unknown".to_owned(),
        source: Box::new(graphloom_llm::LlmError::InvalidResponse {
            model_instance: "unknown".to_owned(),
            operation: "query stream",
            message: "stream ended without a completed event".to_owned(),
        }),
    })
}

pub(crate) async fn global_search_streaming(
    runtime: GlobalQueryRuntime,
    query: &str,
    response_type: &str,
    instrumentation: Option<QueryInstrumentation>,
) -> Result<QueryEventStream> {
    match prepare_global_stream(runtime, query, response_type, instrumentation.clone()).await {
        Ok(events) => Ok(events),
        Err(error) => {
            if let Some(instrumentation) = instrumentation {
                instrumentation.finish_query_error(&error).await;
            }
            Err(error)
        }
    }
}

async fn prepare_global_stream(
    runtime: GlobalQueryRuntime,
    query: &str,
    response_type: &str,
    instrumentation: Option<QueryInstrumentation>,
) -> Result<QueryEventStream> {
    let started = Instant::now();
    let explainability = instrumentation
        .as_ref()
        .and_then(QueryInstrumentation::global_explainability);
    let built = if runtime.dynamic_community_selection {
        let selection = DynamicCommunitySelection::new(
            runtime.global_context.config.clone(),
            runtime.global_context.reports.clone(),
            runtime.global_context.communities.clone(),
            Arc::clone(&runtime.completion_model),
            runtime.completion_model_id.clone(),
            runtime.completion_config.clone(),
            Arc::clone(&runtime.global_context.tokenizer),
            runtime.concurrent_requests,
        )
        .select(query)
        .await?;
        runtime.global_context.build_selected(
            selection.reports,
            selection.usage,
            selection.ratings,
        )?
    } else {
        runtime
            .global_context
            .build_fixed_explainable(explainability.is_some())?
    };
    emit_global_context(&built, explainability).await;
    runtime.callbacks.on_map_response_start(&built.batches);
    let map_outputs = run_map_calls(
        &built,
        query,
        Arc::clone(&runtime.completion_model),
        &runtime.completion_model_id,
        &runtime.completion_config,
        &runtime.map_prompt,
        Arc::clone(&runtime.global_context.tokenizer),
        runtime.concurrent_requests,
        runtime.global_context.config().map_max_length,
        runtime.global_context.config().max_context_tokens,
        explainability.cloned(),
    )
    .await?;
    runtime.callbacks.on_map_response_end(&map_outputs);

    let mut reduce = build_reduce_context(
        &map_outputs,
        runtime.global_context.config().data_max_tokens,
        runtime.global_context.tokenizer.as_ref(),
        explainability.is_some(),
        explainability.is_some_and(|session| session.includes_content()),
    )?;
    emit_reduce_context(
        &mut reduce,
        runtime.global_context.config().data_max_tokens,
        explainability,
    )
    .await;
    let report_data = reduce.text;
    let context = global_context(
        &built,
        report_data.clone(),
        map_outputs_frame(&map_outputs)?,
    )?;
    runtime.callbacks.on_context(&context);
    let map_usage = sum_map_usage(&map_outputs);
    let build_usage = built.usage;

    if !reduce.has_positive_points {
        if let Some(explainability) = explainability {
            explainability
                .emit(
                    explainability.spans().reduce(),
                    Some(explainability.root_span()),
                    ExplainabilityEvent::GlobalReduceSkipped(GlobalReduceSkipped::new(
                        GlobalReduceSkipReason::NoPositivePoints,
                    )),
                )
                .await;
        }
        return Ok(instrument_global_completion_stream(
            no_data_stream(context, started, build_usage, map_usage),
            instrumentation,
            None,
        ));
    }

    let rendered = runtime
        .reduce_prompt
        .bind(&ReducePromptContext {
            report_data: &report_data,
            response_type,
            max_length: runtime.global_context.config().reduce_max_length,
        })
        .and_then(|prompt| prompt.render())
        .map_err(|source| QueryError::QueryPrompt {
            method: SearchMethod::Global,
            operation: "render Global Search reduce prompt",
            prompt: "global_search_reduce_system_prompt.txt",
            source: Box::new(source),
        })?;
    let mut request = CompletionRequest::new(vec![
        ChatMessage::system(rendered),
        ChatMessage::user(query),
    ]);
    let reduce_prompt_tokens = count_completion_input(
        runtime.global_context.tokenizer.as_ref(),
        &request.messages,
        SearchMethod::Global,
        "count Global reduce completion input tokens",
    )?;
    request
        .apply_call_args(&runtime.completion_config.call_args)
        .and_then(|()| {
            request.stream = Some(true);
            request.validate()
        })
        .map_err(|source| QueryError::InvalidQueryConfig {
            method: SearchMethod::Global,
            operation: "build Global Search reduce request",
            message: source.to_string(),
        })?;
    runtime.callbacks.on_reduce_response_start(&report_data);
    emit_llm_started(
        explainability,
        explainability.map(|value| value.spans().reduce()),
        runtime.completion_model_id.as_str(),
        reduce_prompt_tokens,
        request
            .messages
            .first()
            .map(|message| message.content.as_str()),
    )
    .await;
    let reduce_started = Instant::now();
    let provider = runtime
        .completion_model
        .stream(request)
        .await
        .map_err(|source| QueryError::QueryCompletion {
            method: SearchMethod::Global,
            operation: "start Global Search reduce stream",
            model: runtime.completion_model_id.clone(),
            source: Box::new(source),
        })?;
    let reduce_completion_model_id = runtime.completion_model_id.clone();
    let state = CompletionStreamState {
        provider,
        context,
        started,
        categories: BTreeMap::from([
            ("build_context".to_owned(), build_usage),
            ("map".to_owned(), map_usage),
        ]),
        completion_category: "reduce",
        prompt_tokens: reduce_prompt_tokens,
        tokenizer: Arc::clone(&runtime.global_context.tokenizer),
        callbacks: runtime.callbacks,
        completion_model_id: reduce_completion_model_id.clone(),
        method: SearchMethod::Global,
        consume_operation: "consume Global Search reduce stream",
        output_count_operation: "count Global reduce output tokens",
        output_count_is_context_error: true,
        notify_reduce_end: true,
    };
    Ok(instrument_global_completion_stream(
        completion_event_stream(state),
        instrumentation,
        Some(GlobalReduceCompletion {
            started: reduce_started,
            model_id: reduce_completion_model_id,
        }),
    ))
}

#[allow(clippy::too_many_arguments)]
async fn run_map_calls(
    built: &GlobalContextResult,
    query: &str,
    model: Arc<dyn CompletionModel>,
    model_id: &str,
    model_config: &ModelConfig,
    prompt: &PromptTemplate,
    tokenizer: Arc<dyn Tokenizer>,
    concurrent_requests: usize,
    max_length: usize,
    context_token_budget: usize,
    explainability: Option<GlobalQueryExplainability>,
) -> Result<Vec<MapSearchResult>> {
    if let Some(explainability) = &explainability
        && let Some(batch_count) = explainability.usize_to_u32(built.batches.len())
    {
        explainability
            .emit(
                explainability.spans().map(),
                Some(explainability.root_span()),
                ExplainabilityEvent::GlobalMapStarted(GlobalMapStarted::new(batch_count)),
            )
            .await;
    }
    let futures = built
        .batches
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, context)| {
            let model = Arc::clone(&model);
            let tokenizer = Arc::clone(&tokenizer);
            let model_id = model_id.to_owned();
            let call_args = model_config.call_args.clone();
            let prompt = prompt.clone();
            let query = query.to_owned();
            let report_ids = if let Some(handle) = &explainability {
                match built.batch_report_ids.get(index) {
                    Some(ids) => ids.clone(),
                    None => {
                        handle.mark_sidecar_failure("missing_batch_report_ids");
                        Vec::new()
                    }
                }
            } else {
                Vec::new()
            };
            let explainability = explainability.clone();
            async move {
                run_map_call(
                    index,
                    context,
                    &query,
                    model,
                    &model_id,
                    &call_args,
                    &prompt,
                    tokenizer,
                    max_length,
                    context_token_budget,
                    report_ids,
                    explainability,
                )
                .await
            }
        });
    try_buffered_ordered(futures, concurrent_requests).await
}

#[allow(clippy::too_many_arguments)]
async fn run_map_call(
    batch_index: usize,
    context: String,
    query: &str,
    model: Arc<dyn CompletionModel>,
    model_id: &str,
    call_args: &BTreeMap<String, serde_json::Value>,
    prompt: &PromptTemplate,
    tokenizer: Arc<dyn Tokenizer>,
    max_length: usize,
    context_token_budget: usize,
    report_ids: Vec<String>,
    explainability: Option<GlobalQueryExplainability>,
) -> Result<MapSearchResult> {
    let batch_span = explainability
        .as_ref()
        .map(GlobalQueryExplainability::batch_span);
    emit_map_batch_built(
        batch_index,
        &context,
        report_ids,
        tokenizer.as_ref(),
        context_token_budget,
        explainability.as_ref(),
        batch_span.as_ref(),
    )
    .await;
    let rendered = prompt
        .bind(&MapPromptContext {
            context_data: &context,
            max_length,
        })
        .and_then(|prompt| prompt.render())
        .map_err(|source| QueryError::QueryPrompt {
            method: SearchMethod::Global,
            operation: "render Global Search map prompt",
            prompt: "global_search_map_system_prompt.txt",
            source: Box::new(source),
        })?;
    let mut request = CompletionRequest::new(vec![
        ChatMessage::system(rendered),
        ChatMessage::user(query),
    ]);
    let prompt_tokens = count_completion_input(
        tokenizer.as_ref(),
        &request.messages,
        SearchMethod::Global,
        "count Global map completion input tokens",
    )?;
    request
        .apply_call_args(call_args)
        .and_then(|()| {
            request.stream = Some(false);
            request.response_format = Some(json!({"type": "json_object"}));
            request.validate()
        })
        .map_err(|source| QueryError::InvalidQueryConfig {
            method: SearchMethod::Global,
            operation: "build Global Search map request",
            message: source.to_string(),
        })?;
    emit_llm_started(
        explainability.as_ref(),
        batch_span.as_ref(),
        model_id,
        prompt_tokens,
        request
            .messages
            .first()
            .map(|message| message.content.as_str()),
    )
    .await;
    let llm_started = Instant::now();
    let response = model
        .complete(request)
        .await
        .map_err(|source| QueryError::QueryCompletion {
            method: SearchMethod::Global,
            operation: "complete Global Search map call",
            model: model_id.to_owned(),
            source: Box::new(source),
        })?;
    let raw_response = response
        .content()
        .map_err(|source| QueryError::QueryCompletion {
            method: SearchMethod::Global,
            operation: "read Global Search map response",
            model: model_id.to_owned(),
            source: Box::new(source),
        })?
        .to_owned();
    let output_tokens = count(
        tokenizer.as_ref(),
        &raw_response,
        "count Global map output tokens",
    )?;
    emit_llm_completed(
        explainability.as_ref(),
        batch_span.as_ref(),
        model_id,
        prompt_tokens,
        output_tokens,
        llm_started,
        &raw_response,
    )
    .await;
    let points = parse_map_points(&raw_response);
    emit_map_points(
        batch_index,
        &points,
        explainability.as_ref(),
        batch_span.as_ref(),
    )
    .await;
    Ok(MapSearchResult {
        batch_index,
        points,
        raw_response,
        context,
        usage: QueryUsageCategory {
            llm_calls: 1,
            prompt_tokens,
            output_tokens,
        },
    })
}

#[derive(Debug)]
struct GlobalReduceContextResult {
    text: String,
    has_positive_points: bool,
    candidate_count: usize,
    positive_count: usize,
    selected_count: usize,
    tokens_used: usize,
    truncated: bool,
    decisions: Vec<ReducePointDecision>,
}

#[derive(Debug)]
struct ReducePointDecision {
    batch_index: usize,
    point_index: usize,
    score: i64,
    selected: bool,
    reason: GlobalMapPointDecisionReason,
    answer: Option<String>,
}

fn build_reduce_context(
    outputs: &[MapSearchResult],
    max_tokens: usize,
    tokenizer: &dyn Tokenizer,
    capture_decisions: bool,
    capture_content: bool,
) -> Result<GlobalReduceContextResult> {
    let mut decisions = Vec::new();
    let mut decision_positions = BTreeMap::new();
    if capture_decisions {
        for output in outputs {
            decisions.reserve(output.points.len());
            for (point_index, point) in output.points.iter().enumerate() {
                decision_positions.insert((output.batch_index, point_index), decisions.len());
                decisions.push(ReducePointDecision {
                    batch_index: output.batch_index,
                    point_index,
                    score: point.score,
                    selected: false,
                    reason: if point.score > 0 {
                        GlobalMapPointDecisionReason::TokenBudget
                    } else {
                        GlobalMapPointDecisionReason::NonPositiveScore
                    },
                    answer: capture_content.then(|| point.answer.clone()),
                });
            }
        }
    }
    let candidate_count = decisions.len();
    let mut points = outputs
        .iter()
        .flat_map(|output| {
            output
                .points
                .iter()
                .enumerate()
                .filter(|(_, point)| point.score > 0)
                .map(move |(point_index, point)| (output.batch_index, point_index, point))
        })
        .collect::<Vec<_>>();
    points.sort_by(|left, right| {
        right
            .2
            .score
            .cmp(&left.2.score)
            .then_with(|| Ordering::Equal)
    });
    let has_positive_points = !points.is_empty();
    let positive_count = points.len();
    let mut selected = Vec::new();
    let mut tokens = 0_usize;
    let mut truncated = false;
    for (batch_index, point_index, point) in points {
        let text = format!(
            "----Analyst {}----\nImportance Score: {}\n{}",
            batch_index + 1,
            point.score,
            point.answer
        );
        let point_tokens = count(tokenizer, &text, "count Global reduce point tokens")?;
        if tokens.saturating_add(point_tokens) > max_tokens {
            truncated = true;
            break;
        }
        tokens = tokens.saturating_add(point_tokens);
        if let Some(decision_index) = decision_positions.get(&(batch_index, point_index))
            && let Some(decision) = decisions.get_mut(*decision_index)
        {
            decision.selected = true;
            decision.reason = GlobalMapPointDecisionReason::Selected;
        }
        selected.push(text);
    }
    let selected_count = selected.len();
    Ok(GlobalReduceContextResult {
        text: selected.join("\n\n"),
        has_positive_points,
        candidate_count,
        positive_count,
        selected_count,
        tokens_used: tokens,
        truncated,
        decisions,
    })
}

fn map_outputs_frame(outputs: &[MapSearchResult]) -> Result<DataFrame> {
    let rows = outputs
        .iter()
        .map(|output| {
            let points = serde_json::Value::Array(
                output
                    .points
                    .iter()
                    .map(|point| {
                        serde_json::json!({
                            "answer": point.answer,
                            "score": point.score,
                        })
                    })
                    .collect(),
            )
            .to_string();
            vec![
                output.batch_index.to_string(),
                output.raw_response.clone(),
                output.context.clone(),
                points,
                output.usage.llm_calls.to_string(),
                output.usage.prompt_tokens.to_string(),
                output.usage.output_tokens.to_string(),
            ]
        })
        .collect::<Vec<_>>();
    ContextTable::new(
        [
            "batch_index",
            "raw_response",
            "context",
            "points",
            "llm_calls",
            "prompt_tokens",
            "output_tokens",
        ],
        rows,
    )
    .to_dataframe(SearchMethod::Global, "build Global map output records")
}

fn sum_map_usage(outputs: &[MapSearchResult]) -> QueryUsageCategory {
    outputs
        .iter()
        .fold(QueryUsageCategory::default(), |mut total, output| {
            total += output.usage;
            total
        })
}

fn no_data_stream(
    context: QueryContext,
    started: Instant,
    build_usage: QueryUsageCategory,
    map_usage: QueryUsageCategory,
) -> QueryEventStream {
    let result = QueryResult {
        response: NO_DATA_ANSWER.to_owned(),
        context: context.clone(),
        elapsed: started.elapsed(),
        usage: QueryUsage::from_categories(BTreeMap::from([
            ("build_context".to_owned(), build_usage),
            ("map".to_owned(), map_usage),
            ("reduce".to_owned(), QueryUsageCategory::default()),
        ])),
    };
    Box::pin(stream::iter(vec![
        Ok(QueryEvent::Context(context)),
        Ok(QueryEvent::Token(NO_DATA_ANSWER.to_owned())),
        Ok(QueryEvent::Completed(result)),
    ]))
}

async fn emit_global_context(
    built: &GlobalContextResult,
    explainability: Option<&GlobalQueryExplainability>,
) {
    let Some(explainability) = explainability else {
        return;
    };
    let report_count = built.batch_report_ids.iter().map(Vec::len).sum::<usize>();
    let Some(batch_count) = explainability.usize_to_u32(built.batches.len()) else {
        return;
    };
    let Some(report_count) = explainability.usize_to_u32(report_count) else {
        return;
    };
    explainability
        .emit(
            explainability.spans().context(),
            Some(explainability.root_span()),
            ExplainabilityEvent::GlobalContextBuilt(GlobalContextBuilt::new(
                batch_count,
                report_count,
            )),
        )
        .await;
}

async fn emit_map_batch_built(
    batch_index: usize,
    context: &str,
    report_ids: Vec<String>,
    tokenizer: &dyn Tokenizer,
    token_budget: usize,
    explainability: Option<&GlobalQueryExplainability>,
    batch_span: Option<&crate::explainability::ExplainabilitySpanId>,
) {
    let (Some(explainability), Some(batch_span)) = (explainability, batch_span) else {
        return;
    };
    let Some(batch_index) = explainability.usize_to_u32(batch_index) else {
        return;
    };
    let Some(token_budget) = explainability.usize_to_u64(token_budget) else {
        return;
    };
    let tokens_used = match tokenizer.count(context) {
        Ok(value) => explainability.usize_to_u64(value),
        Err(_) => {
            explainability.mark_sidecar_failure("context_token_count");
            None
        }
    };
    let Some(tokens_used) = tokens_used else {
        return;
    };
    let mut event = GlobalMapBatchBuilt::new(batch_index, report_ids, tokens_used, token_budget);
    event.context = explainability.content(context);
    explainability
        .emit(
            batch_span,
            Some(explainability.spans().map()),
            ExplainabilityEvent::GlobalMapBatchBuilt(event),
        )
        .await;
}

async fn emit_llm_started(
    explainability: Option<&GlobalQueryExplainability>,
    span: Option<&crate::explainability::ExplainabilitySpanId>,
    model_id: &str,
    prompt_tokens: usize,
    prompt: Option<&str>,
) {
    let (Some(explainability), Some(span), Some(prompt_tokens)) = (
        explainability,
        span,
        explainability.and_then(|value| value.usize_to_u64(prompt_tokens)),
    ) else {
        return;
    };
    let mut event = LlmRequestStarted::new(model_id.to_owned(), prompt_tokens);
    event.prompt = prompt.and_then(|value| explainability.content(value));
    explainability
        .emit(
            span,
            Some(global_llm_parent(explainability, span)),
            ExplainabilityEvent::LlmRequestStarted(event),
        )
        .await;
}

async fn emit_llm_completed(
    explainability: Option<&GlobalQueryExplainability>,
    span: Option<&crate::explainability::ExplainabilitySpanId>,
    model_id: &str,
    input_tokens: usize,
    output_tokens: usize,
    started: Instant,
    response: &str,
) {
    let (Some(explainability), Some(span)) = (explainability, span) else {
        return;
    };
    let (Some(input_tokens), Some(output_tokens), Some(elapsed_ms)) = (
        explainability.usize_to_u64(input_tokens),
        explainability.usize_to_u64(output_tokens),
        explainability.duration_millis(started.elapsed()),
    ) else {
        return;
    };
    let mut event =
        LlmRequestCompleted::new(model_id.to_owned(), input_tokens, output_tokens, elapsed_ms);
    event.response = explainability.content(response);
    explainability
        .emit(
            span,
            Some(global_llm_parent(explainability, span)),
            ExplainabilityEvent::LlmRequestCompleted(event),
        )
        .await;
}

fn global_llm_parent<'a>(
    explainability: &'a GlobalQueryExplainability,
    span: &crate::explainability::ExplainabilitySpanId,
) -> &'a crate::explainability::ExplainabilitySpanId {
    if span == explainability.spans().reduce() {
        explainability.root_span()
    } else {
        explainability.spans().map()
    }
}

async fn emit_map_points(
    batch_index: usize,
    points: &[super::parse::MapPoint],
    explainability: Option<&GlobalQueryExplainability>,
    batch_span: Option<&crate::explainability::ExplainabilitySpanId>,
) {
    let (Some(explainability), Some(batch_span), Some(batch_index)) = (
        explainability,
        batch_span,
        explainability.and_then(|value| value.usize_to_u32(batch_index)),
    ) else {
        return;
    };
    let mut evidence = Vec::with_capacity(points.len());
    for (point_index, point) in points.iter().enumerate() {
        let Some(point_index) = explainability.usize_to_u32(point_index) else {
            return;
        };
        evidence.push(GlobalMapPointEvidence {
            batch_index,
            point_index,
            score: point.score,
            answer: explainability.content(&point.answer),
        });
    }
    let event = match GlobalMapPointsProduced::try_new(batch_index, evidence) {
        Ok(event) => event,
        Err(_) => {
            explainability.mark_sidecar_failure("map_point_contract");
            return;
        }
    };
    explainability
        .emit(
            batch_span,
            Some(explainability.spans().map()),
            ExplainabilityEvent::GlobalMapPointsProduced(event),
        )
        .await;
}

async fn emit_reduce_context(
    reduce: &mut GlobalReduceContextResult,
    token_budget: usize,
    explainability: Option<&GlobalQueryExplainability>,
) {
    let Some(explainability) = explainability else {
        return;
    };
    let (
        Some(candidate_point_count),
        Some(positive_point_count),
        Some(selected_point_count),
        Some(token_budget),
        Some(tokens_used),
    ) = (
        explainability.usize_to_u64(reduce.candidate_count),
        explainability.usize_to_u64(reduce.positive_count),
        explainability.usize_to_u64(reduce.selected_count),
        explainability.usize_to_u64(token_budget),
        explainability.usize_to_u64(reduce.tokens_used),
    )
    else {
        return;
    };
    let mut points = Vec::with_capacity(reduce.decisions.len());
    for decision in &mut reduce.decisions {
        let (Some(batch_index), Some(point_index)) = (
            explainability.usize_to_u32(decision.batch_index),
            explainability.usize_to_u32(decision.point_index),
        ) else {
            return;
        };
        points.push(GlobalMapPointDecision {
            batch_index,
            point_index,
            score: decision.score,
            selected: decision.selected,
            reason: decision.reason,
            answer: decision.answer.take(),
        });
    }
    let event = GlobalReduceContextBuilt {
        candidate_point_count,
        positive_point_count,
        selected_point_count,
        token_budget,
        tokens_used,
        truncated: reduce.truncated,
        points,
        context: explainability.content(&reduce.text),
    };
    explainability
        .emit(
            explainability.spans().reduce(),
            Some(explainability.root_span()),
            ExplainabilityEvent::GlobalReduceContextBuilt(event),
        )
        .await;
}

#[derive(Debug)]
struct GlobalReduceCompletion {
    started: Instant,
    model_id: String,
}

struct GlobalCompletionState {
    events: QueryEventStream,
    instrumentation: Option<QueryInstrumentation>,
    reduce: Option<GlobalReduceCompletion>,
}

fn instrument_global_completion_stream(
    events: QueryEventStream,
    instrumentation: Option<QueryInstrumentation>,
    reduce: Option<GlobalReduceCompletion>,
) -> QueryEventStream {
    Box::pin(stream::unfold(
        Some(GlobalCompletionState {
            events,
            instrumentation,
            reduce,
        }),
        next_global_completion_event,
    ))
}

async fn next_global_completion_event(
    state: Option<GlobalCompletionState>,
) -> Option<(Result<QueryEvent>, Option<GlobalCompletionState>)> {
    let mut state = state?;
    match state.events.next().await {
        Some(Ok(QueryEvent::Completed(result))) => {
            if let (Some(instrumentation), Some(reduce)) =
                (state.instrumentation.as_ref(), state.reduce.as_ref())
                && let Some(explainability) = instrumentation.global_explainability()
            {
                if let Some(usage) = result.usage.categories.get("reduce") {
                    emit_llm_completed(
                        Some(explainability),
                        Some(explainability.spans().reduce()),
                        &reduce.model_id,
                        usage.prompt_tokens,
                        usage.output_tokens,
                        reduce.started,
                        &result.response,
                    )
                    .await;
                } else {
                    explainability.mark_sidecar_failure("missing_llm_usage");
                }
            }
            if let Some(instrumentation) = &state.instrumentation {
                instrumentation.finish_explainability_success().await;
            }
            Some((Ok(QueryEvent::Completed(result)), None))
        }
        Some(Ok(event)) => Some((Ok(event), Some(state))),
        Some(Err(error)) => {
            if let Some(instrumentation) = &state.instrumentation {
                instrumentation.finish_query_error(&error).await;
            }
            Some((Err(error), None))
        }
        None => {
            if let Some(instrumentation) = &state.instrumentation {
                instrumentation.finish_stream_ended().await;
            }
            None
        }
    }
}

fn count(tokenizer: &dyn Tokenizer, text: &str, operation: &'static str) -> Result<usize> {
    tokenizer
        .count(text)
        .map_err(|source| QueryError::QueryContext {
            method: SearchMethod::Global,
            operation,
            message: source.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use async_trait::async_trait;
    use futures_util::StreamExt;
    use graphloom_llm::{
        CompletionChunk, CompletionModel, CompletionRequest, CompletionResponse, LlmError,
        ModelConfig, Tokenizer,
    };

    use super::{
        build_reduce_context, instrument_global_completion_stream, map_outputs_frame, run_map_calls,
    };
    use crate::{
        explainability::{
            ExplainabilityContentMode, ExplainabilityEvent, ExplainabilityRecord,
            ExplainabilityRunId, ExplainabilitySink, ExplainabilitySinkError,
            GlobalMapPointDecisionReason,
        },
        prompts::{PromptKind, PromptRepository},
        query::{
            MapPoint, MapSearchResult, QueryCallbacks, QueryContext, QueryEvent,
            QueryExplainabilityOptions, QueryInstrumentation, QueryOptions, QueryUsageCategory,
            SearchMethod,
            global::context::GlobalContextResult,
            streaming::{CompletionStreamState, completion_event_stream},
        },
    };

    #[derive(Debug)]
    struct WordTokenizer;

    impl Tokenizer for WordTokenizer {
        fn count(&self, text: &str) -> graphloom_llm::Result<usize> {
            Ok(text.split_whitespace().count())
        }

        fn encode(&self, _text: &str) -> graphloom_llm::Result<Vec<u32>> {
            Err(LlmError::Tokenizer {
                encoding_model: "word-test".to_owned(),
                message: "unused".to_owned(),
            })
        }

        fn decode(&self, _tokens: &[u32]) -> graphloom_llm::Result<String> {
            Err(LlmError::Tokenizer {
                encoding_model: "word-test".to_owned(),
                message: "unused".to_owned(),
            })
        }
    }

    #[derive(Debug, Default)]
    struct ConcurrentRecordingModel {
        requests: Mutex<Vec<CompletionRequest>>,
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
    }

    #[async_trait]
    impl CompletionModel for ConcurrentRecordingModel {
        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> graphloom_llm::Result<CompletionResponse> {
            let current = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(current, Ordering::SeqCst);
            let context = request.messages[0].content.as_str();
            let index = (0..4)
                .find(|index| context.contains(&format!("batch-{index}")))
                .ok_or_else(|| LlmError::InvalidResponse {
                    model_instance: "recording".to_owned(),
                    operation: "complete",
                    message: "missing batch marker".to_owned(),
                })?;
            self.requests
                .lock()
                .map_err(|source| LlmError::InvalidResponse {
                    model_instance: "recording".to_owned(),
                    operation: "record request",
                    message: source.to_string(),
                })?
                .push(request);
            tokio::time::sleep(Duration::from_millis((4 - index) * 5)).await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(CompletionResponse::text_for_test(
                "recording",
                format!(
                    r#"{{"points":[{{"description":"answer-{index}","score":{}}}]}}"#,
                    index + 1
                ),
            ))
        }
    }

    #[derive(Debug, Default)]
    struct RequestRecordingModel {
        requests: Mutex<Vec<CompletionRequest>>,
    }

    #[async_trait]
    impl CompletionModel for RequestRecordingModel {
        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> graphloom_llm::Result<CompletionResponse> {
            self.requests
                .lock()
                .map_err(|source| LlmError::InvalidResponse {
                    model_instance: "request-recording".to_owned(),
                    operation: "record request",
                    message: source.to_string(),
                })?
                .push(request);
            Ok(CompletionResponse::text_for_test(
                "request-recording",
                r#"{"points":[{"description":"answer","score":1}]}"#,
            ))
        }
    }

    #[derive(Debug)]
    struct FailingModel;

    #[async_trait]
    impl CompletionModel for FailingModel {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> graphloom_llm::Result<CompletionResponse> {
            Err(LlmError::Timeout {
                model_instance: "failing".to_owned(),
                operation: "complete",
                attempts: 1,
            })
        }
    }

    #[derive(Debug, Default)]
    struct RecordingSink {
        records: Mutex<Vec<Arc<ExplainabilityRecord>>>,
        finishes: AtomicUsize,
    }

    #[async_trait]
    impl ExplainabilitySink for RecordingSink {
        async fn emit(
            &self,
            record: Arc<ExplainabilityRecord>,
        ) -> std::result::Result<(), ExplainabilitySinkError> {
            self.records
                .lock()
                .map_err(|_| ExplainabilitySinkError::RecordNotAccepted)?
                .push(record);
            Ok(())
        }

        async fn finish_run(
            &self,
            _run_id: &ExplainabilityRunId,
        ) -> std::result::Result<(), ExplainabilitySinkError> {
            self.finishes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct ReduceRecordingCallback {
        events: Mutex<Vec<String>>,
    }

    impl QueryCallbacks for ReduceRecordingCallback {
        fn on_reduce_response_start(&self, _context: &str) {
            self.events
                .lock()
                .expect("events")
                .push("reduce_start".to_owned());
        }

        fn on_reduce_response_end(&self, _output: &str) {
            self.events
                .lock()
                .expect("events")
                .push("reduce_end".to_owned());
        }

        fn on_llm_new_token(&self, token: &str) {
            self.events
                .lock()
                .expect("events")
                .push(format!("token:{token}"));
        }
    }

    fn output(batch_index: usize, points: Vec<(&str, i64)>) -> MapSearchResult {
        MapSearchResult {
            batch_index,
            raw_response: "{}".to_owned(),
            points: points
                .into_iter()
                .map(|(answer, score)| MapPoint {
                    answer: answer.to_owned(),
                    score,
                })
                .collect(),
            context: String::new(),
            usage: QueryUsageCategory::default(),
        }
    }

    #[test]
    fn test_should_format_filter_and_stably_sort_reduce_points() {
        let outputs = [
            output(0, vec![("first tie", 5), ("zero", 0), ("negative", -1)]),
            output(1, vec![("second tie", 5), ("best", 9)]),
        ];
        let reduce = build_reduce_context(&outputs, 100, &WordTokenizer, true, true)
            .expect("reduce context");
        assert!(reduce.has_positive_points);
        assert_eq!(
            reduce.text,
            "----Analyst 2----\nImportance Score: 9\nbest\n\n----Analyst 1----\nImportance Score: \
             5\nfirst tie\n\n----Analyst 2----\nImportance Score: 5\nsecond tie"
        );
    }

    #[test]
    fn test_should_stop_before_point_that_crosses_token_boundary() {
        let outputs = [output(0, vec![("one", 2), ("two", 1)])];
        let first = "----Analyst 1----\nImportance Score: 2\none";
        let tokenizer = WordTokenizer;
        let first_tokens = tokenizer.count(first).expect("tokens");
        let reduce = build_reduce_context(&outputs, first_tokens, &tokenizer, true, true)
            .expect("reduce context");
        assert_eq!(reduce.text, first);
    }

    #[test]
    fn test_should_capture_reduce_decisions_in_the_real_break_loop() {
        let outputs = [
            output(0, vec![("score nine", 9), ("score zero", 0)]),
            output(1, vec![("score eight", 8), ("score seven", 7)]),
        ];
        let tokenizer = WordTokenizer;
        let first = "----Analyst 1----\nImportance Score: 9\nscore nine";
        let second = "----Analyst 2----\nImportance Score: 8\nscore eight";
        let budget = tokenizer.count(first).expect("first tokens")
            + tokenizer.count(second).expect("second tokens");

        let reduce = build_reduce_context(&outputs, budget, &tokenizer, true, true)
            .expect("reduce decisions");

        assert_eq!(reduce.candidate_count, 4);
        assert_eq!(reduce.positive_count, 3);
        assert_eq!(reduce.selected_count, 2);
        assert!(reduce.truncated);
        assert_eq!(reduce.text, format!("{first}\n\n{second}"));
        assert!(reduce.decisions.iter().any(|decision| {
            decision.score == 9
                && decision.selected
                && decision.reason == GlobalMapPointDecisionReason::Selected
        }));
        assert!(reduce.decisions.iter().any(|decision| {
            decision.score == 8
                && decision.selected
                && decision.reason == GlobalMapPointDecisionReason::Selected
        }));
        assert!(reduce.decisions.iter().any(|decision| {
            decision.score == 7
                && !decision.selected
                && decision.reason == GlobalMapPointDecisionReason::TokenBudget
        }));
        assert!(reduce.decisions.iter().any(|decision| {
            decision.score == 0
                && !decision.selected
                && decision.reason == GlobalMapPointDecisionReason::NonPositiveScore
        }));
    }

    #[test]
    fn test_should_preserve_batch_metadata_when_map_points_are_empty() {
        let output = MapSearchResult {
            batch_index: 3,
            raw_response: r#"{"points":[{"invalid":true}]}"#.to_owned(),
            points: Vec::new(),
            context: "batch context".to_owned(),
            usage: QueryUsageCategory {
                llm_calls: 1,
                prompt_tokens: 7,
                output_tokens: 3,
            },
        };
        let frame = map_outputs_frame(&[output]).expect("map output records");
        assert_eq!(frame.height(), 1);
        assert_eq!(
            frame
                .column("raw_response")
                .expect("raw response")
                .str()
                .expect("string")
                .get(0),
            Some(r#"{"points":[{"invalid":true}]}"#)
        );
        assert_eq!(
            frame
                .column("context")
                .expect("context")
                .str()
                .expect("string")
                .get(0),
            Some("batch context")
        );
    }

    #[tokio::test]
    async fn test_should_bound_map_concurrency_restore_order_and_force_json_request() {
        let model = Arc::new(ConcurrentRecordingModel::default());
        let built = GlobalContextResult {
            batches: (0..4).map(|index| format!("batch-{index}")).collect(),
            records: Vec::new(),
            batch_report_ids: Vec::new(),
            usage: QueryUsageCategory::default(),
            dynamic_ratings: Vec::new(),
        };
        let prompt = PromptRepository::new(".")
            .load(PromptKind::GlobalSearchMap, None)
            .await
            .expect("map prompt");
        let config: ModelConfig = serde_json::from_value(serde_json::json!({
            "model_provider": "mock",
            "model": "recording",
            "call_args": {
                "stream": true,
                "response_format": {"type": "text"},
                "temperature": 0.2
            }
        }))
        .expect("model config");
        let outputs = run_map_calls(
            &built,
            "question",
            model.clone(),
            "recording",
            &config,
            &prompt,
            Arc::new(WordTokenizer),
            2,
            100,
            100,
            None,
        )
        .await
        .expect("map calls");
        assert_eq!(model.max_in_flight.load(Ordering::SeqCst), 2);
        assert_eq!(
            outputs
                .iter()
                .map(|output| output.batch_index)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        assert_eq!(
            outputs
                .iter()
                .map(|output| output.points[0].answer.as_str())
                .collect::<Vec<_>>(),
            vec!["answer-0", "answer-1", "answer-2", "answer-3"]
        );
        let requests = model.requests.lock().expect("requests");
        assert_eq!(requests.len(), 4);
        assert!(requests.iter().all(|request| request.stream == Some(false)));
        assert!(requests.iter().all(|request| {
            request.response_format == Some(serde_json::json!({"type": "json_object"}))
        }));
    }

    #[tokio::test]
    async fn test_should_preserve_special_report_csv_in_map_requests() {
        let golden = serde_json::from_str::<serde_json::Value>(include_str!(
            "../../../../../tests/compat/fixtures/query/report_csv_special_characters.json"
        ))
        .expect("report CSV golden");
        let batches = golden["global_batches"]
            .as_array()
            .expect("Global batches")
            .iter()
            .map(|batch| batch.as_str().expect("Global batch").to_owned())
            .collect::<Vec<_>>();
        let built = GlobalContextResult {
            batches: batches.clone(),
            records: Vec::new(),
            batch_report_ids: Vec::new(),
            usage: QueryUsageCategory::default(),
            dynamic_ratings: Vec::new(),
        };
        let prompt = PromptRepository::new(".")
            .load(PromptKind::GlobalSearchMap, None)
            .await
            .expect("map prompt");
        let config: ModelConfig = serde_json::from_value(serde_json::json!({
            "model_provider": "mock",
            "model": "request-recording"
        }))
        .expect("model config");
        let model = Arc::new(RequestRecordingModel::default());

        let outputs = run_map_calls(
            &built,
            "question",
            model.clone(),
            "request-recording",
            &config,
            &prompt,
            Arc::new(WordTokenizer),
            2,
            100,
            100,
            None,
        )
        .await
        .expect("special report map calls");

        assert_eq!(
            outputs
                .iter()
                .map(|output| output.context.as_str())
                .collect::<Vec<_>>(),
            batches.iter().map(String::as_str).collect::<Vec<_>>()
        );
        let requests = model.requests.lock().expect("map requests");
        assert_eq!(requests.len(), batches.len());
        for batch in &batches {
            assert!(requests.iter().any(|request| {
                request
                    .messages
                    .first()
                    .is_some_and(|message| message.content.contains(batch))
            }));
        }
    }

    #[tokio::test]
    async fn test_should_propagate_map_provider_errors_instead_of_score_zero_fallback() {
        let built = GlobalContextResult {
            batches: vec!["batch-0".to_owned()],
            records: Vec::new(),
            batch_report_ids: Vec::new(),
            usage: QueryUsageCategory::default(),
            dynamic_ratings: Vec::new(),
        };
        let prompt = PromptRepository::new(".")
            .load(PromptKind::GlobalSearchMap, None)
            .await
            .expect("map prompt");
        let config: ModelConfig = serde_json::from_value(serde_json::json!({
            "model_provider": "mock",
            "model": "failing"
        }))
        .expect("model config");
        let error = run_map_calls(
            &built,
            "question",
            Arc::new(FailingModel),
            "failing",
            &config,
            &prompt,
            Arc::new(WordTokenizer),
            1,
            100,
            100,
            None,
        )
        .await
        .expect_err("provider error");
        assert!(matches!(
            error,
            crate::query::QueryError::QueryCompletion {
                operation: "complete Global Search map call",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn test_should_not_emit_reduce_end_after_midstream_provider_error() {
        let callbacks = Arc::new(ReduceRecordingCallback::default());
        callbacks.on_reduce_response_start("reduce context");
        let provider = Box::pin(futures_util::stream::iter(vec![
            Ok(CompletionChunk::text_for_test("test", "partial", None)),
            Err(LlmError::Timeout {
                model_instance: "test".to_owned(),
                operation: "stream",
                attempts: 1,
            }),
        ]));
        let state = CompletionStreamState {
            provider,
            context: QueryContext::default(),
            started: std::time::Instant::now(),
            categories: BTreeMap::new(),
            completion_category: "reduce",
            prompt_tokens: 0,
            tokenizer: Arc::new(WordTokenizer),
            callbacks: callbacks.clone(),
            completion_model_id: "test".to_owned(),
            method: crate::query::SearchMethod::Global,
            consume_operation: "consume Global Search reduce stream",
            output_count_operation: "count Global reduce output tokens",
            output_count_is_context_error: true,
            notify_reduce_end: true,
        };
        let mut events = completion_event_stream(state);
        assert!(matches!(
            events.next().await.expect("context event"),
            Ok(QueryEvent::Context(_))
        ));
        let event = events.next().await.expect("token event");
        assert!(matches!(event, Ok(QueryEvent::Token(ref token)) if token == "partial"));
        let event = events.next().await.expect("error event");
        assert!(matches!(
            event,
            Err(crate::query::QueryError::QueryCompletion {
                operation: "consume Global Search reduce stream",
                ..
            })
        ));
        assert!(events.next().await.is_none());
        assert_eq!(
            *callbacks.events.lock().expect("events"),
            ["reduce_start", "token:partial"]
        );
    }

    #[tokio::test]
    async fn test_should_fail_global_run_when_wrapped_stream_ends_without_completed() {
        let sink = Arc::new(RecordingSink::default());
        let options = QueryOptions::new(
            std::path::PathBuf::from("."),
            "question".to_owned(),
            SearchMethod::Global,
        )
        .with_explainability(QueryExplainabilityOptions::new(
            "global-stream-ended".parse().expect("run id"),
            ExplainabilityContentMode::Metadata,
            sink.clone(),
        ));
        let instrumentation = QueryInstrumentation::start(&options, true)
            .await
            .expect("Global instrumentation");
        let empty: crate::query::QueryEventStream = Box::pin(futures_util::stream::empty());
        let mut events = instrument_global_completion_stream(empty, Some(instrumentation), None);

        assert!(events.next().await.is_none());
        let records = sink.records.lock().expect("records");
        assert!(matches!(
            records.last().map(|record| &record.event),
            Some(ExplainabilityEvent::RunFailed(event))
                if event.error_kind == "query_completion"
        ));
        assert_eq!(sink.finishes.load(Ordering::SeqCst), 1);
    }
}
