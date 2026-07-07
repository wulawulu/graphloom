use std::{pin::Pin, sync::Arc};

use futures_util::{Stream, stream};
use graphloom_input::{DocumentStream, InputReader, TextDocument, gen_sha512_hash};
use graphloom_storage::{MemoryTableProvider, TableProvider};
use serde_json::json;

use crate::{
    GraphRagConfig, PipelineFactory, PipelineRunContext, WorkflowRegistry, register_step5_workflows,
};

#[test]
fn test_should_deserialize_chunking_encoding_model_and_keep_future_sections() {
    assert_eq!(
        GraphRagConfig::default().chunking.encoding_model,
        "o200k_base"
    );

    let config = serde_json::from_value::<GraphRagConfig>(json!({
        "chunking": {
            "encoding_model": "o200k_base",
            "size": 64,
            "overlap": 8,
            "prepend_metadata": ["title"],
        },
        "local_search": {
            "enabled": true,
        },
    }))
    .expect("config should deserialize");

    assert_eq!(config.chunking.encoding_model, "o200k_base");
    assert_eq!(config.chunking.size.get(), 64);
    assert_eq!(config.chunking.overlap, 8);
    assert_eq!(config.chunking.prepend_metadata, vec!["title"]);
    assert_eq!(config.sections["local_search"]["enabled"], true);
}

#[derive(Debug)]
struct MemoryInputReader {
    documents: Vec<TextDocument>,
}

impl InputReader for MemoryInputReader {
    fn read_documents(&self) -> DocumentStream<'_> {
        let documents = self
            .documents
            .clone()
            .into_iter()
            .map(Ok)
            .collect::<Vec<_>>();
        Box::pin(stream::iter(documents)) as Pin<Box<dyn Stream<Item = _> + Send + '_>>
    }
}

#[tokio::test]
async fn test_should_run_step5_pipeline_and_populate_documents() {
    let provider = Arc::new(MemoryTableProvider::new());
    let input_text = "Alice met Bob. Alice works with Bob.";
    let reader = Arc::new(MemoryInputReader {
        documents: vec![TextDocument::new(
            gen_sha512_hash([input_text]),
            input_text.to_owned(),
            "doc.txt".to_owned(),
            Some("2026-07-07T00:00:00Z".to_owned()),
            Some(json!({"source": "unit-test"})),
        )],
    });
    let mut context = PipelineRunContext::new(provider.clone()).with_input_reader(reader);
    let mut registry = WorkflowRegistry::new();
    register_step5_workflows(&mut registry);
    let config = GraphRagConfig::default();
    let pipeline = PipelineFactory::new(registry)
        .standard(&config)
        .expect("standard pipeline should be created");

    let outputs = pipeline
        .run(&config, &mut context)
        .await
        .expect("step5 pipeline should run");

    assert_eq!(outputs.len(), 3);
    assert_eq!(context.stats.document_count, 1);
    assert!(context.stats.text_unit_count >= 1);

    let documents = provider
        .read_dataframe("documents")
        .await
        .expect("documents table should exist");
    let text_units = provider
        .read_dataframe("text_units")
        .await
        .expect("text_units table should exist");
    assert_eq!(documents.height(), 1);
    assert_eq!(text_units.height(), context.stats.text_unit_count);
    assert_eq!(
        text_units
            .column("id")
            .expect("id column should exist")
            .str()
            .expect("id should be string")
            .get(0)
            .expect("first text unit id should exist"),
        gen_sha512_hash([text_units
            .column("text")
            .expect("text column should exist")
            .str()
            .expect("text should be string")
            .get(0)
            .expect("first text unit text should exist")])
    );
}

#[tokio::test]
async fn test_should_fail_when_no_documents_are_read() {
    let provider = Arc::new(MemoryTableProvider::new());
    let reader = Arc::new(MemoryInputReader {
        documents: Vec::new(),
    });
    let mut context = PipelineRunContext::new(provider).with_input_reader(reader);
    let mut registry = WorkflowRegistry::new();
    register_step5_workflows(&mut registry);
    let config = GraphRagConfig::default();
    let pipeline = PipelineFactory::new(registry)
        .standard(&config)
        .expect("standard pipeline should be created");

    let result = pipeline.run(&config, &mut context).await;

    assert!(result.is_err());
}
