//! Provider-neutral completion streaming wire types.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{CompletionResponse, CompletionUsage};

/// One provider completion stream chunk.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct CompletionChunk {
    /// Provider response identifier.
    pub id: Option<String>,
    /// Provider model name.
    pub model: Option<String>,
    /// Streamed choices.
    pub choices: Vec<CompletionChunkChoice>,
    /// Usage counters, usually present only on the final chunk.
    pub usage: Option<CompletionUsage>,
    /// Provider extension fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl CompletionChunk {
    /// Construct a deterministic text chunk for tests and fallback streams.
    #[must_use]
    pub fn text_for_test(
        model: impl Into<String>,
        content: impl Into<String>,
        finish_reason: Option<String>,
    ) -> Self {
        Self {
            id: Some("mock-completion".to_owned()),
            model: Some(model.into()),
            choices: vec![CompletionChunkChoice {
                index: 0,
                delta: CompletionDelta {
                    content: Some(content.into()),
                    extra: BTreeMap::new(),
                },
                finish_reason,
                extra: BTreeMap::new(),
            }],
            usage: None,
            extra: BTreeMap::new(),
        }
    }

    /// Convert a complete response into the single-chunk custom-model fallback.
    #[must_use]
    pub fn from_response(response: CompletionResponse) -> Self {
        Self {
            id: Some(response.id),
            model: Some(response.model),
            choices: response
                .choices
                .into_iter()
                .map(|choice| CompletionChunkChoice {
                    index: choice.index,
                    delta: CompletionDelta {
                        content: choice.message.content,
                        extra: BTreeMap::new(),
                    },
                    finish_reason: choice.finish_reason,
                    extra: choice.extra,
                })
                .collect(),
            usage: response.usage,
            extra: response.extra,
        }
    }
}

/// One choice in a completion stream chunk.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct CompletionChunkChoice {
    /// Choice index.
    pub index: u32,
    /// Incremental assistant delta.
    pub delta: CompletionDelta,
    /// Provider finish reason.
    pub finish_reason: Option<String>,
    /// Provider extension fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Incremental assistant content.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct CompletionDelta {
    /// Text emitted by this chunk, if any.
    pub content: Option<String>,
    /// Provider extension fields such as role or refusal metadata.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
