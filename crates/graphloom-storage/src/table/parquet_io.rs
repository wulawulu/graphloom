use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use parquet::arrow::{ArrowWriter, arrow_reader::ParquetRecordBatchReaderBuilder};

use super::TableBatch;
use crate::{Result, StorageError};

pub(super) async fn run_blocking<F, T>(operation: &'static str, task: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|source| StorageError::BlockingTask { operation, source })?
}

#[allow(
    clippy::disallowed_types,
    reason = "parquet::arrow reader requires a std::fs::File handle"
)]
pub(super) fn read_parquet_table(path: &Path, table_name: &str) -> Result<TableBatch> {
    if !path.exists() {
        return Err(StorageError::MissingTable {
            name: table_name.to_owned(),
        });
    }

    let file = std::fs::File::open(path).map_err(|source| StorageError::Filesystem {
        path: path.to_path_buf(),
        source,
    })?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let schema = builder.schema().clone();
    let reader = builder.build()?;
    let mut rows = Vec::new();

    for batch in reader {
        let batch = batch?;
        let mut batch = TableBatch::try_from(batch)?.rows().to_vec();
        rows.append(&mut batch);
    }

    TableBatch::try_new(schema, rows)
}

#[allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "parquet::ArrowWriter and atomic replacement require std fs handles inside \
              spawn_blocking"
)]
pub(super) fn write_parquet_table(path: &Path, batch: &TableBatch) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| StorageError::Filesystem {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let tmp_path = temporary_table_path(path)?;
    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&tmp_path)
        .map_err(|source| StorageError::Filesystem {
            path: tmp_path.clone(),
            source,
        })?;
    let writer_file = file
        .try_clone()
        .map_err(|source| StorageError::Filesystem {
            path: tmp_path.clone(),
            source,
        })?;
    let record_batch = batch.to_record_batch()?;
    let mut writer = ArrowWriter::try_new(writer_file, batch.schema(), None)?;
    writer.write(&record_batch)?;
    writer.close()?;
    file.sync_all().map_err(|source| StorageError::Filesystem {
        path: tmp_path.clone(),
        source,
    })?;

    std::fs::rename(&tmp_path, path).map_err(|source| StorageError::Filesystem {
        path: path.to_path_buf(),
        source,
    })?;
    sync_parent(path)
}

fn temporary_table_path(path: &Path) -> Result<PathBuf> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| StorageError::InvalidPath {
            path: path.display().to_string(),
            reason: "table path does not have a UTF-8 file name",
        })?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| StorageError::InvalidPath {
            path: path.display().to_string(),
            reason: if source.duration().is_zero() {
                "system clock is before Unix epoch"
            } else {
                "system clock error"
            },
        })?
        .as_nanos();
    Ok(parent.join(format!(".{stem}.{}.{}.tmp", std::process::id(), nanos)))
}

#[allow(
    clippy::disallowed_types,
    reason = "directory fsync requires std::fs::File on Unix-like platforms"
)]
fn sync_parent(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };

    let directory = std::fs::File::open(parent).map_err(|source| StorageError::Filesystem {
        path: parent.to_path_buf(),
        source,
    })?;
    directory
        .sync_all()
        .map_err(|source| StorageError::Filesystem {
            path: parent.to_path_buf(),
            source,
        })
}
