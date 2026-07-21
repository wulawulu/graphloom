//! Public Query results, usage accounting, and stream events.

use std::{collections::BTreeMap, ops::AddAssign, pin::Pin, time::Duration};

use futures_util::Stream;
use graphloom_llm::{ChatMessage, Tokenizer};
use polars_core::prelude::DataFrame;

use super::{QueryError, Result, SearchMethod};

/// Query context text supplied to completion models.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum QueryContextText {
    /// No context text.
    #[default]
    Empty,
    /// One context string.
    Text(String),
    /// Ordered map/reduce context batches.
    Batches(Vec<String>),
    /// Named context strings.
    Named(BTreeMap<String, String>),
    /// Named nested context values.
    Composite(BTreeMap<String, QueryContextText>),
}

/// Query context records used to construct text.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum QueryContextRecords {
    /// No context records.
    #[default]
    Empty,
    /// Named logical tables.
    Tables(BTreeMap<String, DataFrame>),
    /// Ordered context batches.
    Batches(Vec<DataFrame>),
    /// Named nested records.
    Named(BTreeMap<String, QueryContextRecords>),
}

/// Context returned by a Query.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct QueryContext {
    /// Exact text supplied to the system prompt.
    pub text: QueryContextText,
    /// Typed records used to build the context.
    pub records: QueryContextRecords,
}

/// Usage for one Query operation category.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueryUsageCategory {
    /// Model calls in this category.
    pub llm_calls: usize,
    /// Input/prompt tokens.
    pub prompt_tokens: usize,
    /// Generated tokens.
    pub output_tokens: usize,
}

impl QueryUsageCategory {
    /// Merge another category using saturating counters.
    pub fn saturating_add_assign(&mut self, other: Self) {
        self.llm_calls = self.llm_calls.saturating_add(other.llm_calls);
        self.prompt_tokens = self.prompt_tokens.saturating_add(other.prompt_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
    }
}

impl AddAssign for QueryUsageCategory {
    fn add_assign(&mut self, rhs: Self) {
        self.saturating_add_assign(rhs);
    }
}

/// Aggregate Query usage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueryUsage {
    /// Total model calls.
    pub llm_calls: usize,
    /// Total input/prompt tokens.
    pub prompt_tokens: usize,
    /// Total generated tokens.
    pub output_tokens: usize,
    /// Usage by semantic operation.
    pub categories: BTreeMap<String, QueryUsageCategory>,
}

impl QueryUsage {
    pub(crate) fn from_categories(categories: BTreeMap<String, QueryUsageCategory>) -> Self {
        Self {
            llm_calls: categories
                .values()
                .fold(0, |total, value| total.saturating_add(value.llm_calls)),
            prompt_tokens: categories
                .values()
                .fold(0, |total, value| total.saturating_add(value.prompt_tokens)),
            output_tokens: categories
                .values()
                .fold(0, |total, value| total.saturating_add(value.output_tokens)),
            categories,
        }
    }
}

pub(crate) fn count_completion_input(
    tokenizer: &dyn Tokenizer,
    messages: &[ChatMessage],
    method: SearchMethod,
    operation: &'static str,
) -> Result<usize> {
    messages.iter().try_fold(0_usize, |total, message| {
        tokenizer
            .count(message.content.as_str())
            .map(|tokens| total.saturating_add(tokens))
            .map_err(|source| QueryError::QueryContext {
                method,
                operation,
                message: source.to_string(),
            })
    })
}

pub(crate) fn resolve_embedding_prompt_tokens(
    provider_prompt_tokens: u64,
    input: &str,
    tokenizer: &dyn Tokenizer,
    method: SearchMethod,
    operation: &'static str,
    model_id: &str,
) -> Result<usize> {
    let provider_prompt_tokens = usize::try_from(provider_prompt_tokens).unwrap_or(usize::MAX);
    if provider_prompt_tokens > 0 {
        return Ok(provider_prompt_tokens);
    }
    tokenizer
        .count(input)
        .map_err(|source| QueryError::QueryEmbedding {
            method,
            operation,
            model: model_id.to_owned(),
            source: Box::new(source),
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use graphloom_llm::{LlmError, Tokenizer};

    use super::{QueryUsage, QueryUsageCategory, resolve_embedding_prompt_tokens};
    use crate::query::SearchMethod;

    #[derive(Debug)]
    struct ByteTokenizer;

    impl Tokenizer for ByteTokenizer {
        fn encode(&self, text: &str) -> graphloom_llm::Result<Vec<u32>> {
            Ok(text.bytes().map(u32::from).collect())
        }

        fn decode(&self, tokens: &[u32]) -> graphloom_llm::Result<String> {
            let bytes = tokens
                .iter()
                .map(|token| {
                    u8::try_from(*token).map_err(|source| LlmError::Tokenizer {
                        encoding_model: "bytes".to_owned(),
                        message: source.to_string(),
                    })
                })
                .collect::<graphloom_llm::Result<Vec<_>>>()?;
            String::from_utf8(bytes).map_err(|source| LlmError::Tokenizer {
                encoding_model: "bytes".to_owned(),
                message: source.to_string(),
            })
        }
    }

    #[test]
    fn test_should_resolve_embedding_usage_with_provider_or_tokenizer_fallback() {
        assert_eq!(
            resolve_embedding_prompt_tokens(
                9,
                "ignored",
                &ByteTokenizer,
                SearchMethod::Basic,
                "test provider usage",
                "embedding",
            )
            .expect("provider usage"),
            9
        );
        assert_eq!(
            resolve_embedding_prompt_tokens(
                0,
                "fallback",
                &ByteTokenizer,
                SearchMethod::Drift,
                "test fallback usage",
                "embedding",
            )
            .expect("fallback usage"),
            "fallback".len()
        );
    }

    #[test]
    fn test_should_saturate_category_merges_and_top_level_totals() {
        let mut category = QueryUsageCategory {
            llm_calls: usize::MAX,
            prompt_tokens: usize::MAX - 1,
            output_tokens: 3,
        };
        category += QueryUsageCategory {
            llm_calls: 1,
            prompt_tokens: 10,
            output_tokens: usize::MAX,
        };
        assert_eq!(
            category,
            QueryUsageCategory {
                llm_calls: usize::MAX,
                prompt_tokens: usize::MAX,
                output_tokens: usize::MAX,
            }
        );

        let usage = QueryUsage::from_categories(BTreeMap::from([
            ("first".to_owned(), category),
            (
                "second".to_owned(),
                QueryUsageCategory {
                    llm_calls: 1,
                    prompt_tokens: 1,
                    output_tokens: 1,
                },
            ),
        ]));
        assert_eq!(usage.llm_calls, usize::MAX);
        assert_eq!(usage.prompt_tokens, usize::MAX);
        assert_eq!(usage.output_tokens, usize::MAX);
    }
}

/// Successful Query result.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct QueryResult {
    /// Final generated answer.
    pub response: String,
    /// Exact context text and records.
    pub context: QueryContext,
    /// Wall-clock elapsed time.
    pub elapsed: Duration,
    /// Model usage.
    pub usage: QueryUsage,
}

/// Event emitted by a streaming Query.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum QueryEvent {
    /// Context construction completed.
    Context(QueryContext),
    /// Provider text delta.
    Token(String),
    /// Query completed with its full summary.
    Completed(QueryResult),
}

/// Provider-neutral Query event stream.
pub type QueryEventStream = Pin<Box<dyn Stream<Item = Result<QueryEvent>> + Send>>;
