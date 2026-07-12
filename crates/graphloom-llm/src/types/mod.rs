//! Provider-neutral canonical model wire types.

mod completion;
mod embedding;
mod metadata;
mod request;

pub use completion::{CompletionChoice, CompletionMessage, CompletionResponse, CompletionUsage};
pub use embedding::{EmbeddingData, EmbeddingResponse, EmbeddingUsage};
pub use metadata::{CacheStatus, ModelCallMetadata};
pub use request::{ChatMessage, ChatRole, CompletionRequest, EmbeddingRequest, MessageContent};
