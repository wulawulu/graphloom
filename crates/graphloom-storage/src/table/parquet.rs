use std::{fmt, io::Cursor, path::Path, sync::Arc};

use async_trait::async_trait;
use polars_core::{
    prelude::{CompatLevel, DataFrame},
    schema::SchemaExt,
};
use polars_io::{
    parquet::write::ParquetCompression,
    prelude::{KeyValueMetadata, ParquetReader, ParquetWriter, SerReader},
};
use polars_parquet::write::schema_to_metadata_key;

use super::{
    Table, TableProvider, append_optional_dataframe, id_column_index, next_dataframe_row,
    row_from_dataframe, row_matches_id, row_stream,
};
use crate::{FileStorage, Result, Storage, StorageError, path::validate_table_name};

/// Parquet-backed [`TableProvider`].
#[derive(Debug, Clone)]
pub struct ParquetTableProvider {
    storage: Arc<dyn Storage>,
}

impl ParquetTableProvider {
    /// Create a Parquet provider over a filesystem-backed storage root.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage root cannot be initialized.
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            storage: Arc::new(FileStorage::new(root)?),
        })
    }

    /// Create a Parquet provider over an existing object storage provider.
    #[must_use]
    pub fn from_storage(storage: Arc<dyn Storage>) -> Self {
        Self { storage }
    }

    fn table_object(table_name: &str) -> Result<String> {
        Ok(format!("{}.parquet", validate_table_name(table_name)?))
    }

    async fn read_optional(&self, table_name: &str) -> Result<Option<DataFrame>> {
        let object = Self::table_object(table_name)?;
        let Some(bytes) = self.storage.get(&object).await? else {
            return Ok(None);
        };
        run_blocking("reading parquet table", move || {
            read_parquet_dataframe(bytes)
        })
        .await
        .map(Some)
    }
}

#[async_trait]
impl TableProvider for ParquetTableProvider {
    async fn read_dataframe(&self, table_name: &str) -> Result<DataFrame> {
        self.read_optional(table_name)
            .await?
            .ok_or_else(|| StorageError::MissingTable {
                name: table_name.to_owned(),
            })
    }

    async fn read_optional_dataframe(&self, table_name: &str) -> Result<Option<DataFrame>> {
        self.read_optional(table_name).await
    }

    async fn write_dataframe(&self, table_name: &str, dataframe: DataFrame) -> Result<()> {
        let object = Self::table_object(table_name)?;
        let bytes = run_blocking("writing parquet table", move || {
            write_parquet_dataframe(dataframe)
        })
        .await?;
        self.storage.set(&object, &bytes).await
    }

    async fn has(&self, table_name: &str) -> Result<bool> {
        self.storage.has(&Self::table_object(table_name)?).await
    }

    async fn list(&self) -> Result<Vec<String>> {
        let mut tables = self
            .storage
            .find(r"\.parquet$")
            .await?
            .into_iter()
            .filter_map(|key| {
                key.strip_suffix(".parquet")
                    .map(std::borrow::ToOwned::to_owned)
            })
            .collect::<Vec<_>>();
        tables.sort();
        Ok(tables)
    }

    async fn open(&self, table_name: &str, truncate: bool) -> Result<Box<dyn Table>> {
        let object = Self::table_object(table_name)?;
        let existing = if truncate {
            None
        } else {
            self.read_optional(table_name).await?
        };

        Ok(Box::new(ParquetTable {
            storage: Arc::clone(&self.storage),
            object,
            existing,
            truncate,
            pending: DataFrame::empty(),
            read_index: 0,
            closed: false,
        }))
    }

    fn child(&self, namespace: Option<&str>) -> Result<Arc<dyn TableProvider>> {
        Ok(Arc::new(Self {
            storage: self.storage.child(namespace)?,
        }))
    }
}

struct ParquetTable {
    storage: Arc<dyn Storage>,
    object: String,
    existing: Option<DataFrame>,
    truncate: bool,
    pending: DataFrame,
    read_index: usize,
    closed: bool,
}

impl fmt::Debug for ParquetTable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParquetTable")
            .field("object", &self.object)
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
impl Table for ParquetTable {
    async fn write(&mut self, dataframe: DataFrame) -> Result<()> {
        self.check_open()?;
        self.pending =
            append_optional_dataframe(Some(std::mem::take(&mut self.pending)), &dataframe)?;
        Ok(())
    }

    fn length(&self) -> usize {
        self.existing.as_ref().map_or(0, DataFrame::height) + self.pending.height()
    }

    fn column_names(&self) -> Vec<String> {
        self.existing
            .as_ref()
            .filter(|dataframe| dataframe.width() > 0)
            .unwrap_or(&self.pending)
            .get_column_names_owned()
            .into_iter()
            .map(|name| name.to_string())
            .collect()
    }

    async fn close(&mut self) -> Result<()> {
        self.check_open()?;
        if self.pending.height() == 0 && (!self.truncate || self.pending.width() == 0) {
            self.closed = true;
            return Ok(());
        }

        let existing = if self.truncate {
            None
        } else {
            match self.storage.get(&self.object).await? {
                Some(bytes) => Some(
                    run_blocking("reading parquet table for append", move || {
                        read_parquet_dataframe(bytes)
                    })
                    .await?,
                ),
                None => None,
            }
        };
        let dataframe = append_optional_dataframe(existing, &self.pending)?;
        let bytes = run_blocking("closing parquet table", move || {
            write_parquet_dataframe(dataframe)
        })
        .await?;
        self.storage.set(&self.object, &bytes).await?;
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

impl ParquetTable {
    fn check_open(&self) -> Result<()> {
        if self.closed {
            return Err(StorageError::TableClosed {
                name: self.object.clone(),
            });
        }
        Ok(())
    }
}

async fn run_blocking<F, T>(operation: &'static str, task: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|source| StorageError::BlockingTask { operation, source })?
}

fn read_parquet_dataframe(bytes: Vec<u8>) -> Result<DataFrame> {
    ParquetReader::new(Cursor::new(bytes))
        .finish()
        .map_err(StorageError::Polars)
}

fn write_parquet_dataframe(mut dataframe: DataFrame) -> Result<Vec<u8>> {
    let mut buffer = Cursor::new(Vec::new());
    let arrow_schema = dataframe.schema().to_arrow(CompatLevel::oldest());
    let compatibility_metadata = schema_to_metadata_key(&arrow_schema);
    ParquetWriter::new(&mut buffer)
        .with_compression(ParquetCompression::Snappy)
        // Polars 0.54 exports String columns as Arrow Utf8View at its newest
        // compatibility level. Persisting that Arrow schema metadata makes
        // PyArrow 22 reconstruct list<string_view>, which pandas cannot
        // materialize. Persist the oldest compatible Arrow schema so Python
        // readers reconstruct canonical string/list<string> logical types.
        .with_key_value_metadata(Some(KeyValueMetadata::Static(vec![compatibility_metadata])))
        .finish(&mut dataframe)
        .map_err(StorageError::Polars)?;
    Ok(buffer.into_inner())
}

#[cfg(test)]
mod tests {
    use polars_core::prelude::{DataFrame, DataType, NamedFrom, Series};
    use polars_parquet::arrow::read::read_metadata;
    use tempfile::TempDir;

    use super::*;

    fn empty_embedding_dataframe() -> DataFrame {
        DataFrame::new(
            0,
            vec![
                Series::new_empty("id".into(), &DataType::String).into(),
                Series::new_empty(
                    "embedding".into(),
                    &DataType::List(Box::new(DataType::Float32)),
                )
                .into(),
            ],
        )
        .expect("empty dataframe")
    }

    fn one_embedding_dataframe() -> DataFrame {
        let rows = vec![Series::new("item".into(), [1.0_f32, 0.0])];
        DataFrame::new(
            1,
            vec![
                Series::new("id".into(), ["old"]).into(),
                Series::new("embedding".into(), rows).into(),
            ],
        )
        .expect("one-row dataframe")
    }

    #[test]
    fn test_should_write_python_compatible_arrow_schema_metadata() {
        let list_values = vec![Series::new("item".into(), ["alpha", "beta"])];
        let dataframe = DataFrame::new(
            1,
            vec![
                Series::new("name".into(), ["fixture"]).into(),
                Series::new("tags".into(), list_values).into(),
            ],
        )
        .expect("compatibility dataframe");
        let expected = schema_to_metadata_key(&dataframe.schema().to_arrow(CompatLevel::oldest()));
        let incompatible =
            schema_to_metadata_key(&dataframe.schema().to_arrow(CompatLevel::newest()));

        let bytes = write_parquet_dataframe(dataframe).expect("write parquet");
        let mut reader = Cursor::new(bytes);
        let metadata = read_metadata(&mut reader).expect("read parquet metadata");
        let actual = metadata
            .key_value_metadata()
            .as_ref()
            .and_then(|items| items.iter().find(|item| item.key == expected.key))
            .expect("ARROW:schema metadata");

        assert_ne!(expected.value, incompatible.value);
        assert_eq!(actual.value, expected.value);
    }

    #[tokio::test]
    async fn test_should_commit_empty_truncate_table_with_schema() {
        let tempdir = TempDir::new().expect("tempdir");
        let provider = ParquetTableProvider::new(tempdir.path()).expect("provider");
        provider
            .write_dataframe("embeddings", one_embedding_dataframe())
            .await
            .expect("seed");

        let mut table = provider.open("embeddings", true).await.expect("open");
        table
            .write(empty_embedding_dataframe())
            .await
            .expect("write empty schema");
        table.close().await.expect("close");

        let dataframe = provider
            .read_dataframe("embeddings")
            .await
            .expect("empty table should exist");
        assert_eq!(dataframe.height(), 0);
        assert_eq!(
            dataframe
                .get_column_names()
                .iter()
                .map(|name| name.as_str())
                .collect::<Vec<_>>(),
            vec!["id", "embedding"]
        );
        assert_eq!(
            dataframe.column("embedding").expect("embedding").dtype(),
            &DataType::List(Box::new(DataType::Float32))
        );
    }
}
