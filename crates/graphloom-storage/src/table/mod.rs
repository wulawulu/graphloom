//! Table model, provider traits, and concrete table-provider implementations.

mod memory;
mod parquet;

use std::{collections::BTreeSet, pin::Pin, sync::Arc};

use async_trait::async_trait;
use futures_core::Stream;
use futures_util::stream;
pub use memory::MemoryTableProvider;
pub use parquet::ParquetTableProvider;
use polars_core::{
    frame::row::Row,
    prelude::{AnyValue, DataFrame},
};

use crate::{Result, StorageError};

/// Stream of table rows.
pub type RowStream<'a> = Pin<Box<dyn Stream<Item = Result<Row<'static>>> + Send + 'a>>;

/// Streaming table handle used by workflows that read or write rows
/// incrementally.
#[async_trait]
pub trait Table: Send + std::fmt::Debug {
    /// Write dataframe rows to the staged table output.
    ///
    /// # Errors
    ///
    /// Returns an error when the dataframe cannot be appended or the
    /// table has already been closed.
    async fn write(&mut self, dataframe: DataFrame) -> Result<()>;

    /// Number of rows visible through this table handle.
    fn length(&self) -> usize;

    /// Number of rows visible through this table handle.
    fn len(&self) -> usize {
        self.length()
    }

    /// Return true when no rows are visible through this table handle.
    fn is_empty(&self) -> bool {
        self.length() == 0
    }

    /// Visible column names in table order.
    fn column_names(&self) -> Vec<String>;

    /// Commit staged rows.
    ///
    /// # Errors
    ///
    /// Returns an error when commit fails.
    async fn close(&mut self) -> Result<()>;

    /// Discard staged rows.
    ///
    /// # Errors
    ///
    /// Returns an error when cleanup fails.
    async fn abort(&mut self) -> Result<()>;

    /// Stream rows from this table handle.
    fn rows(&mut self) -> RowStream<'_>;

    /// Return whether any visible row has an `id` field equal to `row_id`.
    ///
    /// # Errors
    ///
    /// Returns an error when dataframe row extraction fails while scanning.
    async fn has(&self, row_id: &str) -> Result<bool>;
}

/// Provider for named `GraphLoom` tables.
#[async_trait]
pub trait TableProvider: Send + Sync + std::fmt::Debug {
    /// Read a complete table dataframe.
    ///
    /// # Errors
    ///
    /// Returns an error when the table does not exist or cannot be decoded.
    async fn read_dataframe(&self, table_name: &str) -> Result<DataFrame>;

    /// Write a complete table dataframe, replacing any existing table.
    ///
    /// # Errors
    ///
    /// Returns an error when validation or persistence fails.
    async fn write_dataframe(&self, table_name: &str, dataframe: DataFrame) -> Result<()>;

    /// Return whether a table exists.
    ///
    /// # Errors
    ///
    /// Returns an error when the table name is invalid.
    async fn has(&self, table_name: &str) -> Result<bool>;

    /// List table names in this provider namespace.
    ///
    /// # Errors
    ///
    /// Returns an error when the backing store cannot be enumerated.
    async fn list(&self) -> Result<Vec<String>>;

    /// Open a streaming table handle.
    ///
    /// `truncate=true` starts from an empty table. `truncate=false` appends to
    /// the existing table if one exists.
    ///
    /// # Errors
    ///
    /// Returns an error when the table name is invalid or an existing table
    /// cannot be read for append mode.
    async fn open(&self, table_name: &str, truncate: bool) -> Result<Box<dyn Table>>;

    /// Create a namespace view rooted at `namespace`.
    ///
    /// # Errors
    ///
    /// Returns an error when the namespace is invalid.
    fn child(&self, namespace: Option<&str>) -> Result<Arc<dyn TableProvider>>;
}

pub(super) fn append_dataframe(mut existing: DataFrame, pending: &DataFrame) -> Result<DataFrame> {
    if existing.height() == 0 {
        return Ok(pending.clone());
    }
    if pending.height() == 0 {
        return Ok(existing);
    }

    let existing_names = column_name_set(&existing);
    let pending_names = column_name_set(pending);
    if existing_names != pending_names {
        return Err(StorageError::SchemaMismatch {
            column: "<table>".to_owned(),
            reason: format!(
                "cannot append dataframe with columns {pending_names:?} to existing columns \
                 {existing_names:?}"
            ),
        });
    }

    let names = existing
        .get_column_names_owned()
        .into_iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    let pending = pending.select(&names)?;
    existing.vstack_mut(&pending)?;
    Ok(existing)
}

pub(super) fn append_optional_dataframe(
    existing: Option<DataFrame>,
    pending: &DataFrame,
) -> Result<DataFrame> {
    existing.map_or_else(
        || Ok(pending.clone()),
        |existing| append_dataframe(existing, pending),
    )
}

pub(super) fn row_from_dataframe(dataframe: &DataFrame, row_index: usize) -> Result<Row<'static>> {
    let row = dataframe.get_row(row_index)?;
    Ok(Row::new(
        row.0
            .into_iter()
            .map(AnyValue::into_static)
            .collect::<Vec<_>>(),
    ))
}

pub(super) fn next_dataframe_row(
    existing: Option<&DataFrame>,
    pending: &DataFrame,
    read_index: &mut usize,
) -> Result<Option<Row<'static>>> {
    let existing_rows = existing.map_or(0, DataFrame::height);
    let row = if let Some(existing) = existing
        && *read_index < existing_rows
    {
        row_from_dataframe(existing, *read_index)?
    } else {
        let pending_index = read_index.saturating_sub(existing_rows);
        if pending_index >= pending.height() {
            return Ok(None);
        }
        row_from_dataframe(pending, pending_index)?
    };
    *read_index = read_index.saturating_add(1);
    Ok(Some(row))
}

pub(super) fn row_stream<'a, F>(next_row: F) -> RowStream<'a>
where
    F: FnMut() -> Result<Option<Row<'static>>> + Send + 'a,
{
    Box::pin(stream::unfold(
        (next_row, false),
        |(mut next_row, finished)| async move {
            if finished {
                return None;
            }
            match next_row() {
                Ok(Some(row)) => Some((Ok(row), (next_row, false))),
                Ok(None) => None,
                Err(error) => Some((Err(error), (next_row, true))),
            }
        },
    ))
}

pub(super) fn row_matches_id(row: &Row<'static>, id_index: usize, row_id: &str) -> bool {
    row.0.get(id_index).is_some_and(|value| match value {
        AnyValue::String(value) => *value == row_id,
        AnyValue::StringOwned(value) => value.as_str() == row_id,
        _ => false,
    })
}

pub(super) fn id_column_index(dataframe: &DataFrame) -> Option<usize> {
    dataframe
        .get_column_names()
        .iter()
        .position(|name| name.as_str() == "id")
}

fn column_name_set(dataframe: &DataFrame) -> BTreeSet<String> {
    dataframe
        .get_column_names_owned()
        .into_iter()
        .map(|name| name.to_string())
        .collect()
}
