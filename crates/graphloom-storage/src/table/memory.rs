use std::sync::Arc;

use arrow::datatypes::SchemaRef;
use async_trait::async_trait;
use dashmap::DashMap;

use super::{Table, TableBatch, TableProvider, TableRow, ensure_open, validate_row};
use crate::{
    Result, StorageError,
    path::{strip_namespace, validate_table_name},
};

/// In-memory [`TableProvider`] for tests and deterministic local execution.
#[derive(Debug, Clone, Default)]
pub struct MemoryTableProvider {
    tables: Arc<DashMap<String, TableBatch>>,
    namespace: String,
}

impl MemoryTableProvider {
    /// Create an empty memory table provider.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn key(&self, table_name: &str) -> Result<String> {
        let name = validate_table_name(table_name)?;
        if self.namespace.is_empty() {
            Ok(name)
        } else {
            Ok(format!("{}/{}", self.namespace, name))
        }
    }
}

#[async_trait]
impl TableProvider for MemoryTableProvider {
    async fn read_table(&self, table_name: &str) -> Result<TableBatch> {
        let key = self.key(table_name)?;
        self.tables
            .get(&key)
            .map(|batch| batch.value().clone())
            .ok_or(StorageError::MissingTable {
                name: table_name.to_owned(),
            })
    }

    async fn write_table(&self, table_name: &str, batch: TableBatch) -> Result<()> {
        self.tables.insert(self.key(table_name)?, batch);
        Ok(())
    }

    async fn has_table(&self, table_name: &str) -> Result<bool> {
        Ok(self.tables.contains_key(&self.key(table_name)?))
    }

    async fn list_tables(&self) -> Result<Vec<String>> {
        let mut names = self
            .tables
            .iter()
            .filter_map(|entry| {
                let key = entry.key();
                if self.namespace.is_empty() || key.starts_with(&format!("{}/", self.namespace)) {
                    Some(strip_namespace(key, &self.namespace))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        names.sort();
        Ok(names)
    }

    async fn open_table(
        &self,
        table_name: &str,
        schema: SchemaRef,
        truncate: bool,
    ) -> Result<Box<dyn Table>> {
        let key = self.key(table_name)?;
        let rows = if truncate {
            Vec::new()
        } else {
            self.tables
                .get(&key)
                .map(|batch| batch.rows().to_vec())
                .unwrap_or_default()
        };

        Ok(Box::new(MemoryTable {
            tables: Arc::clone(&self.tables),
            key,
            schema,
            rows,
            closed: false,
        }))
    }

    fn child(&self, namespace: &str) -> Result<Arc<dyn TableProvider>> {
        let child = validate_table_name(namespace)?;
        let namespace = if self.namespace.is_empty() {
            child
        } else {
            format!("{}/{}", self.namespace, child)
        };

        Ok(Arc::new(Self {
            tables: Arc::clone(&self.tables),
            namespace,
        }))
    }
}

#[derive(Debug)]
struct MemoryTable {
    tables: Arc<DashMap<String, TableBatch>>,
    key: String,
    schema: SchemaRef,
    rows: Vec<TableRow>,
    closed: bool,
}

#[async_trait]
impl Table for MemoryTable {
    async fn append(&mut self, row: TableRow) -> Result<()> {
        ensure_open(self.closed)?;
        validate_row(&self.schema, &row)?;
        self.rows.push(row);
        Ok(())
    }

    fn len(&self) -> usize {
        self.rows.len()
    }

    async fn close(&mut self) -> Result<()> {
        ensure_open(self.closed)?;
        let batch = TableBatch::try_new(Arc::clone(&self.schema), self.rows.clone())?;
        self.tables.insert(self.key.clone(), batch);
        self.closed = true;
        Ok(())
    }

    async fn abort(&mut self) -> Result<()> {
        self.rows.clear();
        self.closed = true;
        Ok(())
    }
}
