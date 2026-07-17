//! Typed Query errors.

use thiserror::Error;

use super::SearchMethod;

/// Query result type.
pub type Result<T> = std::result::Result<T, QueryError>;

/// Detailed column-level context for an invalid Query table.
#[derive(Debug)]
#[non_exhaustive]
pub struct QueryTableErrorDetails {
    /// Table name.
    pub table: &'static str,
    /// Column name.
    pub column: String,
    /// Expected logical type.
    pub expected: &'static str,
    /// Actual physical type.
    pub actual: String,
    /// Optional row suffix.
    pub row: String,
    /// Additional validation details.
    pub message: String,
    /// Underlying table I/O error, when the failure occurred before adaptation.
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl std::error::Error for QueryTableErrorDetails {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

impl std::fmt::Display for QueryTableErrorDetails {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "table {}: column {}, expected {}, actual {}{}: {}",
            self.table, self.column, self.expected, self.actual, self.row, self.message
        )
    }
}

/// Errors produced by Query configuration, loading, retrieval, and generation.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum QueryError {
    /// Method-specific configuration is invalid.
    #[error("invalid {method} query configuration during {operation}: {message}")]
    InvalidQueryConfig {
        /// Active method.
        method: SearchMethod,
        /// Operation that failed.
        operation: &'static str,
        /// Validation details.
        message: String,
    },
    /// A required Parquet table is missing.
    #[error("{method} query requires table {table} during {operation}")]
    MissingQueryTable {
        /// Active method.
        method: SearchMethod,
        /// Operation that failed.
        operation: &'static str,
        /// Missing table.
        table: &'static str,
    },
    /// A Query table has an incompatible field or value.
    #[error("invalid {details} for {method} during {operation}")]
    InvalidQueryTable {
        /// Active method.
        method: SearchMethod,
        /// Operation that failed.
        operation: &'static str,
        /// Boxed column details keep the public error enum compact.
        #[source]
        details: Box<QueryTableErrorDetails>,
    },
    /// A required vector index is missing.
    #[error("{method} query requires vector index {index} during {operation}: {source}")]
    MissingVectorIndex {
        /// Active method.
        method: SearchMethod,
        /// Operation that failed.
        operation: &'static str,
        /// Index name.
        index: String,
        /// Vector source error.
        #[source]
        source: Box<graphloom_vectors::VectorError>,
    },
    /// A vector index or ANN result is invalid.
    #[error("invalid vector index {index} for {method} during {operation}: {source}")]
    InvalidVectorIndex {
        /// Active method.
        method: SearchMethod,
        /// Operation that failed.
        operation: &'static str,
        /// Index name.
        index: String,
        /// Vector source error.
        #[source]
        source: Box<graphloom_vectors::VectorError>,
    },
    /// Prompt loading or rendering failed.
    #[error("query prompt {prompt} failed for {method} during {operation}: {source}")]
    QueryPrompt {
        /// Active method.
        method: SearchMethod,
        /// Operation that failed.
        operation: &'static str,
        /// Prompt identifier.
        prompt: &'static str,
        /// Prompt source error.
        #[source]
        source: Box<crate::GraphLoomError>,
    },
    /// Query embedding failed.
    #[error("query embedding model {model} failed for {method} during {operation}: {source}")]
    QueryEmbedding {
        /// Active method.
        method: SearchMethod,
        /// Operation that failed.
        operation: &'static str,
        /// Model identifier.
        model: String,
        /// LLM source error.
        #[source]
        source: Box<graphloom_llm::LlmError>,
    },
    /// Query completion failed.
    #[error("query completion model {model} failed for {method} during {operation}: {source}")]
    QueryCompletion {
        /// Active method.
        method: SearchMethod,
        /// Operation that failed.
        operation: &'static str,
        /// Model identifier.
        model: String,
        /// LLM source error.
        #[source]
        source: Box<graphloom_llm::LlmError>,
    },
    /// Structured Query output parsing failed.
    #[error("query parse failed for {method} during {operation}: {message}")]
    QueryParse {
        /// Active method.
        method: SearchMethod,
        /// Operation that failed.
        operation: &'static str,
        /// Parse details.
        message: String,
    },
    /// Context construction failed.
    #[error("query context failed for {method} during {operation}: {message}")]
    QueryContext {
        /// Active method.
        method: SearchMethod,
        /// Operation that failed.
        operation: &'static str,
        /// Context details.
        message: String,
    },
    /// Runtime assembly or table I/O failed.
    #[error("query runtime failed for {method} during {operation}: {source}")]
    QueryRuntime {
        /// Active method.
        method: SearchMethod,
        /// Operation that failed.
        operation: &'static str,
        /// Runtime source error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The method is unknown or not available in the current implementation step.
    #[error("query method failure during {operation}: {message}")]
    QueryMethod {
        /// Parsed method, when known.
        method: Option<SearchMethod>,
        /// Operation that failed.
        operation: &'static str,
        /// Failure details.
        message: String,
    },
}
