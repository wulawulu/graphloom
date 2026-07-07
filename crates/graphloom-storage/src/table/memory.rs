use std::{fmt, sync::Arc};

use async_trait::async_trait;
use dashmap::DashMap;
use polars_core::prelude::DataFrame;

use super::{
    Table, TableProvider, append_optional_dataframe, id_column_index, next_dataframe_row,
    row_from_dataframe, row_matches_id, row_stream,
};
use crate::{
    Result, StorageError,
    path::{path_to_logical, strip_namespace, validate_logical_path, validate_table_name},
};

/// In-memory [`TableProvider`] for tests and deterministic local execution.
#[derive(Debug, Clone, Default)]
pub struct MemoryTableProvider {
    tables: Arc<DashMap<String, DataFrame>>,
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
    async fn read_dataframe(&self, table_name: &str) -> Result<DataFrame> {
        let key = self.key(table_name)?;
        self.tables
            .get(&key)
            .map(|dataframe| dataframe.value().clone())
            .ok_or(StorageError::MissingTable {
                name: table_name.to_owned(),
            })
    }

    async fn write_dataframe(&self, table_name: &str, dataframe: DataFrame) -> Result<()> {
        self.tables.insert(self.key(table_name)?, dataframe);
        Ok(())
    }

    async fn has(&self, table_name: &str) -> Result<bool> {
        Ok(self.tables.contains_key(&self.key(table_name)?))
    }

    async fn list(&self) -> Result<Vec<String>> {
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

    async fn open(&self, table_name: &str, truncate: bool) -> Result<Box<dyn Table>> {
        let key = self.key(table_name)?;
        let existing = if truncate {
            None
        } else {
            self.tables
                .get(&key)
                .map(|dataframe| dataframe.value().clone())
        };

        Ok(Box::new(MemoryTable {
            tables: Arc::clone(&self.tables),
            key,
            existing,
            truncate,
            pending: DataFrame::empty(),
            read_index: 0,
            closed: false,
        }))
    }

    fn child(&self, namespace: Option<&str>) -> Result<Arc<dyn TableProvider>> {
        let Some(namespace) = namespace else {
            return Ok(Arc::new(self.clone()));
        };
        let child = path_to_logical(&validate_logical_path(namespace)?);
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

struct MemoryTable {
    tables: Arc<DashMap<String, DataFrame>>,
    key: String,
    existing: Option<DataFrame>,
    truncate: bool,
    pending: DataFrame,
    read_index: usize,
    closed: bool,
}

impl fmt::Debug for MemoryTable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemoryTable")
            .field("key", &self.key)
            .field(
                "existing_rows",
                &self.existing.as_ref().map_or(0, DataFrame::height),
            )
            .field("truncate", &self.truncate)
            .field("pending_rows", &self.pending.height())
            .field("read_index", &self.read_index)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Table for MemoryTable {
    async fn write(&mut self, dataframe: DataFrame) -> Result<()> {
        self.check_open()?;
        self.pending =
            append_optional_dataframe(Some(std::mem::take(&mut self.pending)), &dataframe)?;
        Ok(())
    }

    fn length(&self) -> usize {
        self.existing.as_ref().map_or(0, DataFrame::height) + self.pending.height()
    }

    async fn close(&mut self) -> Result<()> {
        self.check_open()?;
        if self.pending.height() == 0 {
            self.closed = true;
            return Ok(());
        }

        let existing = if self.truncate {
            None
        } else {
            self.tables.get(&self.key).map(|df| df.clone())
        };
        let dataframe = append_optional_dataframe(existing, &self.pending)?;
        self.tables.insert(self.key.clone(), dataframe);
        self.closed = true;
        Ok(())
    }

    async fn abort(&mut self) -> Result<()> {
        self.pending = DataFrame::empty();
        self.closed = true;
        Ok(())
    }

    fn rows(&mut self) -> super::RowStream<'_> {
        row_stream(move || {
            next_dataframe_row(self.existing.as_ref(), &self.pending, &mut self.read_index)
        })
    }

    async fn has(&self, row_id: &str) -> Result<bool> {
        if let Some(existing) = &self.existing {
            let Some(id_index) = id_column_index(existing) else {
                return Ok(false);
            };
            for row_index in 0..existing.height() {
                let row = row_from_dataframe(existing, row_index)?;
                if row_matches_id(&row, id_index, row_id) {
                    return Ok(true);
                }
            }
        }
        if let Some(id_index) = id_column_index(&self.pending) {
            for row_index in 0..self.pending.height() {
                let row = row_from_dataframe(&self.pending, row_index)?;
                if row_matches_id(&row, id_index, row_id) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

impl MemoryTable {
    fn check_open(&self) -> Result<()> {
        if self.closed {
            return Err(StorageError::TableClosed {
                name: self.key.clone(),
            });
        }
        Ok(())
    }
}
