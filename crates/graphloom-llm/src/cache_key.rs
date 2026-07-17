//! Cache keys compatible with `GraphRAG`'s `graphrag_cache` package.

use std::{collections::BTreeMap, sync::LazyLock};

use regex::Regex;
use serde_json::{Map, Value};
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
const PYYAML_WIDTH: usize = 80;
const PYYAML_SIMPLE_KEY_LIMIT: usize = 128;
const PYYAML_STRING_TAG_LENGTH: usize = 5;
const YAML_INDENT_WIDTH: usize = 2;

#[derive(Debug, Clone, Copy)]
struct ScalarContext {
    initial_column: usize,
    continuation_indent: usize,
    width: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScalarStyle {
    Plain,
    SingleQuoted,
    SingleQuotedMultiline,
    DoubleQuoted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MappingKeyStyle {
    Simple,
    Explicit,
}

// PyYAML 6.0.3 yaml.resolver.Resolver patterns for JSON-representable implicit scalar types.
static PYYAML_IMPLICIT_SCALAR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)\A(?:
                yes|Yes|YES|no|No|NO|true|True|TRUE|false|False|FALSE|on|On|ON|off|Off|OFF
                |~|null|Null|NULL
                |[-+]?0b[0-1_]+
                |[-+]?0[0-7_]+
                |[-+]?(?:0|[1-9][0-9_]*)
                |[-+]?0x[0-9a-fA-F_]+
                |[-+]?[1-9][0-9_]*(?::[0-5]?[0-9])+
                |[-+]?(?:[0-9][0-9_]*)\.[0-9_]*(?:[eE][-+][0-9]+)?
                |\.[0-9][0-9_]*(?:[eE][-+][0-9]+)?
                |[-+]?[0-9][0-9_]*(?::[0-5]?[0-9])+\.[0-9_]*
                |[-+]?\.(?:inf|Inf|INF)
                |\.(?:nan|NaN|NAN)
                |[0-9]{4}-[0-9]{2}-[0-9]{2}
                |[0-9]{4}-[0-9]{1,2}-[0-9]{1,2}
                    (?:[Tt]|[\x20\t]+)[0-9]{1,2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]*)?
                    (?:[\x20\t]*(?:Z|[-+][0-9]{1,2}(?::[0-9]{2})?))?
            )\z",
    )
    .expect("PyYAML resolver regex must compile")
});

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
    completion_request_cache_key(request)
}

/// Create a GraphRAG-compatible completion cache key from request kwargs.
///
/// # Errors
///
/// Returns an error if the request cannot be represented as JSON kwargs.
pub fn completion_request_cache_key(request: &CompletionRequest) -> Result<String> {
    request.validate()?;
    let mut kwargs = serde_json::to_value(request).map_err(cache_key_error)?;
    let object = kwargs
        .as_object_mut()
        .ok_or_else(|| crate::LlmError::Parse {
            kind: "cache key",
            message: "completion request did not serialize to an object".to_owned(),
        })?;
    object
        .entry("response_format".to_owned())
        .or_insert(Value::Null);
    graphrag_cache_key(&kwargs)
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
    request.validate()?;
    graphrag_cache_key(&serde_json::to_value(request).map_err(cache_key_error)?)
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
    let serialized = graphrag_cache_yaml(input_args)?;
    let hash = Sha256::digest(serialized.as_bytes());
    Ok(format!("{}_v{CACHE_VERSION}", to_hex(&hash)))
}

/// Serialize raw model kwargs exactly as `GraphRAG`'s `PyYAML` cache hasher does.
///
/// # Errors
///
/// Returns an error when the JSON value cannot be represented by the
/// GraphRAG-compatible emitter.
pub fn graphrag_cache_yaml(input_args: &Value) -> Result<String> {
    py_yaml_dump(&filter_cache_parameters(input_args))
}

fn cache_key_error(source: impl std::fmt::Display) -> crate::LlmError {
    crate::LlmError::Parse {
        kind: "cache key",
        message: source.to_string(),
    }
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
            output.push_str(&format_scalar(value, indent + 2, current_column(output))?);
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

    write_yaml_mapping_entries(object, indent, false, output)
}

fn write_yaml_mapping_entries(
    object: &Map<String, Value>,
    indent: usize,
    first_inline: bool,
    output: &mut String,
) -> Result<()> {
    let sorted = object.iter().collect::<BTreeMap<&String, &Value>>();
    for (index, (key, value)) in sorted.into_iter().enumerate() {
        if !(first_inline && index == 0) {
            output.push_str(&" ".repeat(indent));
        }
        write_yaml_mapping_entry(key, value, indent, output)?;
    }
    Ok(())
}

fn write_yaml_mapping_entry(
    key: &str,
    value: &Value,
    indent: usize,
    output: &mut String,
) -> Result<()> {
    match classify_mapping_key(key) {
        MappingKeyStyle::Simple => {
            let context = ScalarContext {
                initial_column: current_column(output),
                continuation_indent: indent.saturating_add(YAML_INDENT_WIDTH),
                width: usize::MAX,
            };
            output.push_str(&format_string_with_context(key, context));
            write_simple_mapping_value(value, indent, output)
        }
        MappingKeyStyle::Explicit => {
            output.push_str("? ");
            let context = ScalarContext {
                initial_column: current_column(output),
                continuation_indent: indent.saturating_add(YAML_INDENT_WIDTH),
                width: PYYAML_WIDTH,
            };
            output.push_str(&format_string_with_context(key, context));
            output.push('\n');
            output.push_str(&" ".repeat(indent));
            output.push(':');
            write_explicit_mapping_value(value, indent, output)
        }
    }
}

fn write_simple_mapping_value(value: &Value, indent: usize, output: &mut String) -> Result<()> {
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
            output.push_str(&format_scalar(scalar, indent + 2, current_column(output))?);
            output.push('\n');
        }
    }
    Ok(())
}

fn write_explicit_mapping_value(value: &Value, indent: usize, output: &mut String) -> Result<()> {
    match value {
        Value::Object(object) if object.is_empty() => output.push_str(" {}\n"),
        Value::Array(values) if values.is_empty() => output.push_str(" []\n"),
        Value::Object(object) => {
            output.push(' ');
            write_yaml_mapping_entries(object, indent + 2, true, output)?;
        }
        Value::Array(values) => {
            output.push(' ');
            write_yaml_sequence_with_first(values, indent + 2, true, output)?;
        }
        scalar => {
            output.push(' ');
            output.push_str(&format_scalar(scalar, indent + 2, current_column(output))?);
            output.push('\n');
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

    write_yaml_sequence_with_first(values, indent, false, output)
}

fn write_yaml_sequence_with_first(
    values: &[Value],
    indent: usize,
    first_inline: bool,
    output: &mut String,
) -> Result<()> {
    for (index, value) in values.iter().enumerate() {
        let inline = first_inline && index == 0;
        match value {
            Value::Object(object) if object.is_empty() => {
                write_sequence_prefix(indent, inline, output);
                output.push_str("- {}\n");
            }
            Value::Object(object) => write_yaml_sequence_mapping(object, indent, inline, output)?,
            Value::Array(values) if values.is_empty() => {
                write_sequence_prefix(indent, inline, output);
                output.push_str("- []\n");
            }
            Value::Array(values) => {
                let mut nested = String::new();
                write_yaml_sequence(values, indent + 2, &mut nested)?;
                write_sequence_prefix(indent, inline, output);
                output.push_str("- ");
                output.push_str(nested.trim_start_matches(' '));
            }
            scalar => {
                write_sequence_prefix(indent, inline, output);
                output.push_str("- ");
                output.push_str(&format_scalar(scalar, indent + 2, current_column(output))?);
                output.push('\n');
            }
        }
    }
    Ok(())
}

fn write_sequence_prefix(indent: usize, inline: bool, output: &mut String) {
    if !inline {
        output.push_str(&" ".repeat(indent));
    }
}

fn write_yaml_sequence_mapping(
    object: &Map<String, Value>,
    indent: usize,
    first_inline: bool,
    output: &mut String,
) -> Result<()> {
    let sorted = object.iter().collect::<BTreeMap<&String, &Value>>();
    for (index, (key, value)) in sorted.into_iter().enumerate() {
        if index == 0 {
            write_sequence_prefix(indent, first_inline, output);
            output.push_str("- ");
        } else {
            output.push_str(&" ".repeat(indent + 2));
        }
        write_yaml_mapping_entry(key, value, indent + 2, output)?;
    }
    Ok(())
}

fn format_scalar(
    value: &Value,
    continuation_indent: usize,
    current_column: usize,
) -> Result<String> {
    match value {
        Value::Null => Ok("null".to_owned()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(format_yaml_number(value)),
        Value::String(value) => Ok(format_string(value, continuation_indent, current_column)),
        Value::Array(_) | Value::Object(_) => Err(crate::LlmError::Parse {
            kind: "cache key",
            message: "nested value reached scalar formatter".to_owned(),
        }),
    }
}

fn format_string(value: &str, continuation_indent: usize, current_column: usize) -> String {
    format_string_with_context(
        value,
        ScalarContext {
            initial_column: current_column,
            continuation_indent,
            width: PYYAML_WIDTH,
        },
    )
}

fn format_string_with_context(value: &str, context: ScalarContext) -> String {
    match classify_string_scalar(value) {
        ScalarStyle::Plain => format_plain_scalar(value, context),
        ScalarStyle::SingleQuoted => format_single_quoted_scalar(value, context),
        ScalarStyle::SingleQuotedMultiline => format_single_quoted_multiline_scalar(value, context),
        ScalarStyle::DoubleQuoted => format_double_quoted_scalar(value, context),
    }
}

fn classify_mapping_key(key: &str) -> MappingKeyStyle {
    // PyYAML 6.0.3 `Emitter::check_simple_key` counts the scalar event plus the
    // prepared implicit `!!str` tag and requires the total to be strictly below 128.
    let event_length = key.chars().count().saturating_add(PYYAML_STRING_TAG_LENGTH);
    if !key.is_empty()
        && !key.chars().any(is_pyyaml_line_break)
        && event_length < PYYAML_SIMPLE_KEY_LIMIT
    {
        MappingKeyStyle::Simple
    } else {
        MappingKeyStyle::Explicit
    }
}

fn is_pyyaml_line_break(character: char) -> bool {
    matches!(character, '\n' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

fn classify_string_scalar(value: &str) -> ScalarStyle {
    if needs_double_quoted_string(value) {
        ScalarStyle::DoubleQuoted
    } else if value.contains('\n') {
        ScalarStyle::SingleQuotedMultiline
    } else if plain_scalar_is_allowed(value) {
        ScalarStyle::Plain
    } else {
        ScalarStyle::SingleQuoted
    }
}

fn format_plain_scalar(value: &str, context: ScalarContext) -> String {
    wrap_scalar_words(value, context, "", "")
}

fn format_single_quoted_scalar(value: &str, context: ScalarContext) -> String {
    wrap_scalar_words(&value.replace('\'', "''"), context, "'", "'")
}

fn format_single_quoted_multiline_scalar(value: &str, context: ScalarContext) -> String {
    let mut output = String::with_capacity(value.len().saturating_add(2));
    let mut characters = value.chars().peekable();
    let mut line = String::new();
    let mut first_line = true;

    loop {
        match characters.next() {
            Some('\n') => {
                let line_context = ScalarContext {
                    initial_column: if first_line {
                        context.initial_column
                    } else {
                        context.continuation_indent
                    },
                    ..context
                };
                output.push_str(&wrap_scalar_words(
                    &line.replace('\'', "''"),
                    line_context,
                    if first_line { "'" } else { "" },
                    "",
                ));
                line.clear();
                first_line = false;
                let mut original_newlines = 1usize;
                while characters.next_if_eq(&'\n').is_some() {
                    original_newlines = original_newlines.saturating_add(1);
                }
                write_original_newline_group(
                    &mut output,
                    original_newlines,
                    context.continuation_indent,
                );
            }
            Some(character) => line.push(character),
            None => {
                let line_context = ScalarContext {
                    initial_column: if first_line {
                        context.initial_column
                    } else {
                        context.continuation_indent
                    },
                    ..context
                };
                output.push_str(&wrap_scalar_words(
                    &line.replace('\'', "''"),
                    line_context,
                    if first_line { "'" } else { "" },
                    "'",
                ));
                return output;
            }
        }
    }
}

fn write_original_newline_group(output: &mut String, count: usize, continuation_indent: usize) {
    output.push_str(&"\n".repeat(count.saturating_add(1)));
    output.push_str(&" ".repeat(continuation_indent));
}

fn wrap_scalar_words(value: &str, context: ScalarContext, prefix: &str, suffix: &str) -> String {
    let mut output = String::with_capacity(value.len().saturating_add(prefix.len() + suffix.len()));
    output.push_str(prefix);
    let mut column = context
        .initial_column
        .saturating_add(prefix.chars().count());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character == ' ' && column > context.width && characters.peek().is_some() {
            output.push('\n');
            output.push_str(&" ".repeat(context.continuation_indent));
            column = context.continuation_indent;
        } else {
            output.push(character);
            column = column.saturating_add(1);
        }
    }
    output.push_str(suffix);
    output
}

fn needs_double_quoted_string(value: &str) -> bool {
    value
        .chars()
        .any(|character| character != '\n' && !(' '..='~').contains(&character))
        || value.as_bytes().windows(2).any(|window| window == b"\n ")
        || value.as_bytes().windows(2).any(|window| window == b" \n")
}

fn plain_scalar_is_allowed(value: &str) -> bool {
    !value.is_empty()
        && !value.chars().next().is_some_and(char::is_whitespace)
        && !value.chars().next_back().is_some_and(char::is_whitespace)
        && !starts_with_yaml_indicator(value)
        && !resembles_yaml_implicit_scalar(value)
        && !value.contains(": ")
        && !value.contains(" #")
        && !value.ends_with(':')
}

fn starts_with_yaml_indicator(value: &str) -> bool {
    const ALWAYS_QUOTED: &[char] = &[
        ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`',
    ];
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    ALWAYS_QUOTED.contains(&first)
        || (matches!(first, '-' | '?' | ':') && characters.next().is_none_or(char::is_whitespace))
        || matches!(value, "---" | "...")
        || value.starts_with("--- ")
        || value.starts_with("... ")
}

fn resembles_yaml_implicit_scalar(value: &str) -> bool {
    PYYAML_IMPLICIT_SCALAR.is_match(value) || matches!(value, "<<" | "=")
}

fn format_yaml_number(value: &serde_json::Number) -> String {
    let rendered = value.to_string();
    let Some(exponent_index) = rendered.find(['e', 'E']) else {
        return rendered;
    };
    let exponent = rendered[exponent_index + 1..].parse::<i32>();
    let Ok(exponent) = exponent else {
        return rendered;
    };
    if (0..=15).contains(&exponent)
        && let Some(float) = value.as_f64()
    {
        let decimal = float.to_string();
        return if decimal.contains('.') {
            decimal
        } else {
            format!("{decimal}.0")
        };
    }
    let mantissa = &rendered[..exponent_index];
    let mantissa = if mantissa.contains('.') {
        mantissa.to_owned()
    } else {
        format!("{mantissa}.0")
    };
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{mantissa}e{sign}{:02}", exponent.unsigned_abs())
}

fn format_double_quoted_scalar(value: &str, context: ScalarContext) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(value.len() + 2);
    let mut column = context.initial_column;
    output.push('"');
    column = column.saturating_add(1);
    let mut start = 0usize;
    let mut end = 0usize;
    while end <= characters.len() {
        let character = characters.get(end).copied();
        if character.is_none_or(needs_yaml_escape) {
            if start < end {
                let data = characters[start..end].iter().collect::<String>();
                column = column.saturating_add(data.chars().count());
                output.push_str(&data);
                start = end;
            }
            if let Some(character) = character {
                let escaped = yaml_escape(character);
                column = column.saturating_add(escaped.len());
                output.push_str(&escaped);
                start = end.saturating_add(1);
            }
        }
        if end > 0
            && end < characters.len().saturating_sub(1)
            && (character == Some(' ') || start >= end)
            && projected_double_quoted_column(column, end, start) > context.width
        {
            let data = if start < end {
                characters[start..end].iter().collect::<String>()
            } else {
                String::new()
            };
            if start < end {
                start = end;
            }
            output.push_str(&data);
            output.push_str("\\\n");
            output.push_str(&" ".repeat(context.continuation_indent));
            column = context.continuation_indent;
            if characters.get(start) == Some(&' ') {
                output.push('\\');
                column = column.saturating_add(1);
            }
        }
        end = end.saturating_add(1);
    }
    output.push('"');
    output
}

fn projected_double_quoted_column(column: usize, end: usize, start: usize) -> usize {
    if end >= start {
        column.saturating_add(end - start)
    } else {
        // PyYAML uses `column + (end - start)`. Immediately after escaping a
        // character `start == end + 1`, so the signed delta is negative one.
        column.saturating_sub(start - end)
    }
}

fn needs_yaml_escape(character: char) -> bool {
    yaml_escape_replacement(character).is_some() || !(' '..='~').contains(&character)
}

fn yaml_escape(character: char) -> String {
    yaml_escape_replacement(character).map_or_else(|| yaml_numeric_escape(character), str::to_owned)
}

fn yaml_escape_replacement(character: char) -> Option<&'static str> {
    match character {
        '\0' => Some("\\0"),
        '\u{7}' => Some("\\a"),
        '\u{8}' => Some("\\b"),
        '\t' => Some("\\t"),
        '\n' => Some("\\n"),
        '\u{b}' => Some("\\v"),
        '\u{c}' => Some("\\f"),
        '\r' => Some("\\r"),
        '\u{1b}' => Some("\\e"),
        '"' => Some("\\\""),
        '\\' => Some("\\\\"),
        '\u{85}' => Some("\\N"),
        '\u{a0}' => Some("\\_"),
        '\u{2028}' => Some("\\L"),
        '\u{2029}' => Some("\\P"),
        _ => None,
    }
}

fn yaml_numeric_escape(character: char) -> String {
    match u32::from(character) {
        codepoint if codepoint <= 0xff => format!("\\x{codepoint:02X}"),
        codepoint if codepoint <= 0xffff => format!("\\u{codepoint:04X}"),
        codepoint => format!("\\U{codepoint:08X}"),
    }
}

fn current_column(output: &str) -> usize {
    output
        .rsplit_once('\n')
        .map_or(output.len(), |(_, line)| line.len())
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

#[cfg(test)]
mod tests {
    use super::PYYAML_IMPLICIT_SCALAR;

    #[test]
    fn test_should_compile_and_match_pyyaml_implicit_resolver() {
        for value in [
            "true",
            "~",
            "0xFF",
            ".inf",
            "2001-12-15",
            "2001-12-15 02:59:43.1",
        ] {
            assert!(
                PYYAML_IMPLICIT_SCALAR.is_match(value),
                "resolver should match {value}"
            );
        }
        assert!(!PYYAML_IMPLICIT_SCALAR.is_match("1e10"));
    }
}
