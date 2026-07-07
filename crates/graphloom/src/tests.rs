use std::{pin::Pin, sync::Arc};

use futures_util::{Stream, stream};
use graphloom_input::{DocumentStream, InputReader, TextDocument, gen_sha512_hash};
use graphloom_llm::MockCompletionModel;
use graphloom_storage::{MemoryStorage, MemoryTableProvider, Storage, TableProvider};
use polars_core::prelude::*;
use serde_json::json;

use crate::{
    EXTRACT_GRAPH_WORKFLOW, FINALIZE_GRAPH_WORKFLOW, GraphRagConfig, PipelineFactory,
    PipelineRunContext, WorkflowRegistry, register_step5_workflows, register_step6_workflows,
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
    assert_eq!(config.extract_graph.max_gleanings, 1);
    assert_eq!(
        config.summarize_descriptions.model_instance_name,
        "summarize_descriptions",
    );
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
    let config = GraphRagConfig {
        workflows: crate::workflows::STEP5_WORKFLOWS
            .iter()
            .map(|workflow| (*workflow).to_owned())
            .collect(),
        ..Default::default()
    };
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
    let config = GraphRagConfig {
        workflows: crate::workflows::STEP5_WORKFLOWS
            .iter()
            .map(|workflow| (*workflow).to_owned())
            .collect(),
        ..Default::default()
    };
    let pipeline = PipelineFactory::new(registry)
        .standard(&config)
        .expect("standard pipeline should be created");

    let result = pipeline.run(&config, &mut context).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_should_extract_and_finalize_graph_with_summaries_and_graphml() {
    let provider = Arc::new(MemoryTableProvider::new());
    provider
        .write_dataframe(
            "text_units",
            df!(
                "id" => ["tu-1", "tu-2"],
                "text" => ["Alice works with Bob.", "Alice mentors Bob."],
            )
            .expect("text units dataframe should build"),
        )
        .await
        .expect("text_units should write");
    let storage = Arc::new(MemoryStorage::new());
    let model = Arc::new(MockCompletionModel::new(
        "default_completion_model",
        vec![
            "(\"entity\"<|>Alice<|>person<|>Alice is an \
             engineer)##(\"entity\"<|>Bob<|>person<|>Bob is a \
             researcher)##(\"relationship\"<|>Alice<|>Bob<|>Alice works with \
             Bob<|>2)##<|COMPLETE|>"
                .to_owned(),
            "(\"entity\"<|>Alice<|>person<|>Alice mentors \
             teams)##(\"entity\"<|>Bob<|>person<|>Bob studies \
             graphs)##(\"relationship\"<|>Alice<|>Bob<|>Alice mentors Bob<|>3)##<|COMPLETE|>"
                .to_owned(),
            "Alice is an engineer who mentors teams.".to_owned(),
            "Bob is a researcher who studies graphs.".to_owned(),
            "Alice works with and mentors Bob.".to_owned(),
        ],
    ));
    let mut context = PipelineRunContext::new(provider.clone())
        .with_completion_model("default_completion_model", model);
    context.output_storage = Some(storage.clone());
    let mut registry = WorkflowRegistry::new();
    register_step6_workflows(&mut registry);
    let mut config = GraphRagConfig::default();
    config.extract_graph.max_gleanings = 0;
    config.snapshots.raw_graph = true;
    config.snapshots.graphml = true;
    config.workflows = vec![
        EXTRACT_GRAPH_WORKFLOW.to_owned(),
        FINALIZE_GRAPH_WORKFLOW.to_owned(),
    ];
    let pipeline = PipelineFactory::new(registry)
        .standard(&config)
        .expect("step6 pipeline should be created");

    let outputs = pipeline
        .run(&config, &mut context)
        .await
        .expect("step6 pipeline should run");

    assert_eq!(outputs.len(), 2);
    assert_eq!(context.stats.entity_count, 2);
    assert_eq!(context.stats.relationship_count, 1);
    assert!(provider.has("raw_entities").await.expect("has should work"));
    assert!(
        provider
            .has("raw_relationships")
            .await
            .expect("has should work")
    );

    let entities = provider
        .read_dataframe("entities")
        .await
        .expect("entities should exist");
    let relationships = provider
        .read_dataframe("relationships")
        .await
        .expect("relationships should exist");
    assert_eq!(entities.height(), 2);
    assert_eq!(relationships.height(), 1);
    assert_eq!(
        entities
            .column("human_readable_id")
            .expect("human id should exist")
            .i64()
            .expect("human id should be i64")
            .get(0),
        Some(0),
    );
    assert_eq!(
        entities
            .column("degree")
            .expect("degree should exist")
            .i64()
            .expect("degree should be i64")
            .get(0),
        Some(1),
    );
    assert_eq!(
        relationships
            .column("weight")
            .expect("weight should exist")
            .f64()
            .expect("weight should be f64")
            .get(0),
        Some(5.0),
    );
    assert_eq!(
        relationships
            .column("combined_degree")
            .expect("combined degree should exist")
            .i64()
            .expect("combined degree should be i64")
            .get(0),
        Some(2),
    );
    let graphml = storage
        .get_text("graph.graphml")
        .await
        .expect("graphml read should succeed")
        .expect("graphml should exist");
    assert!(graphml.contains(r#"<node id="ALICE"/>"#));
    assert!(graphml.contains(r#"<edge source="ALICE" target="BOB">"#));
}
