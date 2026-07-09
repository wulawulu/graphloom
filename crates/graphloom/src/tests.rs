use std::{
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use futures_util::{Stream, stream};
use graphloom_input::{DocumentStream, InputReader, TextDocument, gen_sha512_hash};
use graphloom_llm::{
    CompletionModel, CompletionRequest, CompletionResponse, EmbeddingModel, EmbeddingRequest,
    EmbeddingResponse, MockCompletionModel,
};
use graphloom_storage::{MemoryStorage, MemoryTableProvider, Storage, TableProvider};
use graphloom_vectors::{LanceDbVectorStore, VectorStore, VectorStoreConfig};
use polars_core::prelude::*;
use serde_json::json;
use tempfile::TempDir;

use crate::{
    CREATE_COMMUNITIES_WORKFLOW, CREATE_COMMUNITY_REPORTS_WORKFLOW,
    CREATE_FINAL_TEXT_UNITS_WORKFLOW, EXTRACT_COVARIATES_WORKFLOW, EXTRACT_GRAPH_WORKFLOW,
    FINALIZE_GRAPH_WORKFLOW, GENERATE_TEXT_EMBEDDINGS_WORKFLOW, GraphRagConfig, PipelineFactory,
    PipelineRunContext, WorkflowRegistry, register_step5_workflows, register_step6_workflows,
    register_step7_workflows, register_step8_workflows, register_step9_workflows,
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
        "async_mode": "asyncio",
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
    assert_eq!(config.input.input_type, "text");
    assert_eq!(config.input_storage.storage_type, "file");
    assert_eq!(config.input_storage.base_dir, "input");
    assert_eq!(config.output_storage.storage_type, "file");
    assert_eq!(config.output_storage.base_dir, "output");
    assert_eq!(config.reporting.reporting_type, "file");
    assert_eq!(config.reporting.base_dir, "logs");
    assert_eq!(config.cache.cache_type, "json");
    assert_eq!(config.cache.storage.storage_type, "file");
    assert_eq!(config.cache.storage.base_dir, "cache");
    assert_eq!(
        config.summarize_descriptions.model_instance_name,
        "summarize_descriptions",
    );
    assert!(!config.extract_claims.enabled);
    assert_eq!(config.extract_claims.model_instance_name, "extract_claims");
    assert_eq!(config.cluster_graph.max_cluster_size, 10);
    assert!(config.cluster_graph.use_lcc);
    assert_eq!(config.cluster_graph.seed, 0xDEAD_BEEF);
    assert_eq!(
        config.community_reports.completion_model_id,
        "default_completion_model"
    );
    assert_eq!(
        config.community_reports.model_instance_name,
        "community_reporting"
    );
    assert_eq!(config.community_reports.max_length, 2_000);
    assert_eq!(config.community_reports.max_input_length, 8_000);
    assert_eq!(config.sections["async_mode"], "asyncio");
    assert_eq!(config.sections["local_search"]["enabled"], true);
}

#[test]
fn test_should_deserialize_graphrag_storage_cache_and_query_sections() {
    let config = serde_yaml::from_str::<GraphRagConfig>(
        r"
input:
  type: file
input_storage:
  type: file
  base_dir: input_data
output_storage:
  type: file
  base_dir: output_data
reporting:
  type: file
  base_dir: log_data
cache:
  type: none
  storage:
    type: file
    base_dir: cache_data
local_search:
  prompt: prompts/local_search_system_prompt.txt
",
    )
    .expect("config should deserialize");

    assert_eq!(config.input.input_type, "file");
    assert_eq!(config.input_storage.base_dir, "input_data");
    assert_eq!(config.output_storage.base_dir, "output_data");
    assert_eq!(config.reporting.base_dir, "log_data");
    assert_eq!(config.cache.cache_type, "none");
    assert_eq!(config.cache.storage.base_dir, "cache_data");
    assert_eq!(
        config.sections["local_search"]["prompt"],
        "prompts/local_search_system_prompt.txt"
    );
}

#[test]
fn test_should_deserialize_community_reports_camel_and_snake_case() {
    let camel = serde_json::from_value::<GraphRagConfig>(json!({
        "communityReports": {
            "completionModelId": "chat",
            "modelInstanceName": "community_reporting",
            "graphPrompt": "prompts/community_report.txt",
            "textPrompt": "prompts/community_report_text.txt",
            "maxLength": 123,
            "maxInputLength": 456
        }
    }))
    .expect("camel case config should deserialize");
    assert_eq!(camel.community_reports.completion_model_id, "chat");
    assert_eq!(
        camel.community_reports.graph_prompt.as_deref(),
        Some("prompts/community_report.txt")
    );
    assert_eq!(camel.community_reports.max_length, 123);
    assert_eq!(camel.community_reports.max_input_length, 456);

    let snake = serde_json::from_value::<GraphRagConfig>(json!({
        "community_reports": {
            "completion_model_id": "chat",
            "model_instance_name": "reports",
            "graph_prompt": "graph.txt",
            "text_prompt": "text.txt",
            "max_length": 321,
            "max_input_length": 654
        }
    }))
    .expect("snake case config should deserialize");
    assert_eq!(snake.community_reports.model_instance_name, "reports");
    assert_eq!(
        snake.community_reports.text_prompt.as_deref(),
        Some("text.txt")
    );
    assert_eq!(snake.community_reports.max_length, 321);
    assert_eq!(snake.community_reports.max_input_length, 654);
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

fn string_list_column(name: &str, values: &[Vec<String>]) -> Column {
    let series = values
        .iter()
        .map(|values| Series::new(name.into(), values.as_slice()))
        .collect::<Vec<_>>();
    Series::new(name.into(), series).into()
}

fn i64_list_column(name: &str, values: &[Vec<i64>]) -> Column {
    let series = values
        .iter()
        .map(|values| Series::new(name.into(), values.as_slice()))
        .collect::<Vec<_>>();
    Series::new(name.into(), series).into()
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

#[tokio::test]
async fn test_should_run_step7_covariates_communities_and_final_text_units() {
    let provider = Arc::new(MemoryTableProvider::new());
    let mut text_units = df!(
        "id" => ["tu-1"],
        "human_readable_id" => [7i64],
        "text" => ["Alice reports Bob."],
        "n_tokens" => [4i64],
        "document_id" => ["doc-1"],
    )
    .expect("text units dataframe should build");
    text_units
        .with_column(string_list_column("entity_ids", &[Vec::<String>::new()]))
        .expect("entity ids column should build");
    text_units
        .with_column(string_list_column(
            "relationship_ids",
            &[Vec::<String>::new()],
        ))
        .expect("relationship ids column should build");
    text_units
        .with_column(string_list_column("covariate_ids", &[Vec::<String>::new()]))
        .expect("covariate ids column should build");
    provider
        .write_dataframe("text_units", text_units)
        .await
        .expect("text_units should write");
    let mut entities = df!(
        "id" => ["entity-a", "entity-b"],
        "human_readable_id" => [0i64, 1i64],
        "title" => ["ALICE", "BOB"],
        "type" => ["person", "person"],
        "description" => ["Alice", "Bob"],
        "frequency" => [1i64, 1i64],
        "degree" => [1i64, 1i64],
    )
    .expect("entities dataframe should build");
    entities
        .insert_column(
            5,
            string_list_column(
                "text_unit_ids",
                &[vec!["tu-1".to_owned()], vec!["tu-1".to_owned()]],
            ),
        )
        .expect("entity text unit ids column should build");
    provider
        .write_dataframe("entities", entities)
        .await
        .expect("entities should write");
    let mut relationships = df!(
        "id" => ["rel-1"],
        "human_readable_id" => [0i64],
        "source" => ["ALICE"],
        "target" => ["BOB"],
        "description" => ["Alice reports Bob"],
        "weight" => [1.0f64],
        "combined_degree" => [2i64],
    )
    .expect("relationships dataframe should build");
    relationships
        .with_column(string_list_column(
            "text_unit_ids",
            &[vec!["tu-1".to_owned()]],
        ))
        .expect("relationship text unit ids column should build");
    provider
        .write_dataframe("relationships", relationships)
        .await
        .expect("relationships should write");

    let model = Arc::new(MockCompletionModel::new(
        "default_completion_model",
        vec![
            "(ALICE<|>BOB<|>REPORT<|>TRUE<|>2026-07-07<|>2026-07-07<|>Alice reports Bob<|>\"Alice \
             reports Bob\")##<|COMPLETE|>"
                .to_owned(),
        ],
    ));
    let mut context = PipelineRunContext::new(provider.clone())
        .with_completion_model("default_completion_model", model);
    let mut registry = WorkflowRegistry::new();
    register_step7_workflows(&mut registry);
    let mut config = GraphRagConfig::default();
    config.extract_claims.enabled = true;
    config.extract_claims.max_gleanings = 0;
    config.cluster_graph.use_lcc = false;
    config.workflows = vec![
        EXTRACT_COVARIATES_WORKFLOW.to_owned(),
        CREATE_COMMUNITIES_WORKFLOW.to_owned(),
        CREATE_FINAL_TEXT_UNITS_WORKFLOW.to_owned(),
    ];
    let pipeline = PipelineFactory::new(registry)
        .standard(&config)
        .expect("step7 pipeline should be created");

    let outputs = pipeline
        .run(&config, &mut context)
        .await
        .expect("step7 pipeline should run");

    assert_eq!(outputs.len(), 3);
    let covariates = provider
        .read_dataframe("covariates")
        .await
        .expect("covariates should exist");
    assert_eq!(covariates.height(), 1);
    assert_eq!(
        covariates
            .column("human_readable_id")
            .expect("human id should exist")
            .i64()
            .expect("human id should be i64")
            .get(0),
        Some(0),
    );
    assert_eq!(
        covariates
            .column("text_unit_id")
            .expect("text unit id should exist")
            .str()
            .expect("text unit id should be string")
            .get(0),
        Some("tu-1"),
    );

    let communities = provider
        .read_dataframe("communities")
        .await
        .expect("communities should exist");
    assert_eq!(communities.height(), 1);
    assert_eq!(
        communities
            .column("size")
            .expect("size should exist")
            .i64()
            .expect("size should be i64")
            .get(0),
        Some(2),
    );

    let text_units = provider
        .read_dataframe("text_units")
        .await
        .expect("text units should exist");
    assert_eq!(
        text_units
            .column("human_readable_id")
            .expect("human id should exist")
            .i64()
            .expect("human id should be i64")
            .get(0),
        Some(0),
    );
    assert_eq!(
        text_units
            .column("n_tokens")
            .expect("n_tokens should exist")
            .i64()
            .expect("n_tokens should be i64")
            .get(0),
        Some(4),
    );
    assert_eq!(
        text_units
            .column("entity_ids")
            .expect("entity ids should exist")
            .list()
            .expect("entity ids should be list")
            .get_as_series(0)
            .expect("first entity ids should exist")
            .str()
            .expect("entity ids should be string list")
            .len(),
        2,
    );
    assert_eq!(
        text_units
            .column("relationship_ids")
            .expect("relationship ids should exist")
            .list()
            .expect("relationship ids should be list")
            .get_as_series(0)
            .expect("first relationship ids should exist")
            .str()
            .expect("relationship ids should be string list")
            .get(0),
        Some("rel-1"),
    );
    assert!(
        text_units
            .column("covariate_ids")
            .expect("covariate ids should exist")
            .list()
            .expect("covariate ids should be list")
            .get_as_series(0)
            .expect("first covariate ids should exist")
            .str()
            .expect("covariate ids should be string list")
            .get(0)
            .is_some()
    );
}

#[test]
fn test_should_default_workflow_order_to_step9() {
    let order = GraphRagConfig::default().workflow_order();

    assert_eq!(
        order.last().map(String::as_str),
        Some(GENERATE_TEXT_EMBEDDINGS_WORKFLOW)
    );
}

#[tokio::test]
async fn test_should_run_create_community_reports_workflow() {
    let provider = Arc::new(MemoryTableProvider::new());
    write_step8_report_inputs(&provider).await;

    let prompts = Arc::new(Mutex::new(Vec::new()));
    let model = Arc::new(CapturingWorkflowReportModel {
        prompts: Arc::clone(&prompts),
        calls: AtomicUsize::new(0),
    });
    let mut context = PipelineRunContext::new(provider.clone())
        .with_completion_model("default_completion_model", model);
    let mut registry = WorkflowRegistry::new();
    register_step8_workflows(&mut registry);
    let mut config = GraphRagConfig::default();
    config.extract_claims.enabled = true;
    config.community_reports.max_input_length = 80;
    config.workflows = vec![CREATE_COMMUNITY_REPORTS_WORKFLOW.to_owned()];
    let pipeline = PipelineFactory::new(registry)
        .standard(&config)
        .expect("step8 workflow should be registered");

    let outputs = pipeline
        .run(&config, &mut context)
        .await
        .expect("community reports workflow should run");

    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].input_rows, 2);
    assert_eq!(outputs[0].output_rows, 2);
    assert_eq!(context.stats.report_count, 2);
    {
        let prompts = prompts.lock().expect("prompts lock");
        assert_eq!(prompts.len(), 2);
        assert!(!prompts[0].contains("----Reports-----"));
        assert!(prompts[1].contains("----Reports-----"));
        assert!(prompts[1].contains("# Child"));
    }
    let reports = provider
        .read_dataframe("community_reports")
        .await
        .expect("community reports should exist");
    assert_step8_report_schema(&reports);
}

async fn write_step8_report_inputs(provider: &MemoryTableProvider) {
    write_step8_entities(provider).await;
    write_step8_relationships(provider).await;
    write_step8_communities(provider).await;
    write_step8_covariates(provider).await;
}

async fn write_step8_entities(provider: &MemoryTableProvider) {
    provider
        .write_dataframe(
            "entities",
            df!(
                "id" => ["entity-a", "entity-b", "entity-c"],
                "human_readable_id" => [0i64, 1i64, 2i64],
                "title" => ["ALICE", "BOB", "CAROL"],
                "description" => [
                    "Alice, lead",
                    "Bob\nresearcher",
                    "Carol one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty",
                ],
                "degree" => [3i64, 2i64, 1i64],
            )
            .expect("entities dataframe should build"),
        )
        .await
        .expect("entities should write");
}

async fn write_step8_relationships(provider: &MemoryTableProvider) {
    provider
        .write_dataframe(
            "relationships",
            df!(
                "id" => ["rel-1", "rel-2"],
                "human_readable_id" => [0i64, 1i64],
                "source" => ["ALICE", "CAROL"],
                "target" => ["BOB", "ALICE"],
                "description" => [
                    "Alice works with Bob",
                    "Carol collaborates with Alice on one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen",
                ],
                "combined_degree" => [3i64, 2i64],
            )
            .expect("relationships dataframe should build"),
        )
        .await
        .expect("relationships should write");
}

async fn write_step8_communities(provider: &MemoryTableProvider) {
    let mut communities = df!(
        "community" => [1i64, 0i64],
        "level" => [1i64, 0i64],
        "parent" => [0i64, -1i64],
        "period" => ["2026-07-08", "2026-07-08"],
        "size" => [2i64, 2i64],
    )
    .expect("communities dataframe should build");
    communities
        .insert_column(3, i64_list_column("children", &[vec![], vec![1]]))
        .expect("children should build");
    communities
        .insert_column(
            4,
            string_list_column(
                "entity_ids",
                &[
                    vec!["entity-a".to_owned(), "entity-b".to_owned()],
                    vec![
                        "entity-a".to_owned(),
                        "entity-b".to_owned(),
                        "entity-c".to_owned(),
                    ],
                ],
            ),
        )
        .expect("entity ids should build");
    provider
        .write_dataframe("communities", communities)
        .await
        .expect("communities should write");
}

async fn write_step8_covariates(provider: &MemoryTableProvider) {
    provider
        .write_dataframe(
            "covariates",
            df!(
                "id" => ["claim-1"],
                "human_readable_id" => [0i64],
                "covariate_type" => ["claim"],
                "type" => ["REPORT"],
                "description" => ["Alice reports Bob"],
                "subject_id" => ["ALICE"],
                "object_id" => ["BOB"],
                "status" => ["TRUE"],
                "start_date" => ["2026-07-07"],
                "end_date" => ["2026-07-07"],
                "source_text" => ["Alice reports Bob"],
                "text_unit_id" => ["tu-1"],
            )
            .expect("covariates dataframe should build"),
        )
        .await
        .expect("covariates should write");
}

fn assert_step8_report_schema(reports: &DataFrame) {
    assert_eq!(reports.height(), 2);
    assert_eq!(
        reports.column("human_readable_id").expect("hrid").dtype(),
        &DataType::Int64
    );
    assert_eq!(
        reports.column("rank").expect("rank").dtype(),
        &DataType::Float64
    );
    assert_eq!(
        reports.column("children").expect("children").dtype(),
        &DataType::List(Box::new(DataType::Int64))
    );
    assert!(matches!(
        reports.column("findings").expect("findings").dtype(),
        DataType::List(inner) if matches!(inner.as_ref(), DataType::Struct(_))
    ));
    assert_eq!(
        reports
            .column("community")
            .expect("community")
            .i64()
            .expect("community should be i64")
            .get(0),
        Some(1),
    );
    assert!(
        reports
            .column("full_content_json")
            .expect("json")
            .str()
            .expect("json string")
            .get(0)
            .expect("json value")
            .contains("rating_explanation")
    );
}

#[tokio::test]
#[allow(
    clippy::field_reassign_with_default,
    reason = "VectorStoreConfig is non_exhaustive outside graphloom-vectors and must be \
              customized after Default"
)]
async fn test_should_generate_text_embeddings_to_lancedb_and_snapshots() {
    let provider = Arc::new(MemoryTableProvider::new());
    provider
        .write_dataframe(
            "text_units",
            df!(
                "id" => ["tu-1", "tu-2"],
                "text" => ["hello text", "   "],
            )
            .expect("text units dataframe should build"),
        )
        .await
        .expect("text_units should write");
    provider
        .write_dataframe(
            "entities",
            df!(
                "id" => ["entity-1", "entity-2"],
                "title" => [Some("Alice"), None],
                "description" => [Some("Engineer"), None],
            )
            .expect("entities dataframe should build"),
        )
        .await
        .expect("entities should write");
    provider
        .write_dataframe(
            "community_reports",
            df!(
                "id" => ["report-1"],
                "full_content" => ["community content"],
            )
            .expect("community reports dataframe should build"),
        )
        .await
        .expect("community_reports should write");

    let tempdir = TempDir::new().expect("tempdir should create");
    let model = Arc::new(CapturingEmbeddingModel::default());
    let mut context = PipelineRunContext::new(provider.clone())
        .with_embedding_model("default_embedding_model", model.clone());
    let mut registry = WorkflowRegistry::new();
    register_step9_workflows(&mut registry);
    let mut vector_store = VectorStoreConfig::default();
    vector_store.db_uri = tempdir.path().to_string_lossy().to_string();
    vector_store.vector_size = 2;
    let mut config = GraphRagConfig {
        workflows: vec![GENERATE_TEXT_EMBEDDINGS_WORKFLOW.to_owned()],
        snapshots: crate::SnapshotsConfig {
            embeddings: true,
            ..Default::default()
        },
        vector_store,
        ..Default::default()
    };
    config.embedding_models.clear();

    let pipeline = PipelineFactory::new(registry)
        .standard(&config)
        .expect("step9 pipeline should build");
    let outputs = pipeline
        .run(&config, &mut context)
        .await
        .expect("step9 pipeline should run");

    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].input_rows, 5);
    assert_eq!(outputs[0].output_rows, 3);
    assert_eq!(context.stats.embedding_count, 3);
    assert_eq!(context.stats.llm_request_count, 3);
    assert!(model.inputs().iter().any(|input| input == "Alice:Engineer"));
    let summaries = &outputs[0].result;
    assert_eq!(summaries.len(), 3);
    for summary in summaries {
        let input_rows = summary["input_rows"].as_u64().expect("input rows");
        let embedded_rows = summary["embedded_rows"].as_u64().expect("embedded rows");
        let skipped_rows = summary["skipped_rows"].as_u64().expect("skipped rows");
        assert_eq!(embedded_rows + skipped_rows, input_rows);
    }

    let store = LanceDbVectorStore::connect(&config.vector_store)
        .await
        .expect("store should reopen");
    let text_schema = config
        .vector_store
        .schema_for(crate::TEXT_UNIT_TEXT_EMBEDDING);
    let entity_schema = config
        .vector_store
        .schema_for(crate::ENTITY_DESCRIPTION_EMBEDDING);
    let report_schema = config
        .vector_store
        .schema_for(crate::COMMUNITY_FULL_CONTENT_EMBEDDING);
    assert_eq!(store.count(&text_schema).await.expect("text count"), 1);
    assert_eq!(store.count(&entity_schema).await.expect("entity count"), 1);
    assert_eq!(store.count(&report_schema).await.expect("report count"), 1);
    assert!(
        store
            .get_by_id(&text_schema, "tu-1")
            .await
            .expect("get text")
            .is_some()
    );
    assert!(
        store
            .get_by_id(&entity_schema, "entity-1")
            .await
            .expect("get entity")
            .is_some()
    );
    assert!(
        store
            .get_by_id(&report_schema, "report-1")
            .await
            .expect("get report")
            .is_some()
    );

    let snapshots = provider
        .child(Some("embeddings"))
        .expect("snapshot namespace should open");
    for table_name in [
        crate::TEXT_UNIT_TEXT_EMBEDDING,
        crate::ENTITY_DESCRIPTION_EMBEDDING,
        crate::COMMUNITY_FULL_CONTENT_EMBEDDING,
    ] {
        let dataframe = snapshots
            .read_dataframe(table_name)
            .await
            .expect("snapshot should exist");
        assert_eq!(
            dataframe
                .get_column_names()
                .iter()
                .map(|name| name.as_str().to_owned())
                .collect::<Vec<_>>(),
            vec!["id".to_owned(), "embedding".to_owned()]
        );
        assert_eq!(
            dataframe.column("embedding").expect("embedding").dtype(),
            &DataType::List(Box::new(DataType::Float32))
        );
    }

    assert_eq!(
        GraphRagConfig::default()
            .workflow_order()
            .last()
            .map(String::as_str),
        Some(GENERATE_TEXT_EMBEDDINGS_WORKFLOW)
    );
}

#[tokio::test]
async fn test_should_fail_step9_when_embedding_model_is_not_injected_or_configured() {
    let provider = Arc::new(MemoryTableProvider::new());
    provider
        .write_dataframe(
            "text_units",
            df!("id" => ["tu-1"], "text" => ["hello"]).expect("text units"),
        )
        .await
        .expect("text_units should write");
    let mut context = PipelineRunContext::new(provider);
    let mut registry = WorkflowRegistry::new();
    register_step9_workflows(&mut registry);
    let mut config = GraphRagConfig {
        workflows: vec![GENERATE_TEXT_EMBEDDINGS_WORKFLOW.to_owned()],
        ..Default::default()
    };
    config.embedding_models.clear();

    let pipeline = PipelineFactory::new(registry)
        .standard(&config)
        .expect("step9 pipeline should build");
    let error = pipeline
        .run(&config, &mut context)
        .await
        .expect_err("missing model should fail");

    assert!(error.to_string().contains("embedding model"));
}

#[tokio::test]
#[allow(
    clippy::field_reassign_with_default,
    reason = "VectorStoreConfig is non_exhaustive outside graphloom-vectors and must be \
              customized after Default"
)]
async fn test_should_fail_step9_on_duplicate_source_id_across_flushes_without_overwrite() {
    let provider = Arc::new(MemoryTableProvider::new());
    provider
        .write_dataframe(
            "text_units",
            df!(
                "id" => ["duplicate", "duplicate"],
                "text" => ["first", "second"],
            )
            .expect("text units dataframe should build"),
        )
        .await
        .expect("text_units should write");

    let tempdir = TempDir::new().expect("tempdir should create");
    let model = Arc::new(CapturingEmbeddingModel::default());
    let mut context = PipelineRunContext::new(provider.clone())
        .with_embedding_model("default_embedding_model", model.clone());
    let mut registry = WorkflowRegistry::new();
    register_step9_workflows(&mut registry);
    let mut vector_store = VectorStoreConfig::default();
    vector_store.db_uri = tempdir.path().to_string_lossy().to_string();
    vector_store.vector_size = 2;
    let mut config = GraphRagConfig {
        workflows: vec![GENERATE_TEXT_EMBEDDINGS_WORKFLOW.to_owned()],
        concurrent_requests: 1,
        snapshots: crate::SnapshotsConfig {
            embeddings: true,
            ..Default::default()
        },
        vector_store,
        ..Default::default()
    };
    config.embed_text.batch_size = 1;
    config.embed_text.names = vec![crate::TEXT_UNIT_TEXT_EMBEDDING.to_owned()];
    config.embedding_models.clear();

    let pipeline = PipelineFactory::new(registry)
        .standard(&config)
        .expect("step9 pipeline should build");
    let error = pipeline
        .run(&config, &mut context)
        .await
        .expect_err("duplicate id should fail");

    assert!(error.to_string().contains("text_unit_text"));
    assert!(error.to_string().contains("duplicate"));
    assert_eq!(model.inputs(), vec!["first".to_owned()]);

    let store = LanceDbVectorStore::connect(&config.vector_store)
        .await
        .expect("store should reopen");
    let schema = config
        .vector_store
        .schema_for(crate::TEXT_UNIT_TEXT_EMBEDDING);
    let document = store
        .get_by_id(&schema, "duplicate")
        .await
        .expect("get duplicate")
        .expect("first flush should remain committed");
    assert_eq!(document.vector, vec![1.0, 0.0]);

    let snapshots = provider
        .child(Some("embeddings"))
        .expect("snapshot namespace should open");
    assert!(
        !snapshots
            .has(crate::TEXT_UNIT_TEXT_EMBEDDING)
            .await
            .expect("snapshot has should work")
    );
}

#[tokio::test]
#[allow(
    clippy::field_reassign_with_default,
    reason = "VectorStoreConfig is non_exhaustive outside graphloom-vectors and must be \
              customized after Default"
)]
async fn test_should_allow_same_source_id_in_different_embedding_fields() {
    let provider = Arc::new(MemoryTableProvider::new());
    provider
        .write_dataframe(
            "text_units",
            df!("id" => ["shared-id"], "text" => ["text"]).expect("text units"),
        )
        .await
        .expect("text_units should write");
    provider
        .write_dataframe(
            "entities",
            df!(
                "id" => ["shared-id"],
                "title" => ["Alice"],
                "description" => ["Engineer"],
            )
            .expect("entities"),
        )
        .await
        .expect("entities should write");

    let tempdir = TempDir::new().expect("tempdir should create");
    let model = Arc::new(CapturingEmbeddingModel::default());
    let mut context = PipelineRunContext::new(provider)
        .with_embedding_model("default_embedding_model", model.clone());
    let mut registry = WorkflowRegistry::new();
    register_step9_workflows(&mut registry);
    let mut vector_store = VectorStoreConfig::default();
    vector_store.db_uri = tempdir.path().to_string_lossy().to_string();
    vector_store.vector_size = 2;
    let mut config = GraphRagConfig {
        workflows: vec![GENERATE_TEXT_EMBEDDINGS_WORKFLOW.to_owned()],
        vector_store,
        ..Default::default()
    };
    config.embed_text.names = vec![
        crate::TEXT_UNIT_TEXT_EMBEDDING.to_owned(),
        crate::ENTITY_DESCRIPTION_EMBEDDING.to_owned(),
    ];
    config.embedding_models.clear();

    let pipeline = PipelineFactory::new(registry)
        .standard(&config)
        .expect("step9 pipeline should build");
    pipeline
        .run(&config, &mut context)
        .await
        .expect("same id across fields should be valid");

    let store = LanceDbVectorStore::connect(&config.vector_store)
        .await
        .expect("store should reopen");
    let text_schema = config
        .vector_store
        .schema_for(crate::TEXT_UNIT_TEXT_EMBEDDING);
    let entity_schema = config
        .vector_store
        .schema_for(crate::ENTITY_DESCRIPTION_EMBEDDING);
    assert!(
        store
            .get_by_id(&text_schema, "shared-id")
            .await
            .expect("get text")
            .is_some()
    );
    assert!(
        store
            .get_by_id(&entity_schema, "shared-id")
            .await
            .expect("get entity")
            .is_some()
    );
}

#[tokio::test]
#[allow(
    clippy::field_reassign_with_default,
    reason = "VectorStoreConfig is non_exhaustive outside graphloom-vectors and must be \
              customized after Default"
)]
async fn test_should_truncate_embedding_snapshot_when_workflow_reruns_to_empty() {
    let provider = Arc::new(MemoryTableProvider::new());
    provider
        .write_dataframe(
            "text_units",
            df!("id" => ["tu-1"], "text" => ["hello"]).expect("first text units"),
        )
        .await
        .expect("first text_units should write");

    let tempdir = TempDir::new().expect("tempdir should create");
    let mut registry = WorkflowRegistry::new();
    register_step9_workflows(&mut registry);
    let mut vector_store = VectorStoreConfig::default();
    vector_store.db_uri = tempdir.path().to_string_lossy().to_string();
    vector_store.vector_size = 2;
    let mut config = GraphRagConfig {
        workflows: vec![GENERATE_TEXT_EMBEDDINGS_WORKFLOW.to_owned()],
        snapshots: crate::SnapshotsConfig {
            embeddings: true,
            ..Default::default()
        },
        vector_store,
        ..Default::default()
    };
    config.embed_text.names = vec![crate::TEXT_UNIT_TEXT_EMBEDDING.to_owned()];
    config.embedding_models.clear();

    let pipeline = PipelineFactory::new(registry)
        .standard(&config)
        .expect("step9 pipeline should build");
    let model = Arc::new(CapturingEmbeddingModel::default());
    let mut context = PipelineRunContext::new(provider.clone())
        .with_embedding_model("default_embedding_model", model);
    pipeline
        .run(&config, &mut context)
        .await
        .expect("first run should succeed");

    let snapshots = provider
        .child(Some("embeddings"))
        .expect("snapshot namespace should open");
    let first = snapshots
        .read_dataframe(crate::TEXT_UNIT_TEXT_EMBEDDING)
        .await
        .expect("first snapshot should exist");
    assert_eq!(first.height(), 1);
    assert_eq!(
        first
            .column("id")
            .expect("id")
            .str()
            .expect("id string")
            .get(0),
        Some("tu-1")
    );

    provider
        .write_dataframe(
            "text_units",
            df!("id" => ["tu-2"], "text" => ["   "]).expect("second text units"),
        )
        .await
        .expect("second text_units should write");
    let model = Arc::new(CapturingEmbeddingModel::default());
    let mut context = PipelineRunContext::new(provider.clone())
        .with_embedding_model("default_embedding_model", model);
    pipeline
        .run(&config, &mut context)
        .await
        .expect("second run should succeed");

    let second = snapshots
        .read_dataframe(crate::TEXT_UNIT_TEXT_EMBEDDING)
        .await
        .expect("second snapshot should exist");
    assert_eq!(second.height(), 0);
    assert_eq!(
        second
            .get_column_names()
            .iter()
            .map(|name| name.as_str().to_owned())
            .collect::<Vec<_>>(),
        vec!["id".to_owned(), "embedding".to_owned()]
    );
    assert_eq!(
        second.column("embedding").expect("embedding").dtype(),
        &DataType::List(Box::new(DataType::Float32))
    );
}

#[derive(Debug, Default)]
struct CapturingEmbeddingModel {
    inputs: Mutex<Vec<String>>,
}

impl CapturingEmbeddingModel {
    fn inputs(&self) -> Vec<String> {
        self.inputs.lock().expect("inputs lock").clone()
    }
}

#[async_trait]
impl EmbeddingModel for CapturingEmbeddingModel {
    async fn embed(&self, request: EmbeddingRequest) -> graphloom_llm::Result<EmbeddingResponse> {
        self.inputs
            .lock()
            .expect("inputs lock")
            .extend(request.input.iter().cloned());
        Ok(EmbeddingResponse {
            embeddings: request
                .input
                .iter()
                .map(|input| {
                    if input == "first" || input.contains("Alice") {
                        vec![1.0, 0.0]
                    } else {
                        vec![0.0, 1.0]
                    }
                })
                .collect(),
            usage: None,
            request_id: None,
        })
    }
}

#[derive(Debug)]
struct CapturingWorkflowReportModel {
    prompts: Arc<Mutex<Vec<String>>>,
    calls: AtomicUsize,
}

#[async_trait]
impl CompletionModel for CapturingWorkflowReportModel {
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> graphloom_llm::Result<CompletionResponse> {
        let prompt = request
            .messages
            .into_iter()
            .next()
            .map(|message| message.content)
            .unwrap_or_default();
        self.prompts.lock().expect("prompts lock").push(prompt);
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let title = if call == 0 { "Child" } else { "Parent" };
        Ok(CompletionResponse {
            content: format!(
                "{{\"title\":\"{title}\",\"summary\":\"{title} \
                 summary\",\"rating\":7,\"rating_explanation\":\"{title} \
                 reason\",\"findings\":[{{\"summary\":\"{title} \
                 finding\",\"explanation\":\"{title} explanation\"}}]}}"
            ),
            usage: None,
            request_id: None,
        })
    }
}
