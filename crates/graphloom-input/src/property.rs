//! Nested property access for source document metadata.

use serde_json::Value;

pub(crate) fn get_property<'a>(data: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = data;
    for key in path.split('.') {
        current = current.as_object()?.get(key)?;
    }
    Some(current)
}
