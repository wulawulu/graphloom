//! LLM, tokenizer, prompt, and parser contracts for `GraphLoom`.
//!
//! The public API mirrors Microsoft `GraphRAG`'s LLM bridge shape while keeping
//! provider-specific `async-openai` types private to this crate.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

mod cache_key;
mod error;
mod mock;
mod model;
mod openai;
mod parser;
mod prompt;
mod tokenizer;

#[cfg(test)]
mod tests;

pub use cache_key::{completion_cache_key, embedding_cache_key, graphrag_cache_key};
pub use error::{LlmError, Result};
pub use mock::{MockCompletionModel, MockEmbeddingModel};
pub use model::{
    ChatMessage, ChatRole, CompletionModel, CompletionRequest, CompletionResponse, EmbeddingModel,
    EmbeddingRequest, EmbeddingResponse, ModelConfig, Usage,
};
pub use openai::{OpenAiCompletionModel, OpenAiEmbeddingModel};
pub use parser::{
    ClaimRecord, CommunityReport, EntityRecord, GraphExtraction, RelationshipRecord,
    extract_json_object, parse_claim_tuples, parse_community_report, parse_graph_tuples,
    try_parse_json_object,
};
pub use prompt::{DefaultPrompt, PromptLoader};
pub use tokenizer::{TiktokenTokenizer, Tokenizer};
