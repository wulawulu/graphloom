use std::{future::poll_fn, sync::Arc};

use futures_core::Stream;
use graphloom_storage::{MemoryStorage, Storage};
use serde_json::json;
use tempfile::tempdir;

use super::{FileInputReader, InputReader, Result, TextDocument, TextFileReader};

async fn collect_documents<S>(stream: S) -> Vec<TextDocument>
where
    S: Stream<Item = Result<TextDocument>>,
{
    let mut stream = std::pin::pin!(stream);
    let mut documents = Vec::new();
    while let Some(document) = poll_fn(|cx| stream.as_mut().poll_next(cx)).await {
        documents.push(document.expect("document read should succeed"));
    }
    documents
}

#[tokio::test]
async fn test_should_read_default_txt_files_in_stable_order() {
    let storage = Arc::new(MemoryStorage::new());
    storage
        .set("b.txt", b"second")
        .await
        .expect("write should succeed");
    storage
        .set("a.txt", b"first")
        .await
        .expect("write should succeed");
    storage
        .set("ignored.md", b"ignored")
        .await
        .expect("write should succeed");

    let reader = TextFileReader::new(storage).expect("reader should initialize");
    let documents = collect_documents(reader.read_documents()).await;

    assert_eq!(documents.len(), 2);
    assert_eq!(documents[0].title, "a.txt");
    assert_eq!(documents[0].text, "first");
    assert_eq!(
        documents[0].id,
        "7fdd80dbdded156323d36c459e5fd133a4d888c227320cfb7042be9feb35d7f07201e535697af914e69d6f46b2a88655c86c2371288052ccd4fa92058b01d3fd"
    );
    assert_eq!(documents[1].title, "b.txt");
}

#[tokio::test]
async fn test_should_support_filesystem_reader_with_custom_pattern() {
    let temp = tempdir().expect("temporary directory should be created");
    tokio::fs::write(temp.path().join("a.md"), "first")
        .await
        .expect("write should succeed");

    let reader =
        FileInputReader::with_file_pattern(temp.path(), r".*\.md$").expect("reader should init");
    let documents = collect_documents(reader.read_documents()).await;

    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].title, "a.md");
}

#[test]
fn test_should_get_and_collect_standard_and_raw_fields() {
    let document = TextDocument {
        id: "id".to_owned(),
        text: "body".to_owned(),
        title: "title".to_owned(),
        creation_date: None,
        raw_data: Some(json!({"nested": {"field": 42}})),
    };

    assert_eq!(document.get("title"), Some(json!("title")));
    assert_eq!(document.get("nested.field"), Some(json!(42)));
    assert_eq!(document.get("creation_date"), None);
    assert_eq!(
        document.collect(&["nested.field".to_owned(), "missing".to_owned()]),
        vec![("nested.field".to_owned(), json!(42))]
    );
}
