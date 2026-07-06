use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use arrow::datatypes::SchemaRef;
use async_trait::async_trait;

use super::{
    Table, TableBatch, TableProvider, TableRow, ensure_open,
    parquet_io::{read_parquet_table, run_blocking, write_parquet_table},
    validate_row,
};
use crate::{
    Result, StorageError,
    path::{validate_logical_path, validate_table_name},
};

/// Parquet-backed [`TableProvider`].
#[derive(Debug, Clone)]
pub struct ParquetTableProvider {
    root: PathBuf,
    namespace: PathBuf,
}

impl ParquetTableProvider {
    /// Create a Parquet provider rooted at `root`.
    ///
    /// # Errors
    ///
    /// This constructor currently only validates the root path shape and cannot
    /// fail for existing paths. Directories are created by write operations.
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        Ok(Self {
            root,
            namespace: PathBuf::new(),
        })
    }

    fn table_path(&self, table_name: &str) -> Result<PathBuf> {
        Ok(self
            .root
            .join(&self.namespace)
            .join(format!("{}.parquet", validate_table_name(table_name)?)))
    }
}

#[async_trait]
impl TableProvider for ParquetTableProvider {
    async fn read_table(&self, table_name: &str) -> Result<TableBatch> {
        let path = self.table_path(table_name)?;
        let table_name = table_name.to_owned();
        run_blocking("reading parquet table", move || {
            read_parquet_table(&path, &table_name)
        })
        .await
    }

    async fn write_table(&self, table_name: &str, batch: TableBatch) -> Result<()> {
        let path = self.table_path(table_name)?;
        run_blocking("writing parquet table", move || {
            write_parquet_table(&path, &batch)
        })
        .await
    }

    async fn has_table(&self, table_name: &str) -> Result<bool> {
        let path = self.table_path(table_name)?;
        tokio::fs::metadata(&path)
            .await
            .map(|metadata| metadata.is_file())
            .or_else(|source| {
                if source.kind() == std::io::ErrorKind::NotFound {
                    Ok(false)
                } else {
                    Err(StorageError::Filesystem { path, source })
                }
            })
    }

    async fn list_tables(&self) -> Result<Vec<String>> {
        let root = self.root.join(&self.namespace);
        let mut tables = Vec::new();
        if tokio::fs::try_exists(&root)
            .await
            .map_err(|source| StorageError::Filesystem {
                path: root.clone(),
                source,
            })?
        {
            let mut entries =
                tokio::fs::read_dir(&root)
                    .await
                    .map_err(|source| StorageError::Filesystem {
                        path: root.clone(),
                        source,
                    })?;
            while let Some(entry) =
                entries
                    .next_entry()
                    .await
                    .map_err(|source| StorageError::Filesystem {
                        path: root.clone(),
                        source,
                    })?
            {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("parquet")
                    && let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
                {
                    tables.push(stem.to_owned());
                }
            }
        }
        tables.sort();
        Ok(tables)
    }

    async fn open_table(
        &self,
        table_name: &str,
        schema: SchemaRef,
        truncate: bool,
    ) -> Result<Box<dyn Table>> {
        let path = self.table_path(table_name)?;
        let exists =
            tokio::fs::try_exists(&path)
                .await
                .map_err(|source| StorageError::Filesystem {
                    path: path.clone(),
                    source,
                })?;
        let rows = if truncate || !exists {
            Vec::new()
        } else {
            let table_name = table_name.to_owned();
            run_blocking("reading parquet table for append", {
                let path = path.clone();
                move || read_parquet_table(&path, &table_name)
            })
            .await?
            .rows()
            .to_vec()
        };

        Ok(Box::new(ParquetTable {
            path,
            schema,
            rows,
            closed: false,
        }))
    }

    fn child(&self, namespace: &str) -> Result<Arc<dyn TableProvider>> {
        let mut child_namespace = self.namespace.clone();
        child_namespace.push(validate_logical_path(namespace)?);
        Ok(Arc::new(Self {
            root: self.root.clone(),
            namespace: child_namespace,
        }))
    }
}

#[derive(Debug)]
struct ParquetTable {
    path: PathBuf,
    schema: SchemaRef,
    rows: Vec<TableRow>,
    closed: bool,
}

#[async_trait]
impl Table for ParquetTable {
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
        let path = self.path.clone();
        run_blocking("closing parquet table", move || {
            write_parquet_table(&path, &batch)
        })
        .await?;
        self.closed = true;
        Ok(())
    }

    async fn abort(&mut self) -> Result<()> {
        self.rows.clear();
        self.closed = true;
        Ok(())
    }
}
