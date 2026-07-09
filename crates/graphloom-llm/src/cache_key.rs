//! Cache keys compatible with `GraphRAG`'s `graphrag_cache` package.

use std::collections::BTreeMap;

use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};

use crate::{CompletionRequest, EmbeddingRequest, ModelConfig, Result};

const CACHE_VERSION: u32 = 4;
const EXCLUDED_KEYS: &[&str] = &[
    "metrics",
    "stream",
    "stream_options",
    "mock_response",
    "timeout",
    "base_url",
    "api_base",
    "api_version",
    "api_key",
    "azure_ad_token_provider",
    "drop_params",
];

/// Create a GraphRAG-compatible completion cache key.
///
/// # Errors
///
/// Returns an error if the key payload cannot be represented.
pub fn completion_cache_key(
    _model_instance: &str,
    _config: &ModelConfig,
    request: &CompletionRequest,
) -> Result<String> {
    let mut payload = Map::new();
    payload.insert(
        "messages".to_owned(),
        serde_json::to_value(&request.messages).map_err(cache_key_error)?,
    );
    payload.insert(
        "response_format".to_owned(),
        request
            .response_format
            .as_ref()
            .map_or(Value::Null, |response_format| {
                Value::Object(Map::from_iter([(
                    "type".to_owned(),
                    Value::String(response_format.clone()),
                )]))
            }),
    );
    if let Some(temperature) = request.temperature {
        payload.insert("temperature".to_owned(), number_from_f32(temperature)?);
    }
    if let Some(top_p) = request.top_p {
        payload.insert("top_p".to_owned(), number_from_f32(top_p)?);
    }
    if let Some(max_tokens) = request.max_tokens {
        payload.insert(
            "max_tokens".to_owned(),
            Value::Number(Number::from(max_tokens)),
        );
    }
    graphrag_cache_key(&Value::Object(payload))
}

/// Create a GraphRAG-compatible embedding cache key.
///
/// The key intentionally covers only the embedding request payload (`input` and
/// optional `dimensions`). Model-instance isolation belongs to the caller's
/// cache namespace, for example `Cache::child(model_instance_name)`.
///
/// # Errors
///
/// Returns an error if the key payload cannot be represented.
pub fn embedding_request_cache_key(request: &EmbeddingRequest) -> Result<String> {
    let mut payload = Map::new();
    payload.insert(
        "input".to_owned(),
        Value::Array(
            request
                .input
                .iter()
                .map(|input| Value::String(input.clone()))
                .collect(),
        ),
    );
    if let Some(dimensions) = request.dimensions {
        payload.insert(
            "dimensions".to_owned(),
            Value::Number(Number::from(dimensions)),
        );
    }
    graphrag_cache_key(&Value::Object(payload))
}

/// Create a GraphRAG-compatible embedding cache key.
///
/// Model instance arguments are retained for compatibility. Namespace
/// isolation is handled by the cache provider, so this delegates to
/// [`embedding_request_cache_key`].
///
/// # Errors
///
/// Returns an error if the key payload cannot be represented.
pub fn embedding_cache_key(
    _model_instance: &str,
    _config: &ModelConfig,
    request: &EmbeddingRequest,
) -> Result<String> {
    embedding_request_cache_key(request)
}

/// Create a GraphRAG-compatible cache key from raw model call kwargs.
///
/// # Errors
///
/// Returns an error if the key payload cannot be represented.
pub fn graphrag_cache_key(input_args: &Value) -> Result<String> {
    let filtered = filter_cache_parameters(input_args);
    let serialized = py_yaml_dump(&filtered)?;
    let hash = Sha256::digest(serialized.as_bytes());
    Ok(format!("{}_v{CACHE_VERSION}", to_hex(&hash)))
}

fn cache_key_error(source: impl std::fmt::Display) -> crate::LlmError {
    crate::LlmError::Parse {
        kind: "cache key",
        message: source.to_string(),
    }
}

fn number_from_f32(value: f32) -> Result<Value> {
    Number::from_f64(f64::from(value))
        .map(Value::Number)
        .ok_or_else(|| crate::LlmError::Parse {
            kind: "cache key",
            message: format!("non-finite float {value}"),
        })
}

fn filter_cache_parameters(input_args: &Value) -> Value {
    match input_args {
        Value::Object(object) => {
            let filtered = object
                .iter()
                .filter(|(key, _)| !EXCLUDED_KEYS.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<Map<_, _>>();
            Value::Object(filtered)
        }
        value => value.clone(),
    }
}

fn py_yaml_dump(value: &Value) -> Result<String> {
    let mut output = String::new();
    write_yaml_value(value, 0, &mut output)?;
    Ok(output)
}

fn write_yaml_value(value: &Value, indent: usize, output: &mut String) -> Result<()> {
    match value {
        Value::Object(object) => write_yaml_mapping(object, indent, output),
        Value::Array(values) => write_yaml_sequence(values, indent, output),
        value => {
            output.push_str(&format_scalar(value, indent + 2)?);
            output.push('\n');
            Ok(())
        }
    }
}

fn write_yaml_mapping(
    object: &Map<String, Value>,
    indent: usize,
    output: &mut String,
) -> Result<()> {
    if object.is_empty() {
        output.push_str("{}\n");
        return Ok(());
    }

    let sorted = object.iter().collect::<BTreeMap<&String, &Value>>();
    for (key, value) in sorted {
        output.push_str(&" ".repeat(indent));
        output.push_str(key);
        match value {
            Value::Object(object) if object.is_empty() => output.push_str(": {}\n"),
            Value::Array(values) if values.is_empty() => output.push_str(": []\n"),
            Value::Object(_) => {
                output.push_str(":\n");
                write_yaml_value(value, indent + 2, output)?;
            }
            Value::Array(_) => {
                output.push_str(":\n");
                write_yaml_value(value, indent, output)?;
            }
            scalar => {
                output.push_str(": ");
                output.push_str(&format_scalar(scalar, indent + 2)?);
                output.push('\n');
            }
        }
    }
    Ok(())
}

fn write_yaml_sequence(values: &[Value], indent: usize, output: &mut String) -> Result<()> {
    if values.is_empty() {
        output.push_str(&" ".repeat(indent));
        output.push_str("[]\n");
        return Ok(());
    }

    for value in values {
        match value {
            Value::Object(object) if object.is_empty() => {
                output.push_str(&" ".repeat(indent));
                output.push_str("- {}\n");
            }
            Value::Object(object) => write_yaml_sequence_mapping(object, indent, output)?,
            Value::Array(values) if values.is_empty() => {
                output.push_str(&" ".repeat(indent));
                output.push_str("- []\n");
            }
            Value::Array(values) => {
                output.push_str(&" ".repeat(indent));
                output.push_str("-\n");
                write_yaml_sequence(values, indent + 2, output)?;
            }
            scalar => {
                output.push_str(&" ".repeat(indent));
                output.push_str("- ");
                output.push_str(&format_scalar(scalar, indent + 2)?);
                output.push('\n');
            }
        }
    }
    Ok(())
}

fn write_yaml_sequence_mapping(
    object: &Map<String, Value>,
    indent: usize,
    output: &mut String,
) -> Result<()> {
    let sorted = object.iter().collect::<BTreeMap<&String, &Value>>();
    let mut first = true;
    for (key, value) in sorted {
        if first {
            output.push_str(&" ".repeat(indent));
            output.push_str("- ");
            first = false;
        } else {
            output.push_str(&" ".repeat(indent + 2));
        }
        output.push_str(key);
        match value {
            Value::Object(object) if object.is_empty() => output.push_str(": {}\n"),
            Value::Array(values) if values.is_empty() => output.push_str(": []\n"),
            Value::Object(_) => {
                output.push_str(":\n");
                write_yaml_value(value, indent + 4, output)?;
            }
            Value::Array(_) => {
                output.push_str(":\n");
                write_yaml_value(value, indent + 2, output)?;
            }
            scalar => {
                output.push_str(": ");
                output.push_str(&format_scalar(scalar, indent + 4)?);
                output.push('\n');
            }
        }
    }
    Ok(())
}

fn format_scalar(value: &Value, continuation_indent: usize) -> Result<String> {
    match value {
        Value::Null => Ok("null".to_owned()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        Value::String(value) => Ok(format_string(value, continuation_indent)),
        Value::Array(_) | Value::Object(_) => Err(crate::LlmError::Parse {
            kind: "cache key",
            message: "nested value reached scalar formatter".to_owned(),
        }),
    }
}

fn format_string(value: &str, continuation_indent: usize) -> String {
    if value.is_empty() {
        return "''".to_owned();
    }
    if needs_double_quoted_string(value) {
        return double_quoted_string(value);
    }
    if value.contains('\n') {
        return single_quoted_multiline(value, continuation_indent);
    }
    if needs_single_quoted_string(value) {
        return format!("'{}'", value.replace('\'', "''"));
    }
    value.to_owned()
}

fn needs_double_quoted_string(value: &str) -> bool {
    value.contains('\t')
        || value.contains('\r')
        || value.as_bytes().windows(2).any(|window| window == b"\n ")
}

fn needs_single_quoted_string(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    value.starts_with(' ')
        || value.ends_with(' ')
        || value.contains(": ")
        || matches!(
            lower.as_str(),
            "true" | "false" | "null" | "none" | "yes" | "no" | "on" | "off"
        )
        || value.parse::<f64>().is_ok()
        || value.starts_with('{')
        || value.starts_with('[')
}

fn double_quoted_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

fn single_quoted_multiline(value: &str, continuation_indent: usize) -> String {
    let mut output = String::new();
    output.push('\'');
    for (index, line) in value.split('\n').enumerate() {
        if index == 0 {
            output.push_str(&line.replace('\'', "''"));
        } else {
            output.push_str("\n\n");
            if !line.is_empty() {
                output.push_str(&" ".repeat(continuation_indent));
                output.push_str(&line.replace('\'', "''"));
            }
        }
    }
    output.push('\'');
    output
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
