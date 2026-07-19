//! DRIFT primer folds and structured completions.

use std::{collections::BTreeMap, sync::Arc};

use graphloom_llm::{ChatMessage, CompletionModel, CompletionRequest, ModelConfig, Tokenizer};
use serde_json::json;

use super::{
    context::{RankedReport, count},
    parse::{PrimerResponse, parse_primer},
};
use crate::query::{QueryError, QueryUsageCategory, Result, SearchMethod};

// Fixed against GraphRAG 3.1.0:
// graphrag/prompts/query/drift_search_system_prompt.py::DRIFT_PRIMER_PROMPT.
pub(super) const DRIFT_PRIMER_PROMPT: &str = r#"You are a helpful agent designed to reason over a knowledge graph in response to a user query.
This is a unique knowledge graph where edges are freeform text rather than verb operators. You will begin your reasoning looking at a summary of the content of the most relevant communites and will provide:

1. score: How well the intermediate answer addresses the query. A score of 0 indicates a poor, unfocused answer, while a score of 100 indicates a highly focused, relevant answer that addresses the query in its entirety.

2. intermediate_answer: This answer should match the level of detail and length found in the community summaries. The intermediate answer should be exactly 2000 characters long. This must be formatted in markdown and must begin with a header that explains how the following text is related to the query.

3. follow_up_queries: A list of follow-up queries that could be asked to further explore the topic. These should be formatted as a list of strings. Generate at least five good follow-up queries.

Use this information to help you decide whether or not you need more information about the entities mentioned in the report. You may also use your general knowledge to think of entities which may help enrich your answer.

You will also provide a full answer from the content you have available. Use the data provided to generate follow-up queries to help refine your search. Do not ask compound questions, for example: "What is the market cap of Apple and Microsoft?". Use your knowledge of the entity distribution to focus on entity types that will be useful for searching a broad area of the knowledge graph.

For the query:

{query}

The top-ranked community summaries:

{community_reports}

Provide the intermediate answer, and all scores in JSON format following:

{{'intermediate_answer': str,
'score': int,
'follow_up_queries': List[str]}}

Begin:
"#;

#[derive(Debug, Clone)]
pub(super) struct PrimerAggregate {
    pub(super) answer: String,
    pub(super) score: f64,
    pub(super) followups: Vec<String>,
    pub(super) usage: QueryUsageCategory,
}

#[derive(Debug)]
pub(super) struct PrimerResources<'a> {
    pub(super) concurrency: usize,
    pub(super) model: Arc<dyn CompletionModel>,
    pub(super) model_id: &'a str,
    pub(super) model_config: &'a ModelConfig,
    pub(super) tokenizer: Arc<dyn Tokenizer>,
}

pub(super) async fn run_primer(
    reports: &[RankedReport],
    query: &str,
    folds: usize,
    resources: PrimerResources<'_>,
) -> Result<PrimerAggregate> {
    let splits = array_split(reports, folds);
    let calls = splits.into_iter().enumerate().map(|(index, fold)| {
        let model = Arc::clone(&resources.model);
        let tokenizer = Arc::clone(&resources.tokenizer);
        let model_id = resources.model_id.to_owned();
        let call_args = resources.model_config.call_args.clone();
        let query = query.to_owned();
        async move {
            run_fold(
                index, &fold, &query, model, &model_id, &call_args, tokenizer,
            )
            .await
        }
    });
    let results =
        crate::query::concurrency::try_buffered_ordered(calls, resources.concurrency).await?;
    aggregate_primer(results)
}

fn aggregate_primer(results: Vec<(PrimerResponse, QueryUsageCategory)>) -> Result<PrimerAggregate> {
    let answers = results
        .iter()
        .map(|(response, _)| response.intermediate_answer.as_str())
        .collect::<Vec<_>>();
    if answers.is_empty() {
        return Err(QueryError::QueryParse {
            method: SearchMethod::Drift,
            operation: "aggregate DRIFT primer responses",
            message: "primer returned no intermediate answer".to_owned(),
        });
    }
    let followups = results
        .iter()
        .flat_map(|(response, _)| response.follow_up_queries.iter().cloned())
        .collect::<Vec<_>>();
    if followups.is_empty() {
        return Err(QueryError::QueryParse {
            method: SearchMethod::Drift,
            operation: "aggregate DRIFT primer responses",
            message: "primer returned no follow-up queries".to_owned(),
        });
    }
    let score = results
        .iter()
        .map(|(response, _)| response.score as f64)
        .sum::<f64>()
        / results.len() as f64;
    let usage = results
        .iter()
        .fold(QueryUsageCategory::default(), |mut total, (_, usage)| {
            total += *usage;
            total
        });
    Ok(PrimerAggregate {
        answer: answers.join("\n\n"),
        score,
        followups,
        usage,
    })
}

fn array_split<T: Clone>(items: &[T], folds: usize) -> Vec<Vec<T>> {
    let folds = folds.max(1);
    let base = items.len() / folds;
    let remainder = items.len() % folds;
    let mut offset = 0_usize;
    (0..folds)
        .map(|index| {
            let length = base + usize::from(index < remainder);
            let end = offset.saturating_add(length);
            let fold = items.get(offset..end).unwrap_or_default().to_vec();
            offset = end;
            fold
        })
        .collect()
}

async fn run_fold(
    _index: usize,
    reports: &[RankedReport],
    query: &str,
    model: Arc<dyn CompletionModel>,
    model_id: &str,
    call_args: &BTreeMap<String, serde_json::Value>,
    tokenizer: Arc<dyn Tokenizer>,
) -> Result<(PrimerResponse, QueryUsageCategory)> {
    let report_text = reports
        .iter()
        .map(|report| report.full_content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let prompt = render_primer_prompt(query, &report_text)?;
    let prompt_tokens = count(&*tokenizer, &prompt, "count DRIFT primer prompt")?;
    let mut request = CompletionRequest::new(vec![ChatMessage::user(prompt)]);
    request
        .apply_call_args(call_args)
        .and_then(|()| {
            request.stream = Some(false);
            request.response_format = Some(primer_response_format());
            request.validate()
        })
        .map_err(|source| QueryError::InvalidQueryConfig {
            method: SearchMethod::Drift,
            operation: "build DRIFT primer completion request",
            message: source.to_string(),
        })?;
    let response = model
        .complete(request)
        .await
        .map_err(|source| QueryError::QueryCompletion {
            method: SearchMethod::Drift,
            operation: "complete DRIFT primer fold",
            model: model_id.to_owned(),
            source: Box::new(source),
        })?;
    let content = response
        .content()
        .map_err(|source| QueryError::QueryCompletion {
            method: SearchMethod::Drift,
            operation: "read DRIFT primer response",
            model: model_id.to_owned(),
            source: Box::new(source),
        })?;
    let output_tokens = count(&*tokenizer, content, "count DRIFT primer output")?;
    Ok((
        parse_primer(content)?,
        QueryUsageCategory {
            llm_calls: 1,
            prompt_tokens,
            output_tokens,
        },
    ))
}

fn render_primer_prompt(query: &str, reports: &str) -> Result<String> {
    let normalized = DRIFT_PRIMER_PROMPT.replace("{{", "{").replace("}}", "}");
    let (before_query, after_query) =
        normalized
            .split_once("{query}")
            .ok_or_else(|| QueryError::QueryPrompt {
                method: SearchMethod::Drift,
                operation: "render DRIFT primer prompt",
                prompt: "DRIFT_PRIMER_PROMPT",
                source: Box::new(crate::GraphLoomError::PromptRender {
                    kind: "DriftPrimer",
                    name: "DRIFT_PRIMER_PROMPT",
                    prompt_source: "built-in GraphRAG 3.1.0 constant".to_owned(),
                    message: "missing {query} placeholder".to_owned(),
                }),
            })?;
    let (between, after_reports) =
        after_query
            .split_once("{community_reports}")
            .ok_or_else(|| QueryError::QueryPrompt {
                method: SearchMethod::Drift,
                operation: "render DRIFT primer prompt",
                prompt: "DRIFT_PRIMER_PROMPT",
                source: Box::new(crate::GraphLoomError::PromptRender {
                    kind: "DriftPrimer",
                    name: "DRIFT_PRIMER_PROMPT",
                    prompt_source: "built-in GraphRAG 3.1.0 constant".to_owned(),
                    message: "missing {community_reports} placeholder".to_owned(),
                }),
            })?;
    Ok([before_query, query, between, reports, after_reports].concat())
}

fn primer_response_format() -> serde_json::Value {
    json!({
        "type": "json_schema",
        "json_schema": {
            "name": "PrimerResponse",
            "strict": true,
            "schema": {
                "type": "object",
                "title": "PrimerResponse",
                "description": "Response model for the primer.",
                "properties": {
                    "intermediate_answer": {
                        "type": "string",
                        "title": "Intermediate Answer",
                        "description": "This answer should match the level of detail and length found in the community summaries. The intermediate answer should be exactly 2000 characters long. This must be formatted in markdown and must begin with a header that explains how the following text is related to the query."
                    },
                    "score": {
                        "type": "integer",
                        "title": "Score",
                        "description": "A score on how well the intermediate answer addresses the query. A score of 0 indicates a poor, unfocused answer, while a score of 100 indicates a highly focused, relevant answer that addresses the query in its entirety."
                    },
                    "follow_up_queries": {
                        "type": "array",
                        "title": "Follow Up Queries",
                        "items": {"type": "string"},
                        "description": "A list of follow-up queries that could be asked to further explore the topic. These should be formatted as a list of strings. Generate at least five good follow-up queries."
                    }
                },
                "required": ["intermediate_answer", "score", "follow_up_queries"],
                "additionalProperties": false
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use graphloom_llm::{
        CompletionModel, CompletionRequest, CompletionResponse, ModelConfig, TiktokenTokenizer,
        Tokenizer,
    };
    use serde_json::json;

    use super::{PrimerResponse, aggregate_primer, array_split};
    use crate::query::QueryUsageCategory;

    #[derive(Debug)]
    struct ConcurrentModel {
        current: AtomicUsize,
        maximum: AtomicUsize,
    }

    #[async_trait]
    impl CompletionModel for ConcurrentModel {
        async fn complete(
            &self,
            _: CompletionRequest,
        ) -> graphloom_llm::Result<CompletionResponse> {
            let current = self.current.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(current, Ordering::SeqCst);
            tokio::task::yield_now().await;
            self.current.fetch_sub(1, Ordering::SeqCst);
            Ok(CompletionResponse::text_for_test(
                "test",
                r#"{"intermediate_answer":"answer","score":50,"follow_up_queries":["next"]}"#,
            ))
        }
    }

    #[test]
    fn test_should_match_numpy_array_split_for_uneven_and_empty_folds() {
        assert_eq!(
            array_split(&[0, 1, 2, 3, 4], 3),
            vec![vec![0, 1], vec![2, 3], vec![4]]
        );
        assert_eq!(
            array_split(&[0, 1], 4),
            vec![vec![0], vec![1], Vec::<i32>::new(), Vec::new()]
        );
        assert_eq!(array_split(&[0, 1], 0), vec![vec![0, 1]]);
    }

    #[test]
    fn test_should_aggregate_primer_in_fold_order_with_average_score() {
        let aggregate = aggregate_primer(vec![
            (
                PrimerResponse {
                    intermediate_answer: "first".to_owned(),
                    score: 60,
                    follow_up_queries: vec!["one".to_owned(), "same".to_owned()],
                },
                QueryUsageCategory {
                    llm_calls: 1,
                    prompt_tokens: 10,
                    output_tokens: 2,
                },
            ),
            (
                PrimerResponse {
                    intermediate_answer: "second".to_owned(),
                    score: 80,
                    follow_up_queries: vec!["same".to_owned()],
                },
                QueryUsageCategory {
                    llm_calls: 1,
                    prompt_tokens: 20,
                    output_tokens: 3,
                },
            ),
        ])
        .expect("aggregate");

        assert_eq!(aggregate.answer, "first\n\nsecond");
        assert_eq!(aggregate.score, 70.0);
        assert_eq!(aggregate.followups, ["one", "same", "same"]);
        assert_eq!(aggregate.usage.llm_calls, 2);
        assert_eq!(aggregate.usage.prompt_tokens, 30);
        assert_eq!(aggregate.usage.output_tokens, 5);
    }

    #[test]
    fn test_should_preserve_empty_answer_and_reject_missing_followups() {
        let usage = QueryUsageCategory::default();
        let aggregate = aggregate_primer(vec![(
            PrimerResponse {
                intermediate_answer: String::new(),
                score: 1,
                follow_up_queries: vec!["next".to_owned()],
            },
            usage,
        )])
        .expect("required empty answer remains present");
        assert_eq!(aggregate.answer, "");
        assert_eq!(aggregate.followups, ["next"]);
        assert!(
            aggregate_primer(vec![(
                PrimerResponse {
                    intermediate_answer: "answer".to_owned(),
                    score: 1,
                    follow_up_queries: Vec::new(),
                },
                usage,
            )])
            .is_err()
        );
    }

    #[tokio::test]
    async fn test_should_bound_primer_concurrency_and_call_empty_folds() {
        let model = Arc::new(ConcurrentModel {
            current: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
        });
        let model_config: ModelConfig = serde_json::from_value(json!({
            "model_provider": "openai",
            "model": "test",
            "api_key": "test",
        }))
        .expect("model config");
        let tokenizer: Arc<dyn Tokenizer> =
            Arc::new(TiktokenTokenizer::new("cl100k_base").expect("tokenizer"));
        let reports = vec![super::RankedReport {
            short_id: "0".to_owned(),
            community_id: "0".to_owned(),
            full_content: "report".to_owned(),
            similarity: 1.0,
        }];

        let aggregate = super::run_primer(
            &reports,
            "query",
            4,
            super::PrimerResources {
                concurrency: 2,
                model: model.clone(),
                model_id: "test",
                model_config: &model_config,
                tokenizer,
            },
        )
        .await
        .expect("primer");

        assert_eq!(aggregate.usage.llm_calls, 4);
        assert_eq!(model.maximum.load(Ordering::SeqCst), 2);
    }
}
