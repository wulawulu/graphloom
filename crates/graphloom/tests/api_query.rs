use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::Path,
    time::SystemTime,
};

use futures_util::StreamExt;
use graphloom::{
    GraphLoomError, GraphRagConfig, TEXT_UNIT_TEXT_EMBEDDING,
    api::{query, query_stream},
    query::{QueryContextText, QueryError, QueryEvent, QueryOptions, SearchMethod},
};
use graphloom_llm::ModelConfig;
use graphloom_storage::{ParquetTableProvider, TableProvider};
use graphloom_vectors::{LanceDbVectorStore, VectorDocument, VectorIndexSchema, VectorStore};
use polars_core::prelude::{DataFrame, NamedFrom, Series};
use serde_json::{Value, json};
use tempfile::TempDir;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

struct QueryFixture {
    project: TempDir,
    config: GraphRagConfig,
    text_units_path: std::path::PathBuf,
    text_units_hash: u64,
    text_units_modified: SystemTime,
    vector_ids: Vec<String>,
}

async fn mount_query_stub() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [{"object": "embedding", "index": 0, "embedding": [0.25, 0.75]}],
            "model": "embed-test",
            "usage": {"prompt_tokens": 2, "total_tokens": 2}
        })))
        .mount(&server)
        .await;
    let stream = concat!(
        "data: {\"id\":\"chunk-1\",\"model\":\"chat-test\",\"choices\":[{\"index\":0,\"delta\":{\"\
         content\":\"Basic \"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chunk-2\",\"model\":\"chat-test\",\"choices\":[{\"index\":0,\"delta\":{},\
         \"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chunk-3\",\"model\":\"chat-test\",\"choices\":[{\"index\":0,\"delta\":{\"\
         content\":\"answer.\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(stream),
        )
        .mount(&server)
        .await;
    server
}

fn model_config(server: &MockServer, model: &str) -> ModelConfig {
    serde_json::from_value(json!({
        "model_provider": "openai",
        "model": model,
        "api_key": "query-test-secret",
        "api_base": format!("{}/v1", server.uri()),
        "encoding_model": "cl100k_base",
        "call_args": {
            "temperature": 0.0,
            "top_p": 1.0,
            "max_completion_tokens": 128,
            "seed": 42,
            "stop": ["END"],
            "presence_penalty": 0.1,
            "frequency_penalty": 0.2,
            "stream": false
        }
    }))
    .expect("model config")
}

fn text_units(first_text: &str, second_text: &str) -> DataFrame {
    DataFrame::new(
        2,
        vec![
            Series::new("id".into(), ["A", "B"]).into(),
            Series::new("text".into(), [first_text, second_text]).into(),
        ],
    )
    .expect("sparse GraphRAG text units")
}

async fn write_text_units(root: &Path, dataframe: DataFrame) -> std::path::PathBuf {
    tokio::fs::create_dir_all(root).await.expect("table root");
    let provider = ParquetTableProvider::new(root).expect("Parquet provider");
    provider
        .write_dataframe("text_units", dataframe)
        .await
        .expect("write text units");
    root.join("text_units.parquet")
}

async fn fixture(server: &MockServer) -> QueryFixture {
    let project = TempDir::new().expect("project");
    let output = project.path().join("output");
    let text_units_path =
        write_text_units(&output, text_units("first source", "second source")).await;
    let mut config = GraphRagConfig::default();
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(server, "chat-test"),
    );
    config.embedding_models.insert(
        "default_embedding_model".to_owned(),
        model_config(server, "embed-test"),
    );
    config.completion_models.insert(
        "unused_invalid_completion".to_owned(),
        serde_json::from_value(json!({
            "model_provider": "unsupported",
            "model": "must-not-be-created",
            "api_key": "unused-secret"
        }))
        .expect("unused completion config"),
    );
    config.embedding_models.insert(
        "unused_invalid_embedding".to_owned(),
        serde_json::from_value(json!({
            "model_provider": "unsupported",
            "model": "must-not-be-created",
            "api_key": "unused-secret"
        }))
        .expect("unused embedding config"),
    );
    config.vector_store.vector_size = 2;
    config.basic_search.k = 2;
    let absolute_vector_uri = project.path().join("output").join("lancedb");
    config.vector_store.db_uri = absolute_vector_uri.display().to_string();
    let store = LanceDbVectorStore::connect(&config.vector_store)
        .await
        .expect("connect LanceDB");
    let schema = config.vector_store.schema_for(TEXT_UNIT_TEXT_EMBEDDING);
    store.ensure_index(&schema).await.expect("vector index");
    store
        .upsert_documents(
            &schema,
            &[
                VectorDocument {
                    id: "B".to_owned(),
                    vector: vec![0.25, 0.75],
                },
                VectorDocument {
                    id: "A".to_owned(),
                    vector: vec![0.20, 0.70],
                },
            ],
        )
        .await
        .expect("vectors");
    let vector_ids = store.ids(&schema).await.expect("vector ids");
    let text_units_hash = file_hash(&text_units_path).await;
    let text_units_modified = tokio::fs::metadata(&text_units_path)
        .await
        .expect("text unit metadata")
        .modified()
        .expect("modified time");
    QueryFixture {
        project,
        config,
        text_units_path,
        text_units_hash,
        text_units_modified,
        vector_ids,
    }
}

async fn file_hash(path: &Path) -> u64 {
    let bytes = tokio::fs::read(path).await.expect("artifact bytes");
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn basic_options(root: &Path, query_text: &str) -> QueryOptions {
    QueryOptions::new(
        root.to_path_buf(),
        query_text.to_owned(),
        SearchMethod::Basic,
    )
}

#[tokio::test]
async fn test_should_run_basic_api_and_stream_events_without_mutating_index() {
    let server = mount_query_stub().await;
    let fixture = fixture(&server).await;

    let result = query(
        fixture.config.clone(),
        basic_options(fixture.project.path(), "What are the facts?"),
    )
    .await
    .expect("Basic Query");
    assert_eq!(result.response, "Basic answer.");
    let QueryContextText::Text(context) = result.context.text else {
        panic!("expected Basic context text");
    };
    assert_eq!(context, "id|text\n0|first source\n1|second source\n");
    assert_eq!(result.usage.llm_calls, 1);

    let mut events = query_stream(
        fixture.config.clone(),
        basic_options(fixture.project.path(), "What are the facts?"),
    )
    .await
    .expect("Basic Query stream");
    let mut event_order = Vec::new();
    let mut chunks = Vec::new();
    while let Some(event) = events.next().await {
        match event.expect("stream event") {
            QueryEvent::Context(_) => event_order.push("context"),
            QueryEvent::Token(token) => {
                event_order.push("token");
                chunks.push(token);
            }
            QueryEvent::Completed(result) => {
                event_order.push("completed");
                assert_eq!(result.response, "Basic answer.");
            }
            _ => panic!("unexpected Query event"),
        }
    }
    assert_eq!(chunks, ["Basic ", "answer."]);
    assert_eq!(event_order, ["context", "token", "token", "completed"]);

    assert_eq!(
        file_hash(&fixture.text_units_path).await,
        fixture.text_units_hash
    );
    assert_eq!(
        tokio::fs::metadata(&fixture.text_units_path)
            .await
            .expect("metadata after")
            .modified()
            .expect("modified after"),
        fixture.text_units_modified
    );
    let store = LanceDbVectorStore::connect(&fixture.config.vector_store)
        .await
        .expect("reopen LanceDB");
    let schema = fixture
        .config
        .vector_store
        .schema_for(TEXT_UNIT_TEXT_EMBEDDING);
    assert_eq!(
        store.ids(&schema).await.expect("ids after"),
        fixture.vector_ids
    );
    assert_eq!(store.count(&schema).await.expect("count after"), 2);
    assert!(!fixture.project.path().join("cache").exists());

    let requests = server.received_requests().await.expect("requests");
    let completion = requests
        .iter()
        .find_map(|request| {
            request
                .body_json::<Value>()
                .ok()
                .filter(|body| body.get("messages").is_some())
        })
        .expect("completion request");
    assert_eq!(completion["stream"], true);
    assert_eq!(completion["temperature"], 0.0);
    assert_eq!(completion["top_p"], 1.0);
    assert_eq!(completion["max_completion_tokens"], 128);
    assert_eq!(completion["seed"], 42);
    assert_eq!(completion["stop"], json!(["END"]));
    assert_eq!(completion["presence_penalty"], 0.1);
    assert_eq!(completion["frequency_penalty"], 0.2);
    assert!(
        completion["messages"][0]["content"]
            .as_str()
            .is_some_and(|value| value.contains("0|first source\n1|second source"))
    );
}

#[tokio::test]
async fn test_should_use_data_override_only_for_parquet_tables() {
    let server = mount_query_stub().await;
    let fixture = fixture(&server).await;
    let override_root = fixture.project.path().join("alternate_tables");
    write_text_units(
        &override_root,
        text_units("override first", "override second"),
    )
    .await;
    let mut options = basic_options(fixture.project.path(), "facts");
    options.data_dir = Some(override_root);

    let result = query(fixture.config, options)
        .await
        .expect("Query with table override");

    let QueryContextText::Text(context) = result.context.text else {
        panic!("expected context text");
    };
    assert_eq!(context, "id|text\n0|override first\n1|override second\n");
}

#[tokio::test]
async fn test_should_return_typed_errors_for_missing_resources_and_later_methods() {
    let server = mount_query_stub().await;
    let fixture = fixture(&server).await;

    let method_error = query(
        fixture.config.clone(),
        QueryOptions::new(
            fixture.project.path().to_path_buf(),
            "facts".to_owned(),
            SearchMethod::Global,
        ),
    )
    .await
    .expect_err("Global is intentionally unavailable");
    assert!(matches!(
        method_error,
        GraphLoomError::Query(source) if matches!(source.as_ref(), QueryError::QueryMethod {
            method: Some(SearchMethod::Global),
            ..
        })
    ));

    tokio::fs::remove_file(&fixture.text_units_path)
        .await
        .expect("remove text units");
    let table_error = query(
        fixture.config.clone(),
        basic_options(fixture.project.path(), "facts"),
    )
    .await
    .expect_err("missing text units");
    assert!(matches!(
        table_error,
        GraphLoomError::Query(source) if matches!(source.as_ref(), QueryError::MissingQueryTable {
            method: SearchMethod::Basic,
            table: "text_units",
            ..
        })
    ));

    write_text_units(
        &fixture.project.path().join("output"),
        text_units("first source", "second source"),
    )
    .await;
    let mut missing_vector_config = fixture.config;
    missing_vector_config.vector_store.index_schema.insert(
        TEXT_UNIT_TEXT_EMBEDDING.to_owned(),
        VectorIndexSchema::for_embedding_name("missing_text_unit_text", 2),
    );
    let vector_error = query(
        missing_vector_config,
        basic_options(fixture.project.path(), "facts"),
    )
    .await
    .expect_err("missing vector index");
    assert!(matches!(
        vector_error,
        GraphLoomError::Query(source) if matches!(source.as_ref(), QueryError::MissingVectorIndex {
            method: SearchMethod::Basic,
            ..
        })
    ));
}

#[tokio::test]
async fn test_should_not_create_output_vector_or_cache_paths_on_failed_query() {
    let project = TempDir::new().expect("project");
    let config = GraphRagConfig::default();

    let error = query(config, basic_options(project.path(), "facts"))
        .await
        .expect_err("missing Basic index");

    assert!(matches!(
        error,
        GraphLoomError::Query(source) if matches!(source.as_ref(), QueryError::MissingQueryTable {
            method: SearchMethod::Basic,
            table: "text_units",
            ..
        })
    ));
    assert!(!project.path().join("output").exists());
    assert!(!project.path().join("cache").exists());
    assert!(!project.path().join("logs").exists());
}
