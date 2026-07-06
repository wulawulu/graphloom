//! Table model, provider traits, and concrete table-provider implementations.

mod arrow_codec;
mod memory;
mod parquet;
mod parquet_io;
/// Arrow schemas for GraphRAG-compatible final tables.
pub mod schemas;

use std::{collections::BTreeMap, sync::Arc};

use arrow::{
    datatypes::{Schema, SchemaRef},
    record_batch::RecordBatch,
};
use async_trait::async_trait;
pub use memory::MemoryTableProvider;
pub use parquet::ParquetTableProvider;
use serde::{Deserialize, Serialize};

use self::arrow_codec::{build_array, validate_value, value_from_array};
use crate::{Result, StorageError};

/// A `GraphLoom` table value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TableValue {
    /// Null value.
    Null,
    /// Boolean value.
    Bool(bool),
    /// Signed 64-bit integer value.
    Int(i64),
    /// 64-bit floating point value.
    Float(f64),
    /// UTF-8 string value.
    String(String),
    /// Ordered list value.
    List(Vec<TableValue>),
    /// Structured object value.
    Object(BTreeMap<String, TableValue>),
}

/// A table row keyed by column name.
pub type TableRow = BTreeMap<String, TableValue>;

/// A table represented as an Arrow schema plus row values.
#[derive(Debug, Clone, PartialEq)]
pub struct TableBatch {
    schema: SchemaRef,
    rows: Vec<TableRow>,
}

impl TableBatch {
    /// Create a table batch after validating rows against `schema`.
    ///
    /// # Errors
    ///
    /// Returns an error when a row value does not match the declared Arrow
    /// schema.
    pub fn try_new(schema: SchemaRef, rows: Vec<TableRow>) -> Result<Self> {
        let batch = Self { schema, rows };
        batch.validate()?;
        Ok(batch)
    }

    /// Return this batch's schema.
    #[must_use]
    pub fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    /// Return this batch's rows.
    #[must_use]
    pub fn rows(&self) -> &[TableRow] {
        &self.rows
    }

    /// Convert this batch into an Arrow [`RecordBatch`].
    ///
    /// # Errors
    ///
    /// Returns an error when conversion to Arrow arrays fails.
    pub fn to_record_batch(&self) -> Result<RecordBatch> {
        let arrays = self
            .schema
            .fields()
            .iter()
            .map(|field| build_array(field, &self.rows))
            .collect::<Result<Vec<_>>>()?;
        RecordBatch::try_new(Arc::clone(&self.schema), arrays).map_err(StorageError::Arrow)
    }

    fn validate(&self) -> Result<()> {
        for field in self.schema.fields() {
            for row in &self.rows {
                let value = row.get(field.name()).unwrap_or(&TableValue::Null);
                validate_value(field, value)?;
            }
        }
        Ok(())
    }
}

impl TryFrom<RecordBatch> for TableBatch {
    type Error = StorageError;

    fn try_from(batch: RecordBatch) -> Result<Self> {
        let schema = batch.schema();
        let mut rows = Vec::with_capacity(batch.num_rows());

        for row_index in 0..batch.num_rows() {
            let mut row = TableRow::new();
            for (column_index, field) in schema.fields().iter().enumerate() {
                row.insert(
                    field.name().clone(),
                    value_from_array(batch.column(column_index), row_index, field)?,
                );
            }
            rows.push(row);
        }

        Self::try_new(schema, rows)
    }
}

/// Streaming table writer used by workflows that produce rows incrementally.
#[async_trait]
pub trait Table: Send + std::fmt::Debug {
    /// Append a row to the staged table output.
    ///
    /// # Errors
    ///
    /// Returns an error when the row does not match the table schema or the
    /// table has already been closed.
    async fn append(&mut self, row: TableRow) -> Result<()>;

    /// Number of rows currently staged.
    fn len(&self) -> usize;

    /// Return true when no rows are currently staged.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

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
}

/// Provider for named `GraphLoom` tables.
#[async_trait]
pub trait TableProvider: Send + Sync + std::fmt::Debug {
    /// Read a complete table.
    ///
    /// # Errors
    ///
    /// Returns an error when the table does not exist or cannot be decoded.
    async fn read_table(&self, table_name: &str) -> Result<TableBatch>;

    /// Write a complete table, replacing any existing table atomically where the
    /// backing store supports it.
    ///
    /// # Errors
    ///
    /// Returns an error when validation or persistence fails.
    async fn write_table(&self, table_name: &str, batch: TableBatch) -> Result<()>;

    /// Return whether a table exists.
    ///
    /// # Errors
    ///
    /// Returns an error when the table name is invalid.
    async fn has_table(&self, table_name: &str) -> Result<bool>;

    /// List table names in this provider namespace.
    ///
    /// # Errors
    ///
    /// Returns an error when the backing store cannot be enumerated.
    async fn list_tables(&self) -> Result<Vec<String>>;

    /// Open a streaming table writer.
    ///
    /// `truncate=true` starts from an empty table. `truncate=false` appends to
    /// the existing table if one exists.
    ///
    /// # Errors
    ///
    /// Returns an error when the table name is invalid or an existing table
    /// cannot be read for append mode.
    async fn open_table(
        &self,
        table_name: &str,
        schema: SchemaRef,
        truncate: bool,
    ) -> Result<Box<dyn Table>>;

    /// Create a namespace view rooted at `namespace`.
    ///
    /// # Errors
    ///
    /// Returns an error when the namespace is invalid.
    fn child(&self, namespace: &str) -> Result<Arc<dyn TableProvider>>;
}

fn ensure_open(closed: bool) -> Result<()> {
    if closed {
        return Err(StorageError::SchemaMismatch {
            column: "<table>".to_owned(),
            reason: "table writer is already closed".to_owned(),
        });
    }
    Ok(())
}

fn validate_row(schema: &Schema, row: &TableRow) -> Result<()> {
    for field in schema.fields() {
        let value = row.get(field.name()).unwrap_or(&TableValue::Null);
        validate_value(field, value)?;
    }
    Ok(())
}
