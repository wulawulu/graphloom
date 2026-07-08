//! Shared `Polars` dataframe helpers.

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
    any_value_to_string_list(value, workflow)
}

pub(crate) fn list_column_at(
    dataframe: &DataFrame,
    row_index: usize,
    column: &'static str,
    workflow: &'static str,
) -> Result<Vec<String>> {
    let index = column_index(dataframe, column, workflow)?;
    let row = row_to_static(dataframe.get_row(row_index)?);
    list_at(&row, index, workflow)
}

pub(crate) fn string_list_or_string_column_at(
    dataframe: &DataFrame,
    row_index: usize,
    column: &'static str,
    workflow: &'static str,
) -> Result<Vec<String>> {
    let index = column_index(dataframe, column, workflow)?;
    let row = row_to_static(dataframe.get_row(row_index)?);
    let Some(value) = row.0.get(index) else {
        return Ok(Vec::new());
    };
    match value {
        AnyValue::List(_) | AnyValue::Null => any_value_to_string_list(value, workflow),
        AnyValue::String(value) => Ok(vec![(*value).to_owned()]),
        AnyValue::StringOwned(value) => Ok(vec![value.to_string()]),
        _ => Err(invalid_data(
            workflow,
            &format!("expected string or string list column {column}"),
        )),
    }
}

pub(crate) fn i64_column_value(
    dataframe: &DataFrame,
    row_index: usize,
    column: &'static str,
    workflow: &'static str,
) -> Result<i64> {
    let series = dataframe.column(column)?;
    let value = series.get(row_index)?;
    match value {
        AnyValue::Int64(value) => Ok(value),
        AnyValue::Int32(value) => Ok(i64::from(value)),
        AnyValue::UInt32(value) => Ok(i64::from(value)),
        _ => Err(invalid_data(
            workflow,
            &format!("missing integer column {column}"),
        )),
    }
}

pub(crate) fn usize_column_value(
    dataframe: &DataFrame,
    row_index: usize,
    column: &'static str,
    workflow: &'static str,
) -> Result<usize> {
    let series = dataframe.column(column)?;
    let value = series.get(row_index)?;
    match value {
        AnyValue::UInt64(value) => usize::try_from(value).map_err(|source| {
            invalid_data(
                workflow,
                &format!("column {column} is too large for usize: {source}"),
            )
        }),
        AnyValue::UInt32(value) => usize::try_from(value).map_err(|source| {
            invalid_data(
                workflow,
                &format!("column {column} is too large for usize: {source}"),
            )
        }),
        AnyValue::Int64(value) => usize::try_from(value).map_err(|source| {
            invalid_data(
                workflow,
                &format!("column {column} must be non-negative and fit usize: {source}"),
            )
        }),
        AnyValue::Int32(value) => usize::try_from(value).map_err(|source| {
            invalid_data(
                workflow,
                &format!("column {column} must be non-negative and fit usize: {source}"),
            )
        }),
        _ => Err(invalid_data(
            workflow,
            &format!("missing unsigned integer column {column}"),
        )),
    }
}

pub(crate) fn f64_column_value(
    dataframe: &DataFrame,
    row_index: usize,
    column: &'static str,
    workflow: &'static str,
) -> Result<f64> {
    let series = dataframe.column(column)?;
    let value = series.get(row_index)?;
    match value {
        AnyValue::Float64(value) => Ok(value),
        AnyValue::Float32(value) => Ok(f64::from(value)),
        _ => Err(invalid_data(
            workflow,
            &format!("missing float column {column}"),
        )),
    }
}

pub(crate) fn list_column(name: &str, rows: &[Vec<String>]) -> Result<Column> {
    let series_rows = rows
        .iter()
        .map(|values| {
            let refs = values.iter().map(String::as_str).collect::<Vec<_>>();
            Series::new("item".into(), refs)
        })
        .collect::<Vec<_>>();
    Ok(Series::new(name.into(), series_rows).into())
}

fn column_index(
    dataframe: &DataFrame,
    column: &'static str,
    workflow: &'static str,
) -> Result<usize> {
    dataframe
        .get_column_names()
        .iter()
        .position(|name| name.as_str() == column)
        .ok_or_else(|| invalid_data(workflow, &format!("missing column {column}")))
}

fn any_value_to_string(value: &AnyValue<'_>) -> Option<String> {
    match value {
        AnyValue::String(value) => Some((*value).to_owned()),
        AnyValue::StringOwned(value) => Some(value.to_string()),
        AnyValue::Null => None,
        _ => None,
    }
}

fn any_value_to_string_list(value: &AnyValue<'_>, workflow: &'static str) -> Result<Vec<String>> {
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
