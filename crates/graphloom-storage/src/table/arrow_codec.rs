use std::sync::Arc;

use arrow::{
    array::{
        Array, ArrayRef, BooleanArray, BooleanBuilder, Float64Array, Float64Builder, Int64Array,
        Int64Builder, ListArray, ListBuilder, StringArray, StringBuilder,
    },
    datatypes::{DataType, Field},
};

use super::{TableRow, TableValue};
use crate::{Result, StorageError};

pub(super) fn validate_value(field: &Field, value: &TableValue) -> Result<()> {
    if matches!(value, TableValue::Null) {
        if field.is_nullable() {
            return Ok(());
        }
        return Err(StorageError::SchemaMismatch {
            column: field.name().clone(),
            reason: "non-nullable field received null".to_owned(),
        });
    }

    let valid = match (field.data_type(), value) {
        (DataType::Utf8, TableValue::String(_))
        | (DataType::Boolean, TableValue::Bool(_))
        | (DataType::Int64, TableValue::Int(_))
        | (DataType::Float64, TableValue::Float(_)) => true,
        (DataType::List(item), TableValue::List(values)) => values
            .iter()
            .all(|value| validate_list_item(item.data_type(), value)),
        _ => false,
    };

    if valid {
        Ok(())
    } else {
        Err(StorageError::SchemaMismatch {
            column: field.name().clone(),
            reason: format!(
                "value {value:?} is not compatible with {:?}",
                field.data_type()
            ),
        })
    }
}

pub(super) fn build_array(field: &Field, rows: &[TableRow]) -> Result<ArrayRef> {
    match field.data_type() {
        DataType::Utf8 => build_string_array(field, rows),
        DataType::Boolean => build_bool_array(field, rows),
        DataType::Int64 => build_i64_array(field, rows),
        DataType::Float64 => build_f64_array(field, rows),
        DataType::List(item) => build_list_array(field, item.data_type(), rows),
        data_type => Err(StorageError::SchemaMismatch {
            column: field.name().clone(),
            reason: format!("unsupported Arrow data type {data_type:?}"),
        }),
    }
}

pub(super) fn value_from_array(
    array: &ArrayRef,
    row_index: usize,
    field: &Field,
) -> Result<TableValue> {
    if array.is_null(row_index) {
        return Ok(TableValue::Null);
    }

    match field.data_type() {
        DataType::Utf8 => {
            let array = array
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| array_type_error(field))?;
            Ok(TableValue::String(array.value(row_index).to_owned()))
        }
        DataType::Boolean => {
            let array = array
                .as_any()
                .downcast_ref::<BooleanArray>()
                .ok_or_else(|| array_type_error(field))?;
            Ok(TableValue::Bool(array.value(row_index)))
        }
        DataType::Int64 => {
            let array = array
                .as_any()
                .downcast_ref::<Int64Array>()
                .ok_or_else(|| array_type_error(field))?;
            Ok(TableValue::Int(array.value(row_index)))
        }
        DataType::Float64 => {
            let array = array
                .as_any()
                .downcast_ref::<Float64Array>()
                .ok_or_else(|| array_type_error(field))?;
            Ok(TableValue::Float(array.value(row_index)))
        }
        DataType::List(item) => list_value_from_array(array, row_index, field, item.data_type()),
        data_type => Err(StorageError::SchemaMismatch {
            column: field.name().clone(),
            reason: format!("unsupported Arrow data type {data_type:?}"),
        }),
    }
}

fn validate_list_item(data_type: &DataType, value: &TableValue) -> bool {
    match (data_type, value) {
        (_, TableValue::Null)
        | (DataType::Utf8, TableValue::String(_))
        | (DataType::Int64, TableValue::Int(_))
        | (DataType::Float64, TableValue::Float(_))
        | (DataType::Boolean, TableValue::Bool(_)) => true,
        (DataType::List(item), TableValue::List(values)) => values
            .iter()
            .all(|value| validate_list_item(item.data_type(), value)),
        _ => false,
    }
}

fn build_string_array(field: &Field, rows: &[TableRow]) -> Result<ArrayRef> {
    let mut builder = StringBuilder::new();
    for row in rows {
        match row.get(field.name()).unwrap_or(&TableValue::Null) {
            TableValue::Null => builder.append_null(),
            TableValue::String(value) => builder.append_value(value),
            value => return schema_error(field, value),
        }
    }
    Ok(Arc::new(builder.finish()))
}

fn build_bool_array(field: &Field, rows: &[TableRow]) -> Result<ArrayRef> {
    let mut builder = BooleanBuilder::new();
    for row in rows {
        match row.get(field.name()).unwrap_or(&TableValue::Null) {
            TableValue::Null => builder.append_null(),
            TableValue::Bool(value) => builder.append_value(*value),
            value => return schema_error(field, value),
        }
    }
    Ok(Arc::new(builder.finish()))
}

fn build_i64_array(field: &Field, rows: &[TableRow]) -> Result<ArrayRef> {
    let mut builder = Int64Builder::new();
    for row in rows {
        match row.get(field.name()).unwrap_or(&TableValue::Null) {
            TableValue::Null => builder.append_null(),
            TableValue::Int(value) => builder.append_value(*value),
            value => return schema_error(field, value),
        }
    }
    Ok(Arc::new(builder.finish()))
}

fn build_f64_array(field: &Field, rows: &[TableRow]) -> Result<ArrayRef> {
    let mut builder = Float64Builder::new();
    for row in rows {
        match row.get(field.name()).unwrap_or(&TableValue::Null) {
            TableValue::Null => builder.append_null(),
            TableValue::Float(value) => builder.append_value(*value),
            value => return schema_error(field, value),
        }
    }
    Ok(Arc::new(builder.finish()))
}

fn build_list_array(field: &Field, item_type: &DataType, rows: &[TableRow]) -> Result<ArrayRef> {
    match item_type {
        DataType::Utf8 => build_utf8_list_array(field, rows),
        DataType::Int64 => build_i64_list_array(field, rows),
        data_type => Err(StorageError::SchemaMismatch {
            column: field.name().clone(),
            reason: format!("unsupported list item type {data_type:?}"),
        }),
    }
}

fn build_utf8_list_array(field: &Field, rows: &[TableRow]) -> Result<ArrayRef> {
    let mut builder = ListBuilder::new(StringBuilder::new());
    for row in rows {
        match row.get(field.name()).unwrap_or(&TableValue::Null) {
            TableValue::Null => builder.append(false),
            TableValue::List(values) => {
                for value in values {
                    match value {
                        TableValue::Null => builder.values().append_null(),
                        TableValue::String(value) => builder.values().append_value(value),
                        value => return schema_error(field, value),
                    }
                }
                builder.append(true);
            }
            value => return schema_error(field, value),
        }
    }
    Ok(Arc::new(builder.finish()))
}

fn build_i64_list_array(field: &Field, rows: &[TableRow]) -> Result<ArrayRef> {
    let mut builder = ListBuilder::new(Int64Builder::new());
    for row in rows {
        match row.get(field.name()).unwrap_or(&TableValue::Null) {
            TableValue::Null => builder.append(false),
            TableValue::List(values) => {
                for value in values {
                    match value {
                        TableValue::Null => builder.values().append_null(),
                        TableValue::Int(value) => builder.values().append_value(*value),
                        value => return schema_error(field, value),
                    }
                }
                builder.append(true);
            }
            value => return schema_error(field, value),
        }
    }
    Ok(Arc::new(builder.finish()))
}

fn schema_error<T>(field: &Field, value: &TableValue) -> Result<T> {
    Err(StorageError::SchemaMismatch {
        column: field.name().clone(),
        reason: format!(
            "value {value:?} is not compatible with {:?}",
            field.data_type()
        ),
    })
}

fn list_value_from_array(
    array: &ArrayRef,
    row_index: usize,
    field: &Field,
    item_type: &DataType,
) -> Result<TableValue> {
    let list = array
        .as_any()
        .downcast_ref::<ListArray>()
        .ok_or_else(|| array_type_error(field))?;
    let values = list.value(row_index);

    match item_type {
        DataType::Utf8 => {
            let values = values
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| array_type_error(field))?;
            let items = (0..values.len())
                .map(|index| {
                    if values.is_null(index) {
                        TableValue::Null
                    } else {
                        TableValue::String(values.value(index).to_owned())
                    }
                })
                .collect();
            Ok(TableValue::List(items))
        }
        DataType::Int64 => {
            let values = values
                .as_any()
                .downcast_ref::<Int64Array>()
                .ok_or_else(|| array_type_error(field))?;
            let items = (0..values.len())
                .map(|index| {
                    if values.is_null(index) {
                        TableValue::Null
                    } else {
                        TableValue::Int(values.value(index))
                    }
                })
                .collect();
            Ok(TableValue::List(items))
        }
        data_type => Err(StorageError::SchemaMismatch {
            column: field.name().clone(),
            reason: format!("unsupported list item type {data_type:?}"),
        }),
    }
}

fn array_type_error(field: &Field) -> StorageError {
    StorageError::SchemaMismatch {
        column: field.name().clone(),
        reason: "Arrow array type does not match schema".to_owned(),
    }
}
