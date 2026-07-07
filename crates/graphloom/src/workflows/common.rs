//! Shared workflow helpers.

use crate::{GraphLoomError, Result};

pub(crate) fn string_value(
    value: Option<&str>,
    column: &'static str,
    workflow: &'static str,
) -> Result<String> {
    value
        .map(str::to_owned)
        .ok_or_else(|| invalid_data(workflow, &format!("missing string column {column}")))
}

pub(crate) fn invalid_data(workflow: &'static str, message: &str) -> GraphLoomError {
    GraphLoomError::InvalidData {
        workflow,
        message: message.to_owned(),
    }
}
