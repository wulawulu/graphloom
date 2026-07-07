//! Text document data model.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::property::get_property;

/// A text document read from an input source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TextDocument {
    /// Unique document identifier.
    pub id: String,
    /// Main text content.
    pub text: String,
    /// Human-facing title, usually the file name.
    pub title: String,
    /// Optional creation date in ISO-8601 format when provided by storage.
    pub creation_date: Option<String>,
    /// Raw source row/object metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_data: Option<Value>,
}

impl TextDocument {
    /// Create a text document.
    #[must_use]
    pub fn new(
        id: String,
        text: String,
        title: String,
        creation_date: Option<String>,
        raw_data: Option<Value>,
    ) -> Self {
        Self {
            id,
            text,
            title,
            creation_date,
            raw_data,
        }
    }

    /// Get a standard field or a nested raw-data field using dot notation.
    #[must_use]
    pub fn get(&self, field: &str) -> Option<Value> {
        match field {
            "id" => Some(Value::String(self.id.clone())),
            "title" => Some(Value::String(self.title.clone())),
            "text" => Some(Value::String(self.text.clone())),
            "creation_date" => self.creation_date.clone().map(Value::String),
            field => self
                .raw_data
                .as_ref()
                .and_then(|raw_data| get_property(raw_data, field))
                .cloned(),
        }
    }

    /// Collect selected fields, preserving the requested field order.
    #[must_use]
    pub fn collect(&self, fields: &[String]) -> Vec<(String, Value)> {
        fields
            .iter()
            .filter_map(|field| self.get(field).map(|value| (field.clone(), value)))
            .collect()
    }
}
