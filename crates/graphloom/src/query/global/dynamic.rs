//! GraphRAG 3.1 dynamic community rating and hierarchy traversal.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use futures_util::{StreamExt, stream};
use graphloom_llm::{ChatMessage, CompletionModel, CompletionRequest, ModelConfig, Tokenizer};
use serde::Serialize;
use serde_json::Value;

use super::parse::{first_balanced_json_object, first_json_object, python_int};
use crate::{
    config::GlobalSearchConfig,
    query::{Community, CommunityReport, QueryError, QueryUsageCategory, Result, SearchMethod},
};

// Fixed against Microsoft GraphRAG v3.1.0:
// packages/graphrag/graphrag/query/context_builder/rate_prompt.py.
// This is runtime business text, not a project/init prompt asset.
const RATE_PROMPT: &str = r#"
---Role---
You are a helpful assistant responsible for deciding whether the provided information is useful in answering a given question, even if it is only partially relevant.
---Goal---
On a scale from 0 to 5, please rate how relevant or helpful is the provided information in answering the question.
---Information---
{{ description }}
---Question---
{{ question }}
---Target response length and format---
Please response in the following JSON format with two entries:
- "reason": the reasoning of your rating, please include information that you have considered.
- "rating": the relevancy rating from 0 to 5, where 0 is the least relevant and 5 is the most relevant.
{
    "reason": str,
    "rating": int.
}
"#;

/// Rating metadata for one community visited during dynamic selection.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DynamicRating {
    /// Decimal community identifier.
    pub community_id: String,
    /// Majority-vote rating.
    pub selected_rating: i64,
    /// Ratings from every configured repeat.
    pub repeated_ratings: Vec<Value>,
    /// Whether the community remains in the final selected set.
    pub selected: bool,
    /// Hierarchy level at which the community was rated.
    pub level: i64,
}

#[derive(Debug)]
pub(super) struct DynamicSelectionResult {
    pub(super) reports: Vec<CommunityReport>,
    pub(super) ratings: Vec<DynamicRating>,
    pub(super) usage: QueryUsageCategory,
}

#[derive(Debug)]
pub(super) struct DynamicCommunitySelection {
    config: GlobalSearchConfig,
    reports: Vec<CommunityReport>,
    communities: Vec<Community>,
    model: Arc<dyn CompletionModel>,
    model_id: String,
    model_config: ModelConfig,
    tokenizer: Arc<dyn Tokenizer>,
    concurrent_requests: usize,
}

#[derive(Debug)]
struct CommunityVote {
    community_id: String,
    selected_rating: i64,
    repeated_ratings: Vec<Value>,
    usage: QueryUsageCategory,
}

#[derive(Debug, Serialize)]
struct RatePromptContext<'a> {
    description: &'a str,
    question: &'a str,
}

impl DynamicCommunitySelection {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        config: GlobalSearchConfig,
        reports: Vec<CommunityReport>,
        communities: Vec<Community>,
        model: Arc<dyn CompletionModel>,
        model_id: String,
        model_config: ModelConfig,
        tokenizer: Arc<dyn Tokenizer>,
        concurrent_requests: usize,
    ) -> Self {
        Self {
            config,
            reports,
            communities,
            model,
            model_id,
            model_config,
            tokenizer,
            concurrent_requests,
        }
    }

    pub(super) async fn select(&self, query: &str) -> Result<DynamicSelectionResult> {
        let reports_by_id = self
            .reports
            .iter()
            .map(|report| (report.community_id.clone(), report))
            .collect::<HashMap<_, _>>();
        let communities_by_id = self
            .communities
            .iter()
            .map(|community| (community.short_id.clone(), community))
            .collect::<HashMap<_, _>>();
        let mut levels = BTreeMap::<i64, Vec<String>>::new();
        for community in &self.communities {
            if reports_by_id.contains_key(&community.short_id) {
                let communities_at_level = levels.entry(community.level).or_default();
                if !communities_at_level.contains(&community.short_id) {
                    communities_at_level.push(community.short_id.clone());
                }
            }
        }
        for report in &self.reports {
            if !communities_by_id.contains_key(&report.community_id) {
                tracing::warn!(
                    method = %SearchMethod::Global,
                    community = %report.community_id,
                    "dynamic report has no matching community metadata"
                );
            }
        }
        let Some(starting) = levels.get(&0) else {
            return Err(QueryError::QueryContext {
                method: SearchMethod::Global,
                operation: "initialize dynamic community selection",
                message: "no report-backed level 0 communities are available".to_owned(),
            });
        };

        let mut queue = starting.clone();
        let mut queued_or_visited = queue.iter().cloned().collect::<HashSet<_>>();
        let mut traversal_level = 0_i64;
        let mut relevant = HashSet::<String>::new();
        // GraphRAG materializes this collection from a Python set, whose order is not a
        // compatibility contract. Preserve traversal first-seen order here; the downstream
        // Global context builder still applies the Python-compatible seed-86 shuffle.
        let mut relevant_first_seen = Vec::<String>::new();
        let mut ratings = Vec::<DynamicRating>::new();
        let mut usage = QueryUsageCategory::default();

        while !queue.is_empty() && traversal_level <= self.config.dynamic_search_max_level {
            let votes = self.rate_queue(&queue, &reports_by_id, query).await?;
            let mut next_queue = Vec::<String>::new();
            for (community_id, vote) in queue.iter().zip(votes) {
                usage.llm_calls += vote.usage.llm_calls;
                usage.prompt_tokens += vote.usage.prompt_tokens;
                usage.output_tokens += vote.usage.output_tokens;
                let relevant_vote = vote.selected_rating >= self.config.dynamic_search_threshold;
                if relevant_vote {
                    if relevant.insert(community_id.clone()) {
                        relevant_first_seen.push(community_id.clone());
                    }
                    if let Some(community) = communities_by_id.get(community_id) {
                        if !self.config.dynamic_search_keep_parent {
                            relevant.remove(&community.parent.to_string());
                        }
                        if traversal_level < self.config.dynamic_search_max_level {
                            for child in &community.children {
                                let child_id = child.to_string();
                                if !reports_by_id.contains_key(&child_id) {
                                    tracing::debug!(
                                        method = %SearchMethod::Global,
                                        community = %community_id,
                                        child = %child_id,
                                        "dynamic child has no report and was skipped"
                                    );
                                } else if queued_or_visited.insert(child_id.clone()) {
                                    next_queue.push(child_id);
                                } else {
                                    tracing::warn!(
                                        method = %SearchMethod::Global,
                                        community = %community_id,
                                        child = %child_id,
                                        "dynamic hierarchy duplicate or cycle was skipped"
                                    );
                                }
                            }
                        }
                    } else {
                        tracing::warn!(
                            method = %SearchMethod::Global,
                            community = %community_id,
                            "rated dynamic community has no hierarchy metadata"
                        );
                    }
                }
                ratings.push(DynamicRating {
                    community_id: vote.community_id,
                    selected_rating: vote.selected_rating,
                    repeated_ratings: vote.repeated_ratings,
                    selected: false,
                    level: communities_by_id
                        .get(community_id)
                        .map_or(traversal_level, |community| community.level),
                });
            }

            traversal_level = traversal_level.saturating_add(1);
            if next_queue.is_empty()
                && relevant.is_empty()
                && traversal_level <= self.config.dynamic_search_max_level
                && let Some(fallback) = levels.get(&traversal_level)
            {
                for community_id in fallback {
                    if queued_or_visited.insert(community_id.clone()) {
                        next_queue.push(community_id.clone());
                    }
                }
            }
            queue = next_queue;
        }

        for rating in &mut ratings {
            rating.selected = relevant.contains(&rating.community_id);
        }
        let reports = relevant_first_seen
            .into_iter()
            .filter(|community_id| relevant.contains(community_id))
            .filter_map(|community_id| reports_by_id.get(&community_id).copied().cloned())
            .collect();
        Ok(DynamicSelectionResult {
            reports,
            ratings,
            usage,
        })
    }

    async fn rate_queue(
        &self,
        queue: &[String],
        reports: &HashMap<String, &CommunityReport>,
        query: &str,
    ) -> Result<Vec<CommunityVote>> {
        let futures = queue.iter().map(|community_id| {
            let community_id = community_id.clone();
            let report = reports.get(&community_id).copied().cloned();
            let query = query.to_owned();
            async move {
                let report = report.ok_or_else(|| QueryError::QueryContext {
                    method: SearchMethod::Global,
                    operation: "rate dynamic community",
                    message: format!("community {community_id:?} has no report"),
                })?;
                self.rate_community(community_id, report, &query).await
            }
        });
        stream::iter(futures)
            .buffered(self.concurrent_requests)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect()
    }

    async fn rate_community(
        &self,
        community_id: String,
        report: CommunityReport,
        query: &str,
    ) -> Result<CommunityVote> {
        let description = if self.config.dynamic_search_use_summary {
            &report.summary
        } else {
            &report.full_content
        };
        let rendered = render_rate_prompt(description, query)?;
        let prompt_tokens =
            self.tokenizer
                .count(&rendered)
                .map_err(|source| QueryError::QueryContext {
                    method: SearchMethod::Global,
                    operation: "count dynamic rating prompt tokens",
                    message: source.to_string(),
                })?;
        let user_tokens =
            self.tokenizer
                .count(query)
                .map_err(|source| QueryError::QueryContext {
                    method: SearchMethod::Global,
                    operation: "count dynamic rating user tokens",
                    message: source.to_string(),
                })?;
        let mut repeated_ratings = Vec::with_capacity(self.config.dynamic_search_num_repeats);
        let mut output_tokens = 0_usize;
        for _ in 0..self.config.dynamic_search_num_repeats {
            let mut request = CompletionRequest::new(vec![
                ChatMessage::system(rendered.clone()),
                ChatMessage::user(query),
            ]);
            request
                .apply_call_args(&self.model_config.call_args)
                .and_then(|()| {
                    request.stream = Some(false);
                    request.response_format = Some(serde_json::json!({"type": "json_object"}));
                    request.validate()
                })
                .map_err(|source| QueryError::InvalidQueryConfig {
                    method: SearchMethod::Global,
                    operation: "build dynamic rating request",
                    message: source.to_string(),
                })?;
            let response = self.model.complete(request).await.map_err(|source| {
                QueryError::QueryCompletion {
                    method: SearchMethod::Global,
                    operation: "complete dynamic community rating",
                    model: self.model_id.clone(),
                    source: Box::new(source),
                }
            })?;
            let raw = response
                .content()
                .map_err(|source| QueryError::QueryCompletion {
                    method: SearchMethod::Global,
                    operation: "read dynamic community rating",
                    model: self.model_id.clone(),
                    source: Box::new(source),
                })?
                .to_owned();
            output_tokens =
                output_tokens.saturating_add(self.tokenizer.count(&raw).map_err(|source| {
                    QueryError::QueryContext {
                        method: SearchMethod::Global,
                        operation: "count dynamic rating output tokens",
                        message: source.to_string(),
                    }
                })?);
            repeated_ratings.push(parse_rating(&raw)?);
        }
        let selected_rating =
            majority_vote(&repeated_ratings)?.ok_or_else(|| QueryError::QueryContext {
                method: SearchMethod::Global,
                operation: "vote on dynamic community rating",
                message: "dynamic rating produced no repeated ratings".to_owned(),
            })?;
        Ok(CommunityVote {
            community_id,
            selected_rating,
            repeated_ratings,
            usage: QueryUsageCategory {
                llm_calls: self.config.dynamic_search_num_repeats,
                prompt_tokens: prompt_tokens
                    .saturating_add(user_tokens)
                    .saturating_mul(self.config.dynamic_search_num_repeats),
                output_tokens,
            },
        })
    }
}

fn render_rate_prompt(description: &str, question: &str) -> Result<String> {
    let context = tera::Context::from_serialize(RatePromptContext {
        description,
        question,
    })
    .map_err(|source| rate_prompt_error(source.to_string()))?;
    tera::Tera::one_off(RATE_PROMPT, &context, false)
        .map_err(|source| rate_prompt_error(source.to_string()))
}

fn rate_prompt_error(message: String) -> QueryError {
    QueryError::QueryPrompt {
        method: SearchMethod::Global,
        operation: "render dynamic rating prompt",
        prompt: "query/context_builder/rate_prompt.py",
        source: Box::new(crate::GraphLoomError::PromptRender {
            kind: "dynamic community rating",
            name: "rate_prompt.py",
            prompt_source: "built-in GraphRAG 3.1.0 business text".to_owned(),
            message,
        }),
    }
}

fn parse_rating(input: &str) -> Result<Value> {
    let value = if let Some(object) = first_json_object(input) {
        serde_json::from_str::<Value>(object).ok()
    } else {
        first_balanced_json_object(input)
            .map(repair_json_object)
            .and_then(|object| serde_json::from_str::<Value>(&object).ok())
    };
    let Some(value) = value else {
        return Ok(Value::from(1));
    };
    let Some(rating) = value.get("rating") else {
        return Ok(Value::from(1));
    };
    match rating {
        Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(rating.clone()),
        _ => Err(invalid_rating(rating)),
    }
}

/// Apply the two JSON repairs observable in GraphRAG's `json_repair` path that
/// affect rating responses: quote bare object keys and remove trailing commas.
fn repair_json_object(input: &str) -> String {
    let mut repaired = String::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        let Some(character) = input[index..].chars().next() else {
            break;
        };
        if character == '"' {
            let start = index;
            index += character.len_utf8();
            let mut escaped = false;
            while index < input.len() {
                let Some(string_character) = input[index..].chars().next() else {
                    break;
                };
                index += string_character.len_utf8();
                if escaped {
                    escaped = false;
                } else if string_character == '\\' {
                    escaped = true;
                } else if string_character == '"' {
                    break;
                }
            }
            repaired.push_str(&input[start..index]);
            continue;
        }
        if matches!(character, '{' | ',') {
            let delimiter = character;
            index += character.len_utf8();
            let whitespace_start = index;
            while input[index..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
            {
                index += input[index..].chars().next().map_or(0, char::len_utf8);
            }
            if delimiter == ','
                && input[index..]
                    .chars()
                    .next()
                    .is_some_and(|next| matches!(next, '}' | ']'))
            {
                repaired.push_str(&input[whitespace_start..index]);
                continue;
            }
            repaired.push(delimiter);
            repaired.push_str(&input[whitespace_start..index]);
            let key_start = index;
            while input[index..]
                .chars()
                .next()
                .is_some_and(|next| next.is_ascii_alphanumeric() || matches!(next, '_' | '-'))
            {
                index += input[index..].chars().next().map_or(0, char::len_utf8);
            }
            let key_end = index;
            let key_whitespace_start = index;
            while input[index..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
            {
                index += input[index..].chars().next().map_or(0, char::len_utf8);
            }
            if key_end > key_start && input[index..].starts_with(':') {
                repaired.push('"');
                repaired.push_str(&input[key_start..key_end]);
                repaired.push('"');
                repaired.push_str(&input[key_whitespace_start..index]);
                repaired.push(':');
                index += 1;
            } else {
                repaired.push_str(&input[key_start..index]);
            }
            continue;
        }
        repaired.push(character);
        index += character.len_utf8();
    }
    repaired
}

fn majority_vote(ratings: &[Value]) -> Result<Option<i64>> {
    if ratings.is_empty() {
        return Ok(None);
    }
    if ratings.iter().any(Value::is_string) {
        let mut counts = BTreeMap::<String, usize>::new();
        for rating in ratings {
            *counts.entry(python_rating_string(rating)?).or_default() += 1;
        }
        let selected = select_counted(counts)
            .and_then(|rating| rating.trim().parse::<i64>().ok())
            .ok_or_else(|| invalid_rating(&ratings[0]))?;
        return Ok(Some(selected));
    }
    if ratings
        .iter()
        .any(|rating| rating.as_f64().is_some_and(|value| value.fract() != 0.0))
    {
        let mut values = ratings
            .iter()
            .map(|rating| rating.as_f64().ok_or_else(|| invalid_rating(rating)))
            .collect::<Result<Vec<_>>>()?;
        values.sort_by(f64::total_cmp);
        let mut counted = Vec::<(f64, usize)>::new();
        for value in values {
            if let Some((last, count)) = counted.last_mut()
                && *last == value
            {
                *count += 1;
            } else {
                counted.push((value, 1));
            }
        }
        let Some(selected) = select_counted(counted) else {
            return Ok(None);
        };
        return python_int(&Value::from(selected))
            .map(Some)
            .ok_or_else(|| invalid_rating(&Value::from(selected)));
    }
    let mut counts = BTreeMap::<i64, usize>::new();
    for rating in ratings {
        let value = python_int(rating).ok_or_else(|| invalid_rating(rating))?;
        *counts.entry(value).or_default() += 1;
    }
    Ok(select_counted(counts))
}

fn select_counted<T>(values: impl IntoIterator<Item = (T, usize)>) -> Option<T> {
    let mut selected = None::<(T, usize)>;
    for (value, count) in values {
        if selected
            .as_ref()
            .is_none_or(|(_, selected_count)| count > *selected_count)
        {
            selected = Some((value, count));
        }
    }
    selected.map(|(value, _)| value)
}

fn python_rating_string(rating: &Value) -> Result<String> {
    match rating {
        Value::String(value) => Ok(value.clone()),
        Value::Bool(true) => Ok("True".to_owned()),
        Value::Bool(false) => Ok("False".to_owned()),
        Value::Number(value) => Ok(value.to_string()),
        _ => Err(invalid_rating(rating)),
    }
}

fn invalid_rating(rating: &Value) -> QueryError {
    QueryError::QueryParse {
        method: SearchMethod::Global,
        operation: "parse dynamic community rating",
        message: format!("rating value {rating} cannot be converted to an integer"),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, VecDeque},
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use async_trait::async_trait;
    use graphloom_llm::{
        CompletionModel, CompletionRequest, CompletionResponse, LlmError, ModelConfig, Tokenizer,
    };
    use serde_json::Value;

    use super::{DynamicCommunitySelection, majority_vote, parse_rating, render_rate_prompt};
    use crate::{
        config::GlobalSearchConfig,
        query::{Community, CommunityReport, QueryError},
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

    #[derive(Debug)]
    struct ScriptedRatingModel {
        scripts: Mutex<BTreeMap<String, VecDeque<String>>>,
        requests: Mutex<Vec<CompletionRequest>>,
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
    }

    impl ScriptedRatingModel {
        fn new(scripts: impl IntoIterator<Item = (&'static str, Vec<&'static str>)>) -> Self {
            Self {
                scripts: Mutex::new(
                    scripts
                        .into_iter()
                        .map(|(marker, values)| {
                            (
                                marker.to_owned(),
                                values
                                    .into_iter()
                                    .map(ToOwned::to_owned)
                                    .collect::<VecDeque<_>>(),
                            )
                        })
                        .collect(),
                ),
                requests: Mutex::new(Vec::new()),
                in_flight: AtomicUsize::new(0),
                max_in_flight: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl CompletionModel for ScriptedRatingModel {
        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> graphloom_llm::Result<CompletionResponse> {
            let current = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(current, Ordering::SeqCst);
            let system = request.messages[0].content.as_str();
            let (marker, response) = {
                let mut scripts =
                    self.scripts
                        .lock()
                        .map_err(|source| LlmError::InvalidResponse {
                            model_instance: "rating-test".to_owned(),
                            operation: "read scripted rating",
                            message: source.to_string(),
                        })?;
                let marker = scripts
                    .keys()
                    .find(|marker| system.contains(marker.as_str()))
                    .cloned()
                    .ok_or_else(|| LlmError::InvalidResponse {
                        model_instance: "rating-test".to_owned(),
                        operation: "match scripted rating",
                        message: "no description marker matched".to_owned(),
                    })?;
                let response = scripts
                    .get_mut(&marker)
                    .and_then(VecDeque::pop_front)
                    .ok_or_else(|| LlmError::InvalidResponse {
                        model_instance: "rating-test".to_owned(),
                        operation: "read scripted rating",
                        message: format!("no response remains for marker {marker}"),
                    })?;
                (marker, response)
            };
            self.requests
                .lock()
                .map_err(|source| LlmError::InvalidResponse {
                    model_instance: "rating-test".to_owned(),
                    operation: "record rating request",
                    message: source.to_string(),
                })?
                .push(request);
            let delay = if marker.contains("ROOT-A") { 12 } else { 2 };
            tokio::time::sleep(Duration::from_millis(delay)).await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(CompletionResponse::text_for_test("rating-test", response))
        }
    }

    #[derive(Debug)]
    struct FailingRatingModel;

    #[async_trait]
    impl CompletionModel for FailingRatingModel {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> graphloom_llm::Result<CompletionResponse> {
            Err(LlmError::Timeout {
                model_instance: "rating-test".to_owned(),
                operation: "complete",
                attempts: 1,
            })
        }
    }

    fn report(id: i64, marker: &str) -> CommunityReport {
        CommunityReport {
            id: format!("report-{id}"),
            short_id: id.to_string(),
            community_id: id.to_string(),
            title: format!("Report {id}"),
            summary: format!("SUMMARY-{marker}"),
            full_content: format!("FULL-{marker}"),
            rank: Some(1.0),
            full_content_embedding: None,
        }
    }

    fn community(id: i64, level: i64, parent: i64, children: &[i64]) -> Community {
        Community {
            id: format!("community-{id}"),
            short_id: id.to_string(),
            title: format!("Community {id}"),
            level,
            parent,
            children: children.to_vec(),
        }
    }

    fn config(
        threshold: i64,
        keep_parent: bool,
        repeats: usize,
        max_level: i64,
    ) -> GlobalSearchConfig {
        GlobalSearchConfig {
            dynamic_search_threshold: threshold,
            dynamic_search_keep_parent: keep_parent,
            dynamic_search_num_repeats: repeats,
            dynamic_search_max_level: max_level,
            ..GlobalSearchConfig::default()
        }
    }

    fn model_config() -> ModelConfig {
        serde_json::from_value(serde_json::json!({
            "model_provider": "mock",
            "model": "rating-test",
            "call_args": {
                "stream": true,
                "response_format": {"type": "text"},
                "temperature": 0.1,
                "seed": 7
            }
        }))
        .expect("model config")
    }

    fn selector(
        config: GlobalSearchConfig,
        reports: Vec<CommunityReport>,
        communities: Vec<Community>,
        model: Arc<ScriptedRatingModel>,
        concurrent_requests: usize,
    ) -> DynamicCommunitySelection {
        DynamicCommunitySelection::new(
            config,
            reports,
            communities,
            model,
            "rating-test".to_owned(),
            model_config(),
            Arc::new(WordTokenizer),
            concurrent_requests,
        )
    }

    #[test]
    fn test_should_render_exact_rate_prompt() {
        let rendered = render_rate_prompt("DESCRIPTION", "QUESTION").expect("rate prompt");
        assert_eq!(
            rendered,
            include_str!("../../../../../tests/compat/fixtures/query/rate_prompt.txt")
        );
    }

    #[test]
    fn test_should_parse_rating_fallbacks_and_python_integer_semantics() {
        for input in [
            "{}",
            r#"{"reason":"missing"}"#,
            "not json",
            r#"```json
{"reason":"missing"}
```"#,
        ] {
            assert_eq!(parse_rating(input).expect("fallback"), Value::from(1));
        }
        for (input, expected) in [
            (r#"{"rating":3}"#, 3),
            ("prose {\"rating\":\"4\"}", 4),
            (r#"{"rating":4,}"#, 4),
            (r#"{rating: 4}"#, 4),
            (
                r#"```json
{"rating":2.9}
```"#,
                2,
            ),
            (r#"{"rating":true}"#, 1),
        ] {
            assert_eq!(
                majority_vote(&[parse_rating(input).expect("rating")]).expect("vote"),
                Some(expected)
            );
        }
        assert!(matches!(
            parse_rating(r#"{"rating":null}"#),
            Err(QueryError::QueryParse {
                operation: "parse dynamic community rating",
                ..
            })
        ));
    }

    #[test]
    fn test_should_vote_by_majority_and_choose_smallest_rating_on_tie() {
        assert_eq!(
            majority_vote(&[Value::from(4), Value::from(2), Value::from(4)]).expect("majority"),
            Some(4)
        );
        assert_eq!(
            majority_vote(&[Value::from(4), Value::from(2)]).expect("tie"),
            Some(2)
        );
        assert_eq!(
            majority_vote(&[Value::from(2.9), Value::from(3.1), Value::from(3.9),])
                .expect("float vote"),
            Some(2)
        );
        assert_eq!(majority_vote(&[]).expect("empty vote"), None);
    }

    #[tokio::test]
    async fn test_should_select_threshold_equal_and_apply_keep_parent_semantics() {
        for (keep_parent, expected) in [(true, vec!["0", "1"]), (false, vec!["1"])] {
            let model = Arc::new(ScriptedRatingModel::new([
                ("ROOT", vec![r#"{"rating":3}"#]),
                ("CHILD", vec![r#"{"rating":4}"#]),
            ]));
            let result = selector(
                config(3, keep_parent, 1, 2),
                vec![report(0, "ROOT"), report(1, "CHILD")],
                vec![community(0, 0, -1, &[1]), community(1, 1, 0, &[])],
                model,
                2,
            )
            .select("question")
            .await
            .expect("selection");
            assert_eq!(
                result
                    .reports
                    .iter()
                    .map(|report| report.community_id.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(result.usage.llm_calls, 2);
        }
    }

    #[tokio::test]
    async fn test_should_fallback_through_levels_until_relevant() {
        let model = Arc::new(ScriptedRatingModel::new([
            ("LEVEL0", vec![r#"{"rating":0}"#]),
            ("LEVEL1", vec![r#"{"rating":1}"#]),
            ("LEVEL2", vec![r#"{"rating":5}"#]),
        ]));
        let result = selector(
            config(3, false, 1, 2),
            vec![
                report(0, "LEVEL0"),
                report(1, "LEVEL1"),
                report(2, "LEVEL2"),
            ],
            vec![
                community(0, 0, -1, &[]),
                community(1, 1, -1, &[]),
                community(2, 2, -1, &[]),
            ],
            model,
            2,
        )
        .select("question")
        .await
        .expect("fallback");
        assert_eq!(result.reports[0].community_id, "2");
        assert_eq!(result.ratings.len(), 3);
    }

    #[tokio::test]
    async fn test_should_not_jump_missing_level_or_fallback_after_relevant_result() {
        let missing_model = Arc::new(ScriptedRatingModel::new([(
            "LEVEL0",
            vec![r#"{"rating":0}"#],
        )]));
        let missing = selector(
            config(3, true, 1, 2),
            vec![report(0, "LEVEL0"), report(2, "LEVEL2")],
            vec![community(0, 0, -1, &[]), community(2, 2, -1, &[])],
            missing_model,
            1,
        )
        .select("question")
        .await
        .expect("missing fallback level");
        assert!(missing.reports.is_empty());
        assert_eq!(missing.usage.llm_calls, 1);

        let relevant_model = Arc::new(ScriptedRatingModel::new([(
            "LEVEL0",
            vec![r#"{"rating":5}"#],
        )]));
        let relevant = selector(
            config(3, true, 1, 2),
            vec![report(0, "LEVEL0"), report(1, "LEVEL1")],
            vec![community(0, 0, -1, &[]), community(1, 1, -1, &[])],
            relevant_model,
            1,
        )
        .select("question")
        .await
        .expect("relevant root");
        assert_eq!(relevant.reports[0].community_id, "0");
        assert_eq!(relevant.usage.llm_calls, 1);
    }

    #[tokio::test]
    async fn test_should_dedupe_children_and_break_hierarchy_cycles() {
        let model = Arc::new(ScriptedRatingModel::new([
            ("ROOT", vec![r#"{"rating":5}"#]),
            ("CHILD", vec![r#"{"rating":5}"#]),
        ]));
        let result = selector(
            config(3, true, 1, 3),
            vec![report(0, "ROOT"), report(1, "CHILD")],
            vec![community(0, 0, -1, &[1, 1]), community(1, 1, 0, &[0])],
            Arc::clone(&model),
            2,
        )
        .select("question")
        .await
        .expect("cycle-safe selection");
        assert_eq!(result.usage.llm_calls, 2);
        assert_eq!(result.reports.len(), 2);
        assert_eq!(model.requests.lock().expect("requests").len(), 2);
    }

    #[tokio::test]
    async fn test_should_dedupe_duplicate_level_zero_communities() {
        let model = Arc::new(ScriptedRatingModel::new([(
            "ROOT",
            vec![r#"{"rating":5}"#],
        )]));
        let result = selector(
            config(3, true, 1, 1),
            vec![report(0, "ROOT")],
            vec![community(0, 0, -1, &[]), community(0, 0, -1, &[])],
            model.clone(),
            2,
        )
        .select("question")
        .await
        .expect("deduplicated roots");
        assert_eq!(result.ratings.len(), 1);
        assert_eq!(result.usage.llm_calls, 1);
        assert_eq!(model.requests.lock().expect("requests").len(), 1);
    }

    #[tokio::test]
    async fn test_should_skip_missing_child_report_and_stop_at_max_level() {
        let model = Arc::new(ScriptedRatingModel::new([(
            "ROOT",
            vec![r#"{"rating":5}"#],
        )]));
        let result = selector(
            config(3, true, 1, 0),
            vec![report(0, "ROOT")],
            vec![community(0, 0, -1, &[1])],
            Arc::clone(&model),
            2,
        )
        .select("question")
        .await
        .expect("max-level selection");
        assert_eq!(result.reports[0].community_id, "0");
        assert_eq!(result.usage.llm_calls, 1);
        assert_eq!(model.requests.lock().expect("requests").len(), 1);
    }

    #[tokio::test]
    async fn test_should_skip_child_without_report() {
        let model = Arc::new(ScriptedRatingModel::new([(
            "ROOT",
            vec![r#"{"rating":5}"#],
        )]));
        let result = selector(
            config(3, true, 1, 1),
            vec![report(0, "ROOT")],
            vec![community(0, 0, -1, &[99])],
            model,
            1,
        )
        .select("question")
        .await
        .expect("missing child report");
        assert_eq!(result.usage.llm_calls, 1);
        assert_eq!(result.reports.len(), 1);
    }

    #[tokio::test]
    async fn test_should_rate_report_backed_child_without_community_metadata() {
        let model = Arc::new(ScriptedRatingModel::new([
            ("ROOT", vec![r#"{"rating":5}"#]),
            ("ORPHAN", vec![r#"{"rating":5}"#]),
        ]));
        let result = selector(
            config(3, true, 1, 2),
            vec![report(0, "ROOT"), report(9, "ORPHAN")],
            vec![community(0, 0, -1, &[9])],
            model,
            2,
        )
        .select("question")
        .await
        .expect("missing metadata selection");
        assert_eq!(
            result
                .reports
                .iter()
                .map(|report| report.community_id.as_str())
                .collect::<Vec<_>>(),
            ["0", "9"]
        );
        assert_eq!(result.ratings[1].level, 1);
    }

    #[tokio::test]
    async fn test_should_use_summary_when_configured() {
        let model = Arc::new(ScriptedRatingModel::new([(
            "SUMMARY-SUMMARY_ONLY",
            vec![r#"{"rating":5}"#],
        )]));
        let mut dynamic_config = config(3, true, 1, 0);
        dynamic_config.dynamic_search_use_summary = true;
        let result = selector(
            dynamic_config,
            vec![report(0, "SUMMARY_ONLY")],
            vec![community(0, 0, -1, &[])],
            model,
            1,
        )
        .select("question")
        .await
        .expect("summary selection");
        assert_eq!(result.reports.len(), 1);
    }

    #[tokio::test]
    async fn test_should_bound_repeated_rating_concurrency_and_preserve_queue_order() {
        let model = Arc::new(ScriptedRatingModel::new([
            ("ROOT-A", vec![r#"{"rating":4}"#, r#"{"rating":2}"#]),
            ("ROOT-B", vec![r#"{"rating":3}"#, r#"{"rating":3}"#]),
        ]));
        let result = selector(
            config(2, true, 2, 0),
            vec![report(0, "ROOT-A"), report(1, "ROOT-B")],
            vec![community(0, 0, -1, &[]), community(1, 0, -1, &[])],
            Arc::clone(&model),
            2,
        )
        .select("question")
        .await
        .expect("concurrent selection");
        assert_eq!(model.max_in_flight.load(Ordering::SeqCst), 2);
        assert_eq!(result.usage.llm_calls, 4);
        assert_eq!(
            result
                .reports
                .iter()
                .map(|report| report.community_id.as_str())
                .collect::<Vec<_>>(),
            ["0", "1"]
        );
        assert_eq!(
            result
                .ratings
                .iter()
                .map(|rating| (
                    rating.community_id.as_str(),
                    rating.selected_rating,
                    rating.repeated_ratings.clone(),
                ))
                .collect::<Vec<_>>(),
            vec![
                ("0", 2, vec![Value::from(4), Value::from(2)]),
                ("1", 3, vec![Value::from(3), Value::from(3)]),
            ]
        );
        let requests = model.requests.lock().expect("requests");
        assert!(requests.iter().all(|request| request.stream == Some(false)));
        assert!(requests.iter().all(|request| {
            request.response_format == Some(serde_json::json!({"type": "json_object"}))
                && request.temperature == Some(0.1)
                && request.seed == Some(7)
        }));
    }

    #[tokio::test]
    async fn test_should_return_typed_error_without_report_backed_level_zero() {
        let model = Arc::new(ScriptedRatingModel::new([]));
        let error = selector(
            config(1, false, 1, 2),
            vec![report(1, "LEVEL1")],
            vec![community(1, 1, -1, &[])],
            model,
            1,
        )
        .select("question")
        .await
        .expect_err("missing level zero");
        assert!(matches!(
            error,
            QueryError::QueryContext {
                operation: "initialize dynamic community selection",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn test_should_propagate_dynamic_rating_provider_error() {
        let selector = DynamicCommunitySelection::new(
            config(1, false, 1, 1),
            vec![report(0, "ROOT")],
            vec![community(0, 0, -1, &[])],
            Arc::new(FailingRatingModel),
            "rating-test".to_owned(),
            model_config(),
            Arc::new(WordTokenizer),
            1,
        );
        let error = selector
            .select("question")
            .await
            .expect_err("provider error");
        assert!(matches!(
            error,
            QueryError::QueryCompletion {
                operation: "complete dynamic community rating",
                ..
            }
        ));
    }
}
