//! Public Query results, usage accounting, and stream events.

use std::{collections::BTreeMap, pin::Pin, time::Duration};

use futures_util::Stream;
use polars_core::prelude::DataFrame;

use super::Result;

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
            llm_calls: categories.values().map(|value| value.llm_calls).sum(),
            prompt_tokens: categories.values().map(|value| value.prompt_tokens).sum(),
            output_tokens: categories.values().map(|value| value.output_tokens).sum(),
            categories,
        }
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
