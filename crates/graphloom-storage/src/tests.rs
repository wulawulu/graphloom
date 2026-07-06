use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Schema};
use tempfile::tempdir;

use crate::{
    FileStorage, MemoryStorage, MemoryTableProvider, ParquetTableProvider, Storage, TableBatch,
    TableProvider, TableRow, TableValue,
};

fn demo_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("human_readable_id", DataType::Int64, false),
        Field::new(
            "text_unit_ids",
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
            false,
        ),
    ]))
}

fn demo_row() -> TableRow {
    TableRow::from([
        ("id".to_owned(), TableValue::String("doc-1".to_owned())),
        ("human_readable_id".to_owned(), TableValue::Int(0)),
        (
            "text_unit_ids".to_owned(),
            TableValue::List(vec![
                TableValue::String("tu-1".to_owned()),
                TableValue::String("tu-2".to_owned()),
            ]),
        ),
    ])
}

#[tokio::test]
async fn test_should_reject_file_storage_path_traversal() {
    let temp = tempdir().expect("temporary directory should be created");
    let storage = FileStorage::new(temp.path()).expect("file storage should initialize");

    let result = storage.set("../escape", b"bad").await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_should_namespace_memory_storage() {
    let storage = MemoryStorage::new();
    let child = storage
        .child("snapshots")
        .expect("child namespace is valid");

    child
        .set("graph.graphml", b"graph")
        .await
        .expect("memory write should succeed");

    assert!(
        child
            .has("graph.graphml")
            .await
            .expect("memory has should succeed")
    );
    assert!(
        !storage
            .has("graph.graphml")
            .await
            .expect("root lookup should succeed")
    );
}

#[tokio::test]
async fn test_should_round_trip_memory_table_provider() {
    let provider = MemoryTableProvider::new();
    let batch =
        TableBatch::try_new(demo_schema(), vec![demo_row()]).expect("demo batch should be valid");

    provider
        .write_table("documents", batch.clone())
        .await
        .expect("memory table write should succeed");
    let read = provider
        .read_table("documents")
        .await
        .expect("memory table read should succeed");

    assert_eq!(read, batch);
}

#[tokio::test]
async fn test_should_round_trip_parquet_table_provider() {
    let temp = tempdir().expect("temporary directory should be created");
    let provider =
        ParquetTableProvider::new(temp.path()).expect("parquet provider should initialize");
    let batch =
        TableBatch::try_new(demo_schema(), vec![demo_row()]).expect("demo batch should be valid");

    provider
        .write_table("documents", batch.clone())
        .await
        .expect("parquet table write should succeed");
    let read = provider
        .read_table("documents")
        .await
        .expect("parquet table read should succeed");

    assert_eq!(read.schema().as_ref(), batch.schema().as_ref());
    assert_eq!(read.rows(), batch.rows());
}

#[tokio::test]
async fn test_should_commit_streaming_table_on_close() {
    let provider = MemoryTableProvider::new();
    let mut table = provider
        .open_table("documents", demo_schema(), true)
        .await
        .expect("table should open");

    table.append(demo_row()).await.expect("row should append");
    table.close().await.expect("table should close");

    assert!(
        provider
            .has_table("documents")
            .await
            .expect("has table should succeed")
    );
}
