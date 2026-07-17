//! Global map/reduce request construction and shared streaming orchestration.

use std::{cmp::Ordering, collections::BTreeMap, sync::Arc, time::Instant};

use futures_util::{StreamExt, stream};
use graphloom_llm::{
    ChatMessage, CompletionModel, CompletionRequest, CompletionStream, ModelConfig, Tokenizer,
};
use polars_core::prelude::DataFrame;
use serde::Serialize;
use serde_json::json;

use super::{
    context::{GlobalContextResult, global_context},
    dynamic::DynamicCommunitySelection,
    parse::{MapSearchResult, parse_map_points},
};
use crate::{
    prompts::PromptTemplate,
    query::{
        GlobalQueryRuntime, QueryCallbacks, QueryContext, QueryError, QueryEvent, QueryEventStream,
        QueryResult, QueryUsage, QueryUsageCategory, Result, SearchMethod, context::ContextTable,
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

struct GlobalStreamState {
    provider: CompletionStream,
    context: QueryContext,
    response: String,
    started: Instant,
    usage: BTreeMap<String, QueryUsageCategory>,
    reduce_prompt_tokens: usize,
    tokenizer: Arc<dyn Tokenizer>,
    callbacks: Arc<dyn QueryCallbacks>,
    completion_model_id: String,
    phase: GlobalStreamPhase,
}

#[derive(Debug, Clone, Copy)]
enum GlobalStreamPhase {
    Context,
    Tokens,
}

pub(crate) async fn global_search(
    runtime: GlobalQueryRuntime,
    query: &str,
    response_type: &str,
) -> Result<QueryResult> {
    let mut events = global_search_streaming(runtime, query, response_type).await?;
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
) -> Result<QueryEventStream> {
    let started = Instant::now();
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
        runtime.global_context.build_fixed()?
    };
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
    )
    .await?;
    runtime.callbacks.on_map_response_end(&map_outputs);

    let (report_data, has_positive_points) = build_reduce_context(
        &map_outputs,
        runtime.global_context.config().data_max_tokens,
        runtime.global_context.tokenizer.as_ref(),
    )?;
    let context = global_context(
        &built,
        report_data.clone(),
        map_outputs_frame(&map_outputs)?,
    )?;
    runtime.callbacks.on_context(&context);
    let map_usage = sum_map_usage(&map_outputs);
    let build_usage = built.usage;

    if !has_positive_points {
        return Ok(no_data_stream(context, started, build_usage, map_usage));
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
    let reduce_prompt_tokens = count(
        runtime.global_context.tokenizer.as_ref(),
        &rendered,
        "count Global reduce prompt tokens",
    )?;
    let mut request = CompletionRequest::new(vec![
        ChatMessage::system(rendered),
        ChatMessage::user(query),
    ]);
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
    let state = GlobalStreamState {
        provider,
        context,
        response: String::new(),
        started,
        usage: BTreeMap::from([
            ("build_context".to_owned(), build_usage),
            ("map".to_owned(), map_usage),
        ]),
        reduce_prompt_tokens,
        tokenizer: runtime.global_context.tokenizer,
        callbacks: runtime.callbacks,
        completion_model_id: runtime.completion_model_id,
        phase: GlobalStreamPhase::Context,
    };
    Ok(Box::pin(stream::unfold(Some(state), |state| async move {
        next_event(state).await
    })))
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
) -> Result<Vec<MapSearchResult>> {
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
            async move {
                run_map_call(
                    index, context, &query, model, &model_id, &call_args, &prompt, tokenizer,
                    max_length,
                )
                .await
            }
        });
    stream::iter(futures)
        .buffered(concurrent_requests)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect()
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
) -> Result<MapSearchResult> {
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
    let prompt_tokens = count(
        tokenizer.as_ref(),
        &rendered,
        "count Global map prompt tokens",
    )?;
    let mut request = CompletionRequest::new(vec![
        ChatMessage::system(rendered),
        ChatMessage::user(query),
    ]);
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
    Ok(MapSearchResult {
        batch_index,
        points: parse_map_points(&raw_response),
        raw_response,
        context,
        usage: QueryUsageCategory {
            llm_calls: 1,
            prompt_tokens,
            output_tokens,
        },
    })
}

fn build_reduce_context(
    outputs: &[MapSearchResult],
    max_tokens: usize,
    tokenizer: &dyn Tokenizer,
) -> Result<(String, bool)> {
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
    let mut selected = Vec::new();
    let mut tokens = 0_usize;
    for (batch_index, _, point) in points {
        let text = format!(
            "----Analyst {}----\nImportance Score: {}\n{}",
            batch_index + 1,
            point.score,
            point.answer
        );
        let point_tokens = count(tokenizer, &text, "count Global reduce point tokens")?;
        if tokens.saturating_add(point_tokens) > max_tokens {
            break;
        }
        tokens = tokens.saturating_add(point_tokens);
        selected.push(text);
    }
    Ok((selected.join("\n\n"), has_positive_points))
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
            total.llm_calls += output.usage.llm_calls;
            total.prompt_tokens += output.usage.prompt_tokens;
            total.output_tokens += output.usage.output_tokens;
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

async fn next_event(
    state: Option<GlobalStreamState>,
) -> Option<(Result<QueryEvent>, Option<GlobalStreamState>)> {
    let mut state = state?;
    match state.phase {
        GlobalStreamPhase::Context => {
            state.phase = GlobalStreamPhase::Tokens;
            Some((Ok(QueryEvent::Context(state.context.clone())), Some(state)))
        }
        GlobalStreamPhase::Tokens => loop {
            match state.provider.next().await {
                Some(Ok(chunk)) => {
                    let content = chunk
                        .choices
                        .first()
                        .and_then(|choice| choice.delta.content.as_deref())
                        .unwrap_or_default();
                    if content.is_empty() {
                        continue;
                    }
                    state.response.push_str(content);
                    state.callbacks.on_llm_new_token(content);
                    return Some((Ok(QueryEvent::Token(content.to_owned())), Some(state)));
                }
                Some(Err(source)) => {
                    return Some((
                        Err(QueryError::QueryCompletion {
                            method: SearchMethod::Global,
                            operation: "consume Global Search reduce stream",
                            model: state.completion_model_id.clone(),
                            source: Box::new(source),
                        }),
                        None,
                    ));
                }
                None => return Some(completed_event(state)),
            }
        },
    }
}

fn completed_event(
    mut state: GlobalStreamState,
) -> (Result<QueryEvent>, Option<GlobalStreamState>) {
    let output_tokens = match count(
        state.tokenizer.as_ref(),
        &state.response,
        "count Global reduce output tokens",
    ) {
        Ok(value) => value,
        Err(error) => return (Err(error), None),
    };
    state.callbacks.on_reduce_response_end(&state.response);
    state.usage.insert(
        "reduce".to_owned(),
        QueryUsageCategory {
            llm_calls: 1,
            prompt_tokens: state.reduce_prompt_tokens,
            output_tokens,
        },
    );
    let result = QueryResult {
        response: state.response,
        context: state.context,
        elapsed: state.started.elapsed(),
        usage: QueryUsage::from_categories(state.usage),
    };
    (Ok(QueryEvent::Completed(result)), None)
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
    use graphloom_llm::{
        CompletionChunk, CompletionModel, CompletionRequest, CompletionResponse, LlmError,
        ModelConfig, Tokenizer,
    };

    use super::{
        GlobalStreamPhase, GlobalStreamState, build_reduce_context, map_outputs_frame, next_event,
        run_map_calls,
    };
    use crate::{
        prompts::{PromptKind, PromptRepository},
        query::{
            MapPoint, MapSearchResult, QueryCallbacks, QueryContext, QueryEvent,
            QueryUsageCategory, global::context::GlobalContextResult,
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
        let (context, positive) =
            build_reduce_context(&outputs, 100, &WordTokenizer).expect("reduce context");
        assert!(positive);
        assert_eq!(
            context,
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
        let (context, _) =
            build_reduce_context(&outputs, first_tokens, &tokenizer).expect("reduce context");
        assert_eq!(context, first);
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
        let state = GlobalStreamState {
            provider,
            context: QueryContext::default(),
            response: String::new(),
            started: std::time::Instant::now(),
            usage: BTreeMap::new(),
            reduce_prompt_tokens: 0,
            tokenizer: Arc::new(WordTokenizer),
            callbacks: callbacks.clone(),
            completion_model_id: "test".to_owned(),
            phase: GlobalStreamPhase::Tokens,
        };
        let (event, state) = next_event(Some(state)).await.expect("token event");
        assert!(matches!(event, Ok(QueryEvent::Token(ref token)) if token == "partial"));
        let (event, state) = next_event(state).await.expect("error event");
        assert!(matches!(
            event,
            Err(crate::query::QueryError::QueryCompletion {
                operation: "consume Global Search reduce stream",
                ..
            })
        ));
        assert!(state.is_none());
        assert_eq!(
            *callbacks.events.lock().expect("events"),
            ["reduce_start", "token:partial"]
        );
    }
}
