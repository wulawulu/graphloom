//! Deterministic cache keys matching GraphRAG's semantic input selection.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{CompletionRequest, EmbeddingRequest, ModelConfig, Result};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompletionKey<'a> {
    version: u32,
    model_instance: &'a str,
    model: &'a str,
    messages: &'a [crate::ChatMessage],
    temperature: Option<f32>,
    top_p: Option<f32>,
    max_tokens: Option<u32>,
    response_format: &'a Option<String>,
    cache_namespace: &'a Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EmbeddingKey<'a> {
    version: u32,
    model_instance: &'a str,
    model: &'a str,
    input: &'a [String],
    dimensions: Option<u32>,
    cache_namespace: &'a Option<String>,
}

const CACHE_VERSION: u32 = 4;

/// Create a deterministic completion cache key.
///
/// # Errors
///
/// Returns an error if the key payload cannot be serialized.
pub fn completion_cache_key(
    model_instance: &str,
    config: &ModelConfig,
    request: &CompletionRequest,
) -> Result<String> {
    let payload = CompletionKey {
        version: CACHE_VERSION,
        model_instance,
        model: &config.model,
        messages: &request.messages,
        temperature: request.temperature,
        top_p: request.top_p,
        max_tokens: request.max_tokens,
        response_format: &request.response_format,
        cache_namespace: &request.cache_namespace,
    };
    hash_payload("completion", &payload)
}

/// Create a deterministic embedding cache key.
///
/// # Errors
///
/// Returns an error if the key payload cannot be serialized.
pub fn embedding_cache_key(
    model_instance: &str,
    config: &ModelConfig,
    request: &EmbeddingRequest,
) -> Result<String> {
    let payload = EmbeddingKey {
        version: CACHE_VERSION,
        model_instance,
        model: &config.model,
        input: &request.input,
        dimensions: request.dimensions,
        cache_namespace: &request.cache_namespace,
    };
    hash_payload("embedding", &payload)
}

fn hash_payload<T>(prefix: &str, payload: &T) -> Result<String>
where
    T: Serialize,
{
    let serialized = serde_json::to_vec(payload).map_err(|source| crate::LlmError::Parse {
        kind: "cache key",
        message: source.to_string(),
    })?;
    let hash = Sha256::digest(serialized);
    Ok(format!("{prefix}_{}_v{CACHE_VERSION}", to_hex(&hash)))
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}
