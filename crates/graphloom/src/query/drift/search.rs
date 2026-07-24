//! DRIFT orchestration, recursive local actions, and final reduce.

use std::{collections::BTreeMap, sync::Arc, time::Instant};

use futures_util::{StreamExt, stream};
use graphloom_llm::{ChatMessage, CompletionModel, CompletionRequest, CompletionStream, Tokenizer};
use serde::Serialize;

use super::{
    action::{DriftActionMetadata, DriftActionResponse},
    context::{DriftRandom, RankedReport, SystemDriftRandom, count},
    parse::parse_action,
    primer::{PrimerAggregate, PrimerResources, run_primer},
    state::DriftQueryState,
};
use crate::query::{
    DriftQueryRuntime, QueryCallbacks, QueryContext, QueryContextRecords, QueryContextText,
    QueryError, QueryEvent, QueryEventStream, QueryResult, QueryUsage, QueryUsageCategory, Result,
    SearchMethod, context::ContextTable, result::count_completion_input,
};

#[derive(Debug)]
struct DriftPrepared {
    context: QueryContext,
    state_context: String,
    reduce_context: String,
    usage: BTreeMap<String, QueryUsageCategory>,
}

#[derive(Debug, Serialize)]
struct DriftLocalPrompt<'a> {
    context_data: &'a str,
    response_type: &'a str,
    global_query: &'a str,
    followups: usize,
}

#[derive(Debug, Serialize)]
struct DriftReducePrompt<'a> {
    context_data: &'a str,
    response_type: &'a str,
}

pub(crate) async fn drift_search(
    runtime: DriftQueryRuntime,
    query: &str,
    response_type: &str,
) -> Result<QueryResult> {
    validate_query(query)?;
    let started = Instant::now();
    let mut random = SystemDriftRandom;
    let prepared = prepare(&runtime, query, &mut random).await?;
    let rendered = render_reduce(&runtime, &prepared.reduce_context, response_type)?;
    let mut request = CompletionRequest::new(vec![
        ChatMessage::system(rendered),
        ChatMessage::user(query),
    ]);
    let prompt_tokens = count_completion_input(
        runtime.context.tokenizer.as_ref(),
        &request.messages,
        SearchMethod::Drift,
        "count DRIFT reduce completion input tokens",
    )?;
    apply_reduce_request(&runtime, &mut request, false)?;
    runtime
        .callbacks
        .on_reduce_response_start(&prepared.state_context);
    let response = runtime
        .context
        .completion_model
        .complete(request)
        .await
        .map_err(|source| completion_error(&runtime, "complete DRIFT reduce response", source))?;
    let answer = response
        .content()
        .map_err(|source| completion_error(&runtime, "read DRIFT reduce response", source))?
        .to_owned();
    runtime.callbacks.on_reduce_response_end(&answer);
    let output_tokens = count(
        &*runtime.context.tokenizer,
        &answer,
        "count DRIFT reduce output",
    )?;
    let mut categories = prepared.usage;
    categories.insert(
        "reduce".to_owned(),
        QueryUsageCategory {
            llm_calls: 1,
            prompt_tokens,
            output_tokens,
        },
    );
    Ok(QueryResult {
        response: answer,
        context: prepared.context,
        elapsed: started.elapsed(),
        usage: QueryUsage::from_categories(categories),
    })
}

pub(crate) async fn drift_search_streaming(
    runtime: DriftQueryRuntime,
    query: &str,
    response_type: &str,
) -> Result<QueryEventStream> {
    validate_query(query)?;
    let started = Instant::now();
    let mut random = SystemDriftRandom;
    let prepared = prepare(&runtime, query, &mut random).await?;
    let rendered = render_reduce(&runtime, &prepared.reduce_context, response_type)?;
    let mut request = CompletionRequest::new(vec![
        ChatMessage::system(rendered),
        ChatMessage::user(query),
    ]);
    let prompt_tokens = count_completion_input(
        runtime.context.tokenizer.as_ref(),
        &request.messages,
        SearchMethod::Drift,
        "count DRIFT reduce completion input tokens",
    )?;
    apply_reduce_request(&runtime, &mut request, true)?;
    let state = DriftStreamState {
        model: Arc::clone(&runtime.context.completion_model),
        model_id: runtime.context.completion_model_id.clone(),
        request: Some(request),
        provider: None,
        context: prepared.context,
        state_context: prepared.state_context,
        response: String::new(),
        started,
        usage: prepared.usage,
        prompt_tokens,
        tokenizer: Arc::clone(&runtime.context.tokenizer),
        callbacks: runtime.callbacks,
        phase: DriftStreamPhase::Context,
    };
    Ok(Box::pin(stream::unfold(Some(state), next_stream_event)))
}

async fn prepare(
    runtime: &DriftQueryRuntime,
    query: &str,
    random: &mut dyn DriftRandom,
) -> Result<DriftPrepared> {
    let (ranked, build_usage) = runtime.context.build_ranked_context(query, random).await?;
    let primer = run_primer(
        &ranked,
        query,
        runtime.context.config.effective_primer_folds(),
        PrimerResources {
            concurrency: runtime.context.config.concurrency,
            model: Arc::clone(&runtime.context.completion_model),
            model_id: &runtime.context.completion_model_id,
            model_config: &runtime.context.completion_config,
            tokenizer: Arc::clone(&runtime.context.tokenizer),
        },
    )
    .await?;
    let mut state = DriftQueryState::default();
    state.add_root(
        query.to_owned(),
        primer.answer.clone(),
        primer.score,
        &primer.followups,
    );
    let action_usage = run_depths(runtime, query, random, &mut state).await?;
    let state_context = state.to_json()?;
    let reduce_context = python_list_repr(&state.reduce_answers());
    let context = build_query_context(&ranked, &primer, &state, &state_context, &reduce_context)?;
    Ok(DriftPrepared {
        context,
        state_context,
        reduce_context,
        usage: BTreeMap::from([
            ("build_context".to_owned(), build_usage),
            ("primer".to_owned(), primer.usage),
            ("action".to_owned(), action_usage),
        ]),
    })
}

async fn run_depths(
    runtime: &DriftQueryRuntime,
    original_query: &str,
    random: &mut dyn DriftRandom,
    state: &mut DriftQueryState,
) -> Result<QueryUsageCategory> {
    let mut total = QueryUsageCategory::default();
    for _ in 0..runtime.context.config.n_depth {
        let selected = select_actions(state, random, runtime.context.config.drift_k_followups)?;
        if selected.is_empty() {
            break;
        }
        let queries = selected
            .iter()
            .map(|id| {
                state
                    .query(*id)
                    .map(str::to_owned)
                    .ok_or_else(|| QueryError::QueryContext {
                        method: SearchMethod::Drift,
                        operation: "select DRIFT incomplete actions",
                        message: format!("action id {id} is absent"),
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        let calls = queries
            .into_iter()
            .map(|query| async move { run_action(runtime, original_query, query).await });
        let results = crate::query::concurrency::try_buffered_ordered(
            calls,
            runtime.context.config.concurrency,
        )
        .await?;
        for (id, (response, metadata)) in selected.into_iter().zip(results) {
            total += metadata.usage;
            state.apply(id, response, metadata)?;
        }
    }
    Ok(total)
}

fn select_actions(
    state: &DriftQueryState,
    random: &mut dyn DriftRandom,
    limit: usize,
) -> Result<Vec<usize>> {
    let mut selected = state.incomplete_ids();
    random.shuffle_actions(&mut selected)?;
    selected.truncate(limit);
    Ok(selected)
}

async fn run_action(
    runtime: &DriftQueryRuntime,
    original_query: &str,
    query: String,
) -> Result<(DriftActionResponse, DriftActionMetadata)> {
    let built = runtime.context.local.build(&query, None).await?;
    let context_text = match &built.context.text {
        QueryContextText::Text(value) => value,
        _ => {
            return Err(QueryError::QueryContext {
                method: SearchMethod::Drift,
                operation: "read DRIFT Local context text",
                message: "DRIFT Local context must be a single string".to_owned(),
            });
        }
    };
    let rendered = runtime
        .local_prompt
        .bind(&DriftLocalPrompt {
            context_data: context_text,
            response_type: "multiple paragraphs",
            global_query: original_query,
            followups: runtime.context.config.drift_k_followups,
        })
        .and_then(|prompt| prompt.render())
        .map_err(|source| QueryError::QueryPrompt {
            method: SearchMethod::Drift,
            operation: "render DRIFT Local prompt",
            prompt: "drift_search_system_prompt.txt",
            source: Box::new(source),
        })?;
    let mut request = CompletionRequest::new(vec![
        ChatMessage::system(rendered),
        ChatMessage::user(&query),
    ]);
    let prompt_tokens = count_completion_input(
        runtime.context.tokenizer.as_ref(),
        &request.messages,
        SearchMethod::Drift,
        "count DRIFT Local completion input tokens",
    )?;
    request
        .apply_call_args(&runtime.context.completion_config.call_args)
        .and_then(|()| {
            request.temperature = Some(runtime.context.config.local_search_temperature);
            request.top_p = Some(runtime.context.config.local_search_top_p);
            request.n = Some(
                u32::try_from(runtime.context.config.local_search_n).map_err(|_| {
                    graphloom_llm::LlmError::InvalidRequest {
                        operation: "build DRIFT Local request",
                        message: "local_search_n exceeds u32".to_owned(),
                    }
                })?,
            );
            request.max_completion_tokens = runtime
                .context
                .config
                .local_search_llm_max_gen_completion_tokens;
            request.response_format = None;
            request.stream = Some(true);
            request.validate()
        })
        .map_err(|source| QueryError::InvalidQueryConfig {
            method: SearchMethod::Drift,
            operation: "build DRIFT Local completion request",
            message: source.to_string(),
        })?;
    let mut provider = runtime
        .context
        .completion_model
        .stream(request)
        .await
        .map_err(|source| completion_error(runtime, "start DRIFT Local completion", source))?;
    let mut raw = String::new();
    while let Some(chunk) = provider.next().await {
        let chunk = chunk.map_err(|source| {
            completion_error(runtime, "consume DRIFT Local completion", source)
        })?;
        let text = chunk
            .choices
            .first()
            .and_then(|choice| choice.delta.content.as_deref())
            .unwrap_or_default();
        if !text.is_empty() {
            raw.push_str(text);
            runtime.callbacks.on_llm_new_token(text);
        }
    }
    // GraphRAG's LocalSearch callback publishes the completed context after
    // consuming the intermediate response stream.
    runtime.callbacks.on_context(&built.context);
    let output_tokens = count(
        &*runtime.context.tokenizer,
        &raw,
        "count DRIFT Local output",
    )?;
    let mut usage = built.usage;
    usage += QueryUsageCategory {
        llm_calls: 1,
        prompt_tokens,
        output_tokens,
    };
    Ok((
        parse_action(&raw)?,
        DriftActionMetadata {
            usage,
            context: Some(built.context),
        },
    ))
}

fn build_query_context(
    reports: &[RankedReport],
    primer: &PrimerAggregate,
    state: &DriftQueryState,
    state_context: &str,
    reduce_context: &str,
) -> Result<QueryContext> {
    let primer_rows = reports
        .iter()
        .map(|report| {
            vec![
                report.short_id.clone(),
                report.community_id.clone(),
                report.full_content.clone(),
            ]
        })
        .collect::<Vec<_>>();
    let primer_table = ContextTable::new(["short_id", "community_id", "full_content"], primer_rows);
    let ranking_table = ContextTable::new(
        ["short_id", "similarity"],
        reports
            .iter()
            .map(|report| vec![report.short_id.clone(), report.similarity.to_string()])
            .collect(),
    );
    let primer_text =
        primer_table.render_csv(SearchMethod::Drift, "render DRIFT primer context")?;
    let mut action_text = BTreeMap::new();
    let mut action_records = BTreeMap::new();
    for action in state.nodes() {
        if let Some(context) = &action.metadata.context {
            action_text.insert(action.query.clone(), context.text.clone());
            action_records.insert(action.query.clone(), context.records.clone());
        }
    }
    let node_table = ContextTable::new(
        ["id", "query", "answer", "score"],
        state
            .nodes()
            .iter()
            .map(|node| {
                vec![
                    node.id.to_string(),
                    node.query.clone(),
                    node.answer.clone().unwrap_or_default(),
                    if node.score.is_finite() {
                        node.score.to_string()
                    } else {
                        String::new()
                    },
                ]
            })
            .collect(),
    )
    .to_dataframe(SearchMethod::Drift, "build DRIFT node records")?;
    let edge_table = ContextTable::new(
        ["source", "target", "weight"],
        state
            .edges_in_graph_order()
            .iter()
            .map(|edge| {
                vec![
                    edge.source.to_string(),
                    edge.target.to_string(),
                    edge.weight.to_string(),
                ]
            })
            .collect(),
    )
    .to_dataframe(SearchMethod::Drift, "build DRIFT edge records")?;
    Ok(QueryContext {
        text: QueryContextText::Composite(BTreeMap::from([
            ("primer".to_owned(), QueryContextText::Text(primer_text)),
            (
                "state".to_owned(),
                QueryContextText::Text(state_context.to_owned()),
            ),
            (
                "actions".to_owned(),
                QueryContextText::Composite(action_text),
            ),
            (
                "reduce".to_owned(),
                QueryContextText::Text(reduce_context.to_owned()),
            ),
        ])),
        records: QueryContextRecords::Named(BTreeMap::from([
            (
                "primer".to_owned(),
                QueryContextRecords::Tables(BTreeMap::from([
                    (
                        "top_k_reports".to_owned(),
                        primer_table
                            .to_dataframe(SearchMethod::Drift, "build DRIFT primer records")?,
                    ),
                    (
                        "ranking".to_owned(),
                        ranking_table
                            .to_dataframe(SearchMethod::Drift, "build DRIFT ranking records")?,
                    ),
                ])),
            ),
            (
                "state".to_owned(),
                QueryContextRecords::Tables(BTreeMap::from([
                    ("nodes".to_owned(), node_table),
                    ("edges".to_owned(), edge_table),
                ])),
            ),
            (
                "actions".to_owned(),
                QueryContextRecords::Named(action_records),
            ),
            (
                "primer_response".to_owned(),
                QueryContextRecords::Tables(BTreeMap::from([(
                    "aggregate".to_owned(),
                    ContextTable::new(
                        ["answer", "score", "follow_up_queries"],
                        vec![vec![
                            primer.answer.clone(),
                            primer.score.to_string(),
                            serde_json::to_string(&primer.followups).map_err(|source| {
                                QueryError::QueryParse {
                                    method: SearchMethod::Drift,
                                    operation: "serialize DRIFT primer follow-ups",
                                    message: source.to_string(),
                                }
                            })?,
                        ]],
                    )
                    .to_dataframe(SearchMethod::Drift, "build DRIFT primer aggregate")?,
                )])),
            ),
        ])),
    })
}

fn python_list_repr(answers: &[&str]) -> String {
    format!(
        "[{}]",
        answers
            .iter()
            .map(|answer| python_string_repr(answer))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn python_string_repr(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut result = String::from(quote);
    for character in value.chars() {
        match character {
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            value if value == quote => {
                result.push('\\');
                result.push(value);
            }
            value if value.is_control() => {
                use std::fmt::Write;
                let _ = write!(result, "\\x{:02x}", u32::from(value));
            }
            value => result.push(value),
        }
    }
    result.push(quote);
    result
}

fn render_reduce(
    runtime: &DriftQueryRuntime,
    context: &str,
    response_type: &str,
) -> Result<String> {
    runtime
        .reduce_prompt
        .bind(&DriftReducePrompt {
            context_data: context,
            response_type,
        })
        .and_then(|prompt| prompt.render())
        .map_err(|source| QueryError::QueryPrompt {
            method: SearchMethod::Drift,
            operation: "render DRIFT reduce prompt",
            prompt: "drift_reduce_prompt.txt",
            source: Box::new(source),
        })
}

fn apply_reduce_request(
    runtime: &DriftQueryRuntime,
    request: &mut CompletionRequest,
    stream: bool,
) -> Result<()> {
    request
        .apply_call_args(&runtime.context.completion_config.call_args)
        .and_then(|()| {
            request.temperature = Some(runtime.context.config.reduce_temperature);
            request.max_completion_tokens = runtime.context.config.reduce_max_completion_tokens;
            request.stream = Some(stream);
            request.response_format = None;
            request.validate()
        })
        .map_err(|source| QueryError::InvalidQueryConfig {
            method: SearchMethod::Drift,
            operation: "build DRIFT reduce completion request",
            message: source.to_string(),
        })
}

fn validate_query(query: &str) -> Result<()> {
    if query.is_empty() {
        Err(QueryError::InvalidQueryConfig {
            method: SearchMethod::Drift,
            operation: "validate DRIFT Search query",
            message: "DRIFT Search query cannot be empty".to_owned(),
        })
    } else {
        Ok(())
    }
}

fn completion_error(
    runtime: &DriftQueryRuntime,
    operation: &'static str,
    source: graphloom_llm::LlmError,
) -> QueryError {
    QueryError::QueryCompletion {
        method: SearchMethod::Drift,
        operation,
        model: runtime.context.completion_model_id.clone(),
        source: Box::new(source),
    }
}

#[derive(Debug, Clone, Copy)]
enum DriftStreamPhase {
    Context,
    Start,
    Tokens,
}

struct DriftStreamState {
    model: Arc<dyn CompletionModel>,
    model_id: String,
    request: Option<CompletionRequest>,
    provider: Option<CompletionStream>,
    context: QueryContext,
    state_context: String,
    response: String,
    started: Instant,
    usage: BTreeMap<String, QueryUsageCategory>,
    prompt_tokens: usize,
    tokenizer: Arc<dyn Tokenizer>,
    callbacks: Arc<dyn QueryCallbacks>,
    phase: DriftStreamPhase,
}

async fn next_stream_event(
    state: Option<DriftStreamState>,
) -> Option<(Result<QueryEvent>, Option<DriftStreamState>)> {
    let mut state = state?;
    loop {
        match state.phase {
            DriftStreamPhase::Context => {
                state.phase = DriftStreamPhase::Start;
                return Some((Ok(QueryEvent::Context(state.context.clone())), Some(state)));
            }
            DriftStreamPhase::Start => {
                state
                    .callbacks
                    .on_reduce_response_start(&state.state_context);
                let Some(request) = state.request.take() else {
                    return Some((Err(stream_error(&state, "missing reduce request")), None));
                };
                match state.model.stream(request).await {
                    Ok(provider) => {
                        state.provider = Some(provider);
                        state.phase = DriftStreamPhase::Tokens;
                    }
                    Err(source) => {
                        return Some((
                            Err(QueryError::QueryCompletion {
                                method: SearchMethod::Drift,
                                operation: "start DRIFT reduce stream",
                                model: state.model_id,
                                source: Box::new(source),
                            }),
                            None,
                        ));
                    }
                }
            }
            DriftStreamPhase::Tokens => loop {
                let Some(provider) = state.provider.as_mut() else {
                    return Some((Err(stream_error(&state, "missing reduce stream")), None));
                };
                match provider.next().await {
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
                                method: SearchMethod::Drift,
                                operation: "consume DRIFT reduce stream",
                                model: state.model_id,
                                source: Box::new(source),
                            }),
                            None,
                        ));
                    }
                    None => {
                        state.callbacks.on_reduce_response_end(&state.response);
                        let output_tokens = match count(
                            &*state.tokenizer,
                            &state.response,
                            "count DRIFT reduce output",
                        ) {
                            Ok(value) => value,
                            Err(error) => return Some((Err(error), None)),
                        };
                        state.usage.insert(
                            "reduce".to_owned(),
                            QueryUsageCategory {
                                llm_calls: 1,
                                prompt_tokens: state.prompt_tokens,
                                output_tokens,
                            },
                        );
                        let result = QueryResult {
                            response: state.response,
                            context: state.context,
                            elapsed: state.started.elapsed(),
                            usage: QueryUsage::from_categories(state.usage),
                        };
                        return Some((Ok(QueryEvent::Completed(result)), None));
                    }
                }
            },
        }
    }
}

fn stream_error(state: &DriftStreamState, message: &str) -> QueryError {
    QueryError::QueryCompletion {
        method: SearchMethod::Drift,
        operation: "advance DRIFT reduce stream",
        model: state.model_id.clone(),
        source: Box::new(graphloom_llm::LlmError::InvalidResponse {
            model_instance: state.model_id.clone(),
            operation: "query stream",
            message: message.to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::{DriftQueryState, python_list_repr, python_string_repr, select_actions};
    use crate::query::{
        QueryUsageCategory,
        drift::{
            action::{DriftActionMetadata, DriftActionResponse},
            context::{DriftRandom, ScriptedDriftRandom, SystemDriftRandom},
        },
    };

    #[derive(Debug, Deserialize)]
    struct ScriptedTrajectory {
        report_indices: Vec<usize>,
        action_orders: Vec<Vec<usize>>,
        expected_selected_queries: Vec<Vec<String>>,
        expected_node_queries: Vec<String>,
        expected_edges: serde_json::Value,
        expected_reduce_answers: Vec<String>,
    }

    fn scripted_trajectory() -> ScriptedTrajectory {
        serde_json::from_str(include_str!(
            "../../../../../tests/compat/fixtures/query/drift_random_trajectory.json"
        ))
        .expect("shared DRIFT random trajectory")
    }

    #[test]
    fn test_should_format_python_compatible_string_lists() {
        assert_eq!(
            python_list_repr(&["one", "it's \"quoted\"\\next\nline"]),
            r#"['one', 'it\'s "quoted"\\next\nline']"#
        );
        assert_eq!(python_string_repr("it's fine"), r#""it's fine""#);
    }

    #[test]
    fn test_should_use_injected_rng_for_stable_action_selection() {
        let mut state = DriftQueryState::default();
        state.add_root(
            "root".to_owned(),
            "answer".to_owned(),
            1.0,
            &["one".to_owned(), "two".to_owned(), "three".to_owned()],
        );
        let mut random = ScriptedDriftRandom::new([], [vec![2, 1, 0], vec![2, 1, 0]]);

        assert_eq!(
            select_actions(&state, &mut random, 2).expect("first scripted selection"),
            [3, 2]
        );
        assert_eq!(
            select_actions(&state, &mut random, 2).expect("second scripted selection"),
            [3, 2]
        );
    }

    #[test]
    fn test_should_limit_selection_and_only_return_incomplete_actions() {
        let mut state = DriftQueryState::default();
        state.add_root(
            "root".to_owned(),
            "answer".to_owned(),
            1.0,
            &["one".to_owned(), "two".to_owned(), "three".to_owned()],
        );
        let incomplete = state.incomplete_ids();
        let mut random = ScriptedDriftRandom::new([], [vec![1, 2, 0]]);

        let selected = select_actions(&state, &mut random, 2).expect("scripted selection");

        assert_eq!(selected.len(), incomplete.len().min(2));
        assert!(selected.iter().all(|id| incomplete.contains(id)));
    }

    #[test]
    fn test_should_consume_scripted_orders_across_depths() {
        let mut state = DriftQueryState::default();
        state.add_root(
            "root".to_owned(),
            "Primer".to_owned(),
            80.0,
            &["Q1".to_owned(), "Q2".to_owned(), "Q3".to_owned()],
        );
        let trajectory = scripted_trajectory();
        let mut random = ScriptedDriftRandom::new(
            trajectory.report_indices.clone(),
            trajectory.action_orders.clone(),
        );

        for expected in &trajectory.expected_selected_queries {
            let selected = select_actions(&state, &mut random, 2).expect("depth selection");
            let selected_queries = selected
                .iter()
                .filter_map(|id| state.query(*id))
                .map(str::to_owned)
                .collect::<Vec<_>>();
            assert_eq!(selected_queries.as_slice(), expected.as_slice());
            for id in selected {
                let query = state.query(id).expect("selected query").to_owned();
                let follow_up_queries = match query.as_str() {
                    "Q3" => vec!["C3".to_owned()],
                    "Q1" => vec!["C1".to_owned()],
                    _ => Vec::new(),
                };
                state
                    .apply(
                        id,
                        DriftActionResponse {
                            answer: Some(format!("answer-{query}")),
                            score: 90.0,
                            follow_up_queries,
                        },
                        DriftActionMetadata {
                            usage: QueryUsageCategory {
                                llm_calls: 1,
                                prompt_tokens: 4,
                                output_tokens: 5,
                            },
                            context: None,
                        },
                    )
                    .expect("apply scripted action");
            }
        }

        let state_json: serde_json::Value =
            serde_json::from_str(&state.to_json().expect("scripted state JSON"))
                .expect("valid scripted state JSON");
        let node_queries = state
            .nodes()
            .iter()
            .map(|node| node.query.clone())
            .collect::<Vec<_>>();
        let reduce_answers = state
            .reduce_answers()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(node_queries, trajectory.expected_node_queries);
        assert_eq!(state_json["edges"], trajectory.expected_edges);
        assert_eq!(reduce_answers, trajectory.expected_reduce_answers);
        for node in state
            .nodes()
            .iter()
            .filter(|node| matches!(node.query.as_str(), "Q1" | "Q3" | "C3" | "C1"))
        {
            assert_eq!(
                node.metadata.usage,
                QueryUsageCategory {
                    llm_calls: 1,
                    prompt_tokens: 4,
                    output_tokens: 5,
                }
            );
        }
    }

    #[test]
    fn test_should_fail_when_scripted_action_trajectory_is_exhausted() {
        let mut state = DriftQueryState::default();
        state.add_root(
            "root".to_owned(),
            "answer".to_owned(),
            1.0,
            &["one".to_owned()],
        );
        let mut random = ScriptedDriftRandom::new([], []);

        let error = select_actions(&state, &mut random, 1)
            .expect_err("exhausted trajectory must not use system randomness");

        assert!(
            error
                .to_string()
                .contains("scripted DRIFT action trajectory exhausted")
        );
    }

    #[test]
    fn test_should_reject_invalid_scripted_action_order() {
        let mut state = DriftQueryState::default();
        state.add_root(
            "root".to_owned(),
            "answer".to_owned(),
            1.0,
            &["one".to_owned(), "two".to_owned()],
        );
        let mut random = ScriptedDriftRandom::new([], [vec![0, 0]]);

        let error = select_actions(&state, &mut random, 2)
            .expect_err("duplicate trajectory index must fail");

        assert!(error.to_string().contains("action index 0 is duplicated"));
    }

    #[test]
    fn test_should_keep_system_random_available_for_production() {
        let mut random = SystemDriftRandom;
        let mut actions = [7];

        assert_eq!(random.choose_report(1).expect("single report selection"), 0);
        random
            .shuffle_actions(&mut actions)
            .expect("single action shuffle");
        assert_eq!(actions, [7]);
    }

    #[test]
    fn test_should_generate_the_same_action_graph_for_the_same_trajectory() {
        fn run() -> String {
            let mut state = DriftQueryState::default();
            state.add_root(
                "root".to_owned(),
                "primer".to_owned(),
                90.0,
                &["one".to_owned(), "two".to_owned()],
            );
            let mut random = ScriptedDriftRandom::new([], [vec![1, 0]]);
            let selected =
                select_actions(&state, &mut random, 1).expect("scripted graph selection");
            let selected_id = selected
                .first()
                .copied()
                .expect("one scripted graph action");
            state
                .apply(
                    selected_id,
                    DriftActionResponse {
                        answer: Some("action".to_owned()),
                        score: 80.0,
                        follow_up_queries: vec!["child".to_owned()],
                    },
                    DriftActionMetadata::default(),
                )
                .expect("apply scripted graph action");
            state.to_json().expect("scripted state JSON")
        }

        assert_eq!(run(), run());
    }

    #[test]
    fn test_should_deduplicate_equal_queries_before_scripted_selection() {
        let mut state = DriftQueryState::default();
        state.add_root(
            "root".to_owned(),
            "answer".to_owned(),
            1.0,
            &["same".to_owned(), "same".to_owned()],
        );
        let mut random = ScriptedDriftRandom::new([], [vec![0]]);

        let selected = select_actions(&state, &mut random, 20)
            .expect("duplicate query selection must follow GraphRAG identity");

        assert_eq!(selected, [1]);
        assert_eq!(state.edges_in_graph_order().len(), 2);
    }
}
