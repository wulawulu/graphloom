//! Shared workflow helpers.

use polars_core::{frame::row::Row, prelude::*};

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

pub(crate) fn row_to_static(row: Row<'_>) -> Row<'static> {
    Row::new(row.0.into_iter().map(AnyValue::into_static).collect())
}

pub(crate) fn string_at(
    row: &Row<'static>,
    index: usize,
    column: &'static str,
    workflow: &'static str,
) -> Result<String> {
    row.0
        .get(index)
        .and_then(any_value_to_string)
        .ok_or_else(|| invalid_data(workflow, &format!("missing string column {column}")))
}

pub(crate) fn optional_string_at(row: &Row<'static>, index: usize) -> Option<String> {
    row.0.get(index).and_then(any_value_to_string)
}

pub(crate) fn list_at(
    row: &Row<'static>,
    index: usize,
    workflow: &'static str,
) -> Result<Vec<String>> {
    let Some(value) = row.0.get(index) else {
        return Ok(Vec::new());
    };
    match value {
        AnyValue::List(series) => {
            let strings = series.str()?;
            Ok((0..series.len())
                .filter_map(|index| strings.get(index).map(str::to_owned))
                .collect())
        }
        AnyValue::Null => Ok(Vec::new()),
        AnyValue::String(value) => Ok(vec![(*value).to_owned()]),
        AnyValue::StringOwned(value) => Ok(vec![value.to_string()]),
        _ => Err(invalid_data(workflow, "expected string list column")),
    }
}

pub(crate) fn string_list_or_string_at(
    row: &Row<'static>,
    index: usize,
    workflow: &'static str,
) -> Vec<String> {
    let values = list_at(row, index, workflow)
        .ok()
        .filter(|values| !values.is_empty())
        .or_else(|| optional_string_at(row, index).map(|value| vec![value]));
    let Some(values) = values else {
        return Vec::new();
    };
    values
}

pub(crate) fn i64_at(
    row: &Row<'static>,
    index: usize,
    column: &'static str,
    workflow: &'static str,
) -> Result<i64> {
    row.0
        .get(index)
        .and_then(|value| match value {
            AnyValue::Int64(value) => Some(*value),
            AnyValue::Int32(value) => Some(i64::from(*value)),
            AnyValue::UInt32(value) => Some(i64::from(*value)),
            _ => None,
        })
        .ok_or_else(|| invalid_data(workflow, &format!("missing integer column {column}")))
}

pub(crate) fn f64_at(
    row: &Row<'static>,
    index: usize,
    column: &'static str,
    workflow: &'static str,
) -> Result<f64> {
    row.0
        .get(index)
        .and_then(|value| match value {
            AnyValue::Float64(value) => Some(*value),
            AnyValue::Float32(value) => Some(f64::from(*value)),
            _ => None,
        })
        .ok_or_else(|| invalid_data(workflow, &format!("missing float column {column}")))
}

fn any_value_to_string(value: &AnyValue<'_>) -> Option<String> {
    match value {
        AnyValue::String(value) => Some((*value).to_owned()),
        AnyValue::StringOwned(value) => Some(value.to_string()),
        AnyValue::Null => None,
        _ => None,
    }
}
