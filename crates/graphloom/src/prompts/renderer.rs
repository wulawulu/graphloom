//! `GraphRAG` single-brace prompt rendering and template validation.

use serde::Serialize;
use serde_json::{Map, Value};

use super::PromptKind;
use crate::{GraphLoomError, Result};

pub(super) fn render_graphrag_prompt<T>(
    kind: PromptKind,
    template: &str,
    values: &T,
) -> Result<String>
where
    T: Serialize,
{
    let values = prompt_values(kind, values)?;
    let bytes = template.as_bytes();
    let mut output = String::with_capacity(template.len());
    let mut cursor = 0usize;
    let mut literal_start = 0usize;

    while cursor < bytes.len() {
        match bytes[cursor] {
            b'{' => {
                output.push_str(&template[literal_start..cursor]);
                if bytes.get(cursor.saturating_add(1)) == Some(&b'{') {
                    output.push('{');
                    cursor = cursor.saturating_add(2);
                    literal_start = cursor;
                    continue;
                }
                let variable_start = cursor.saturating_add(1);
                let Some(relative_end) = template[variable_start..].find('}') else {
                    return prompt_error(kind, "unclosed `{` in GraphRAG prompt");
                };
                let variable_end = variable_start.saturating_add(relative_end);
                let variable = template[variable_start..variable_end].trim();
                if variable.is_empty() {
                    return prompt_error(kind, "empty GraphRAG prompt variable");
                }
                let value = values
                    .get(variable)
                    .ok_or_else(|| GraphLoomError::PromptRender {
                        name: kind.filename(),
                        message: format!("missing prompt variable `{variable}`"),
                    })?;
                output.push_str(&value_as_prompt_text(value));
                cursor = variable_end.saturating_add(1);
                literal_start = cursor;
            }
            b'}' => {
                output.push_str(&template[literal_start..cursor]);
                if bytes.get(cursor.saturating_add(1)) == Some(&b'}') {
                    output.push('}');
                    cursor = cursor.saturating_add(2);
                    literal_start = cursor;
                    continue;
                }
                return prompt_error(kind, "isolated `}` in GraphRAG prompt");
            }
            _ => cursor = cursor.saturating_add(1),
        }
    }
    output.push_str(&template[literal_start..]);
    Ok(output)
}

pub(super) fn prompt_values<T>(kind: PromptKind, values: &T) -> Result<Map<String, Value>>
where
    T: Serialize,
{
    let value = serde_json::to_value(values).map_err(|source| GraphLoomError::PromptRender {
        name: kind.filename(),
        message: format!("failed to serialize prompt values: {source}"),
    })?;
    let Value::Object(values) = value else {
        return prompt_error(kind, "prompt values must serialize to an object");
    };
    for variable in kind.required_variables() {
        if !values.contains_key(*variable) {
            return prompt_error(
                kind,
                &format!("missing required prompt variable `{variable}`"),
            );
        }
    }
    Ok(values)
}

fn value_as_prompt_text(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => "null".to_owned(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn prompt_error<T>(kind: PromptKind, message: &str) -> Result<T> {
    Err(GraphLoomError::PromptRender {
        name: kind.filename(),
        message: message.to_owned(),
    })
}
