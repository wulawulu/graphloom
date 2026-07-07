use std::sync::Arc;

use futures_util::StreamExt;
use polars_core::{frame::row::Row, prelude::*};
use tempfile::tempdir;

use crate::{
    FileStorage, MemoryStorage, MemoryTableProvider, ParquetTableProvider, Storage, StorageError,
    TableProvider,
};

fn demo_dataframe(id: &str) -> DataFrame {
    df!(
        "id" => &[id],
        "human_readable_id" => &[0i64],
        "text" => &[format!("document {id}")]
    )
    .expect("demo dataframe should be valid")
}

fn row_id(row: &Row<'static>) -> Option<String> {
    row.0.first().and_then(|value| match value {
        AnyValue::String(value) => Some((*value).to_owned()),
        AnyValue::StringOwned(value) => Some(value.to_string()),
        _ => None,
    })
}

#[tokio::test]
async fn test_should_reject_file_storage_path_traversal() {
    let temp = tempdir().expect("temporary directory should be created");
    let storage = FileStorage::new(temp.path()).expect("file storage should initialize");

    let result = storage.set("../escape", b"bad").await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_should_return_storage_miss_for_missing_get() {
    let storage = MemoryStorage::new();

    let result = storage
        .get("missing.bin")
        .await
        .expect("get should succeed");

    assert_eq!(result, None);
}

#[tokio::test]
async fn test_should_round_trip_storage_text_and_bytes() {
    let storage = MemoryStorage::new();

    storage
        .set("bytes.bin", b"bytes")
        .await
        .expect("bytes should write");
    storage
        .set_text("text.txt", "hello")
        .await
        .expect("text should write");

    assert_eq!(
        storage
            .get("bytes.bin")
            .await
            .expect("bytes should read")
            .expect("bytes should exist"),
        b"bytes"
    );
    assert_eq!(
        storage
            .get_text("text.txt")
            .await
            .expect("text should read"),
        Some("hello".to_owned())
    );
}

#[tokio::test]
async fn test_should_find_storage_objects_by_regex_and_return_keys() {
    let storage = MemoryStorage::new();
    storage.set("root.txt", b"root").await.expect("write root");
    storage
        .set("nested/one.txt", b"nested")
        .await
        .expect("write nested");
    storage.set("data.bin", b"data").await.expect("write data");

    assert_eq!(
        storage.keys().await.expect("keys should succeed"),
        vec!["data.bin".to_owned(), "root.txt".to_owned()]
    );
    assert_eq!(
        storage.find(r"\.txt$").await.expect("find should succeed"),
        vec!["nested/one.txt".to_owned(), "root.txt".to_owned()]
    );
}

#[tokio::test]
async fn test_should_return_current_storage_for_child_none() {
    let storage = MemoryStorage::new();
    let current = storage.child(None).expect("child none should succeed");

    current
        .set("same.txt", b"same")
        .await
        .expect("write through current view");

    assert!(
        storage
            .has("same.txt")
            .await
            .expect("root has should succeed")
    );
}

#[tokio::test]
async fn test_should_isolate_memory_storage_child_data() {
    let storage = MemoryStorage::new();
    storage
        .set("root.txt", b"root")
        .await
        .expect("root write should succeed");
    let child = storage
        .child(Some("snapshots"))
        .expect("child namespace is valid");

    child
        .set("child.txt", b"child")
        .await
        .expect("child write should succeed");

    assert!(
        !child
            .has("root.txt")
            .await
            .expect("child has should succeed")
    );
    assert!(
        !storage
            .has("child.txt")
            .await
            .expect("root has should succeed")
    );
}

#[tokio::test]
async fn test_should_clear_file_storage_contents_but_keep_namespace_dir() {
    let temp = tempdir().expect("temporary directory should be created");
    let storage = FileStorage::new(temp.path()).expect("file storage should initialize");
    let child = storage
        .child(Some("snapshots"))
        .expect("child namespace is valid");

    child
        .set("nested/graph.graphml", b"graph")
        .await
        .expect("file write should succeed");
    child.clear().await.expect("clear should succeed");

    assert!(temp.path().join("snapshots").is_dir());
    assert!(
        !child
            .has("nested/graph.graphml")
            .await
            .expect("has should succeed")
    );
}

#[tokio::test]
async fn test_should_round_trip_memory_table_provider_dataframe() {
    let provider = MemoryTableProvider::new();
    let dataframe = demo_dataframe("doc-1");

    provider
        .write_dataframe("documents", dataframe.clone())
        .await
        .expect("memory table write should succeed");
    let read = provider
        .read_dataframe("documents")
        .await
        .expect("memory table read should succeed");

    assert!(read.equals_missing(&dataframe));
}

#[tokio::test]
async fn test_should_round_trip_parquet_provider_through_storage_object() {
    let storage = Arc::new(MemoryStorage::new());
    let provider = ParquetTableProvider::from_storage(storage.clone());
    let dataframe = demo_dataframe("doc-1");

    provider
        .write_dataframe("documents", dataframe.clone())
        .await
        .expect("parquet table write should succeed");

    assert!(
        storage
            .has("documents.parquet")
            .await
            .expect("storage has should succeed")
    );
    let read = provider
        .read_dataframe("documents")
        .await
        .expect("parquet table read should succeed");
    assert!(read.equals_missing(&dataframe));
}

#[tokio::test]
async fn test_should_list_parquet_tables_using_storage_find() {
    let storage = Arc::new(MemoryStorage::new());
    let provider = ParquetTableProvider::from_storage(storage.clone());

    provider
        .write_dataframe("documents", demo_dataframe("doc-1"))
        .await
        .expect("parquet table write should succeed");
    storage
        .set("notes.txt", b"not a table")
        .await
        .expect("non-table write should succeed");

    assert_eq!(
        provider.list().await.expect("list should succeed"),
        vec!["documents".to_owned()]
    );
}

#[tokio::test]
async fn test_should_namespace_parquet_provider_child() {
    let temp = tempdir().expect("temporary directory should be created");
    let provider =
        ParquetTableProvider::new(temp.path()).expect("parquet provider should initialize");
    let child = provider
        .child(Some("snapshots"))
        .expect("child provider should initialize");

    child
        .write_dataframe("documents", demo_dataframe("doc-1"))
        .await
        .expect("child write should succeed");

    assert!(
        !provider
            .has("documents")
            .await
            .expect("root has should succeed")
    );
    assert!(
        child
            .has("documents")
            .await
            .expect("child has should succeed")
    );
}

#[tokio::test]
async fn test_should_not_create_parquet_file_on_empty_close() {
    let storage = Arc::new(MemoryStorage::new());
    let provider = ParquetTableProvider::from_storage(storage.clone());
    let mut table = provider
        .open("documents", true)
        .await
        .expect("table should open");

    table.close().await.expect("table should close");

    assert!(
        !storage
            .has("documents.parquet")
            .await
            .expect("storage has should succeed")
    );
}

#[tokio::test]
async fn test_should_append_parquet_rows_by_concatenating_existing_dataframe() {
    let storage = Arc::new(MemoryStorage::new());
    let provider = ParquetTableProvider::from_storage(storage);

    provider
        .write_dataframe("documents", demo_dataframe("doc-1"))
        .await
        .expect("initial write should succeed");
    let mut table = provider
        .open("documents", false)
        .await
        .expect("append table should open");
    table
        .write(demo_dataframe("doc-2"))
        .await
        .expect("row should write");
    table.close().await.expect("table should close");

    let read = provider
        .read_dataframe("documents")
        .await
        .expect("read should succeed");
    assert_eq!(read.height(), 2);
}

#[tokio::test]
async fn test_should_truncate_parquet_table_when_opened_with_truncate_true() {
    let storage = Arc::new(MemoryStorage::new());
    let provider = ParquetTableProvider::from_storage(storage);

    provider
        .write_dataframe("documents", demo_dataframe("doc-1"))
        .await
        .expect("initial write should succeed");
    let mut table = provider
        .open("documents", true)
        .await
        .expect("truncate table should open");
    table
        .write(demo_dataframe("doc-2"))
        .await
        .expect("row should write");
    table.close().await.expect("table should close");

    let read = provider
        .read_dataframe("documents")
        .await
        .expect("read should succeed");
    assert_eq!(read.height(), 1);
    let table = provider
        .open("documents", false)
        .await
        .expect("table should open");
    assert!(table.has("doc-2").await.expect("has should succeed"));
    assert!(!table.has("doc-1").await.expect("has should succeed"));
}

#[tokio::test]
async fn test_should_truncate_memory_table_when_opened_with_truncate_true() {
    let provider = MemoryTableProvider::new();

    provider
        .write_dataframe("documents", demo_dataframe("doc-1"))
        .await
        .expect("initial write should succeed");
    let mut table = provider
        .open("documents", true)
        .await
        .expect("truncate table should open");
    table
        .write(demo_dataframe("doc-2"))
        .await
        .expect("row should write");
    table.close().await.expect("table should close");

    let table = provider
        .open("documents", false)
        .await
        .expect("table should open");
    assert_eq!(table.length(), 1);
    assert!(table.has("doc-2").await.expect("has should succeed"));
    assert!(!table.has("doc-1").await.expect("has should succeed"));
}

#[tokio::test]
async fn test_should_stream_table_rows_report_length_and_has_row_id() {
    let provider = MemoryTableProvider::new();
    provider
        .write_dataframe("documents", demo_dataframe("doc-1"))
        .await
        .expect("initial rows should write");
    let mut table = provider
        .open("documents", false)
        .await
        .expect("table should open");
    table
        .write(demo_dataframe("doc-2"))
        .await
        .expect("row should write");

    assert_eq!(table.length(), 2);
    assert!(table.has("doc-1").await.expect("has should succeed"));
    assert!(table.has("doc-2").await.expect("has should succeed"));
    let mut rows = table.rows();
    let first = rows
        .next()
        .await
        .expect("first row should stream")
        .expect("first row should be valid");
    let second = rows
        .next()
        .await
        .expect("second row should stream")
        .expect("second row should be valid");

    assert_eq!(row_id(&first), Some("doc-1".to_owned()));
    assert_eq!(row_id(&second), Some("doc-2".to_owned()));
    assert!(rows.next().await.is_none());
}

#[tokio::test]
async fn test_should_return_table_closed_error_after_close() {
    let provider = MemoryTableProvider::new();
    let mut table = provider
        .open("documents", true)
        .await
        .expect("table should open");

    table.close().await.expect("table should close");
    let result = table.write(demo_dataframe("doc-1")).await;

    assert!(matches!(result, Err(StorageError::TableClosed { .. })));
}
