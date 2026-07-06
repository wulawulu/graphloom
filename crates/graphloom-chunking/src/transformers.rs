//! Built-in chunk text transformers.

use serde_json::Value;

/// Metadata transformer that prepends or appends key/value rows.
#[derive(Debug, Clone)]
pub struct MetadataTransform {
    metadata_text: String,
    append: bool,
}

impl MetadataTransform {
    /// Apply this transformer to `text`.
    #[must_use]
    pub fn transform(&self, text: &str) -> String {
        if self.append {
            let mut output = String::with_capacity(text.len() + self.metadata_text.len());
            output.push_str(text);
            output.push_str(&self.metadata_text);
            output
        } else {
            let mut output = String::with_capacity(text.len() + self.metadata_text.len());
            output.push_str(&self.metadata_text);
            output.push_str(text);
            output
        }
    }
}

/// Create a metadata transformer.
///
/// `GraphRAG` writes metadata as `key: value` rows joined by `\n`, then adds one
/// final line delimiter before concatenating the chunk text.
#[must_use]
pub fn add_metadata(
    metadata: &[(String, Value)],
    delimiter: &str,
    line_delimiter: &str,
    append: bool,
) -> MetadataTransform {
    let mut metadata_text = metadata
        .iter()
        .map(|(key, value)| format!("{key}{delimiter}{}", value_to_text(value)))
        .collect::<Vec<_>>()
        .join(line_delimiter);
    metadata_text.push_str(line_delimiter);

    MetadataTransform {
        metadata_text,
        append,
    }
}

/// Prepend metadata using `GraphRAG`'s default delimiters.
#[must_use]
pub fn prepend_metadata(text: &str, metadata: &[(String, Value)]) -> String {
    add_metadata(metadata, ": ", "\n", false).transform(text)
}

fn value_to_text(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}
