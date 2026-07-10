//! Error types for LLM, tokenizer, and parser operations.

use thiserror::Error;

/// Result type used by `graphloom-llm`.
pub type Result<T> = std::result::Result<T, LlmError>;

/// Errors raised by `graphloom-llm`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LlmError {
    /// The model configuration is invalid.
    #[error("invalid model configuration for {model_instance}: {message}")]
    InvalidConfig {
        /// Model instance name.
        model_instance: String,
        /// Validation failure message.
        message: String,
    },

    /// The provider reported an API failure.
    #[error(
        "provider request failed for {model_instance} during {operation} after {attempts} \
         attempt(s): {source}"
    )]
    Provider {
        /// Model instance name.
        model_instance: String,
        /// Operation name.
        operation: &'static str,
        /// Number of attempts made.
        attempts: u32,
        /// Provider request id when available.
        request_id: Option<String>,
        /// Original provider error.
        #[source]
        source: Box<async_openai::error::OpenAIError>,
    },

    /// The provider request timed out.
    #[error(
        "provider request timed out for {model_instance} during {operation} after {attempts} \
         attempt(s)"
    )]
    Timeout {
        /// Model instance name.
        model_instance: String,
        /// Operation name.
        operation: &'static str,
        /// Number of attempts made.
        attempts: u32,
    },

    /// A model response did not contain the expected content.
    #[error("invalid model response for {model_instance} during {operation}: {message}")]
    InvalidResponse {
        /// Model instance name.
        model_instance: String,
        /// Operation name.
        operation: &'static str,
        /// Validation failure message.
        message: String,
    },

    /// Tokenization failed.
    #[error("tokenizer {encoding_model} failed: {message}")]
    Tokenizer {
        /// Encoding model name.
        encoding_model: String,
        /// Failure message.
        message: String,
    },

    /// LLM output parsing failed.
    #[error("failed to parse {kind}: {message}")]
    Parse {
        /// Parser kind.
        kind: &'static str,
        /// Failure message.
        message: String,
    },
}
