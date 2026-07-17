use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::Path,
    sync::{Arc, Mutex},
    time::SystemTime,
};

use futures_util::StreamExt;
use graphloom::{
    ENTITY_DESCRIPTION_EMBEDDING, GraphLoomError, GraphRagConfig, TEXT_UNIT_TEXT_EMBEDDING,
    api::{query, query_stream},
    query::{
        MapSearchResult, QueryCallbacks, QueryContext, QueryContextRecords, QueryContextText,
        QueryError, QueryEvent, QueryOptions, SearchMethod,
    },
};
use graphloom_llm::ModelConfig;
use graphloom_storage::{ParquetTableProvider, TableProvider};
use graphloom_vectors::{LanceDbVectorStore, VectorDocument, VectorIndexSchema, VectorStore};
use polars_core::prelude::{Column, DataFrame, NamedFrom, Series};
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

#[derive(Debug, Default)]
struct RecordingQueryCallbacks {
    events: Mutex<Vec<String>>,
}

impl QueryCallbacks for RecordingQueryCallbacks {
    fn on_context(&self, _context: &QueryContext) {
        self.events
            .lock()
            .expect("callback mutex")
            .push("context".to_owned());
    }

    fn on_llm_new_token(&self, token: &str) {
        self.events
            .lock()
            .expect("callback mutex")
            .push(format!("token:{token}"));
    }

    fn on_map_response_start(&self, contexts: &[String]) {
        self.events
            .lock()
            .expect("callback mutex")
            .push(format!("map_start:{}", contexts.len()));
    }

    fn on_map_response_end(&self, outputs: &[MapSearchResult]) {
        self.events
            .lock()
            .expect("callback mutex")
            .push(format!("map_end:{}", outputs.len()));
    }

    fn on_reduce_response_start(&self, _context: &str) {
        self.events
            .lock()
            .expect("callback mutex")
            .push("reduce_start".to_owned());
    }

    fn on_reduce_response_end(&self, output: &str) {
        self.events
            .lock()
            .expect("callback mutex")
            .push(format!("reduce_end:{output}"));
    }
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

async fn mount_global_query_stub() -> MockServer {
    use wiremock::matchers::body_partial_json;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({"stream": false})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "map-response",
            "object": "chat.completion",
            "created": 0,
            "model": "chat-test",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "{\"points\":[{\"description\":\"Mapped fact\",\"score\":8}]}",
                    "refusal": null
                },
                "finish_reason": "stop"
            }]
        })))
        .mount(&server)
        .await;
    let stream = concat!(
        r#"data: {"id":"reduce-1","model":"chat-test","choices":[{"index":0,"delta":{"content":"Global "},"finish_reason":null}]}"#,
        "\n\n",
        r#"data: {"id":"reduce-2","model":"chat-test","choices":[{"index":0,"delta":{"content":"answer."},"finish_reason":"stop"}]}"#,
        "\n\n",
        "data: [DONE]\n\n"
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({"stream": true})))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(stream),
        )
        .mount(&server)
        .await;
    server
}

async fn mount_global_no_data_stub() -> MockServer {
    use wiremock::matchers::body_partial_json;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({"stream": false})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "map-no-data",
            "object": "chat.completion",
            "created": 0,
            "model": "chat-test",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "{\"points\":[{\"description\":\"Irrelevant\",\"score\":0}]}",
                    "refusal": null
                },
                "finish_reason": "stop"
            }]
        })))
        .mount(&server)
        .await;
    server
}

async fn mount_global_map_failure_stub() -> MockServer {
    use wiremock::matchers::body_partial_json;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({"stream": false})))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"error": {"message": "invalid API key"}})),
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
            "max_tokens": 64,
            "max_completion_tokens": 128,
            "seed": 42,
            "stop": ["END"],
            "presence_penalty": 0.1,
            "frequency_penalty": 0.2,
            "stream": false,
            "custom_query_arg": {"enabled": true}
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

fn string_list_column(name: &str, rows: &[Vec<String>]) -> Column {
    let values = rows
        .iter()
        .map(|row| {
            Series::new(
                "item".into(),
                row.iter().map(String::as_str).collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    Series::new(name.into(), values).into()
}

fn i64_list_column(name: &str, rows: &[Vec<i64>]) -> Column {
    let values = rows
        .iter()
        .map(|row| Series::new("item".into(), row.as_slice()))
        .collect::<Vec<_>>();
    Series::new(name.into(), values).into()
}

async fn write_local_tables(root: &Path) -> Vec<std::path::PathBuf> {
    let provider = ParquetTableProvider::new(root).expect("Parquet provider");
    let mut entities = DataFrame::new(
        1,
        vec![
            Series::new("id".into(), ["entity-a"]).into(),
            Series::new("human_readable_id".into(), [0_i64]).into(),
            Series::new("title".into(), ["Alice"]).into(),
            Series::new("description".into(), ["Alice description"]).into(),
            Series::new("degree".into(), [2_i64]).into(),
        ],
    )
    .expect("entities");
    entities
        .with_column(string_list_column(
            "text_unit_ids",
            &[vec!["A".to_owned(), "B".to_owned()]],
        ))
        .expect("entity text units");
    let mut communities = DataFrame::new(
        1,
        vec![
            Series::new("id".into(), ["community-a"]).into(),
            Series::new("community".into(), [1_i64]).into(),
            Series::new("level".into(), [0_i64]).into(),
            Series::new("title".into(), ["Community A"]).into(),
            Series::new("parent".into(), [-1_i64]).into(),
        ],
    )
    .expect("communities");
    communities
        .with_column(string_list_column(
            "entity_ids",
            &[vec!["entity-a".to_owned()]],
        ))
        .expect("community entities");
    communities
        .with_column(i64_list_column("children", &[Vec::new()]))
        .expect("community children");
    let reports = DataFrame::new(
        1,
        vec![
            Series::new("id".into(), ["report-a"]).into(),
            Series::new("community".into(), [1_i64]).into(),
            Series::new("level".into(), [0_i64]).into(),
            Series::new("title".into(), ["Report A"]).into(),
            Series::new("summary".into(), ["Alice summary"]).into(),
            Series::new("full_content".into(), ["Alice full report"]).into(),
            Series::new("rank".into(), [9.0_f64]).into(),
        ],
    )
    .expect("reports");
    let mut units = text_units("first source", "second source");
    units
        .with_column(string_list_column(
            "relationship_ids",
            &[vec!["relationship-a".to_owned()], Vec::new()],
        ))
        .expect("text unit relationships");
    let mut relationships = DataFrame::new(
        1,
        vec![
            Series::new("id".into(), ["relationship-a"]).into(),
            Series::new("human_readable_id".into(), [0_i64]).into(),
            Series::new("source".into(), ["Alice"]).into(),
            Series::new("target".into(), ["External"]).into(),
            Series::new("description".into(), ["Alice to External"]).into(),
            Series::new("weight".into(), [1.0_f64]).into(),
            Series::new("combined_degree".into(), [2_i64]).into(),
        ],
    )
    .expect("relationships");
    relationships
        .with_column(string_list_column("text_unit_ids", &[vec!["A".to_owned()]]))
        .expect("relationship text units");
    for (name, dataframe) in [
        ("entities", entities),
        ("communities", communities),
        ("community_reports", reports),
        ("text_units", units),
        ("relationships", relationships),
    ] {
        provider
            .write_dataframe(name, dataframe)
            .await
            .expect("write Local table");
    }
    [
        "entities",
        "communities",
        "community_reports",
        "text_units",
        "relationships",
    ]
    .iter()
    .map(|name| root.join(format!("{name}.parquet")))
    .collect()
}

async fn write_dynamic_global_tables(root: &Path) -> Vec<std::path::PathBuf> {
    let provider = ParquetTableProvider::new(root).expect("Parquet provider");
    let mut entities = DataFrame::new(
        4,
        vec![
            Series::new(
                "id".into(),
                ["entity-0", "entity-1", "entity-2", "entity-3"],
            )
            .into(),
            Series::new("human_readable_id".into(), [0_i64, 1, 2, 3]).into(),
            Series::new(
                "title".into(),
                ["Entity 0", "Entity 1", "Entity 2", "Entity 3"],
            )
            .into(),
            Series::new(
                "description".into(),
                [
                    "Entity 0 description",
                    "Entity 1 description",
                    "Entity 2 description",
                    "Entity 3 description",
                ],
            )
            .into(),
            Series::new("degree".into(), [1_i64, 1, 1, 1]).into(),
        ],
    )
    .expect("Dynamic entities");
    entities
        .with_column(string_list_column(
            "text_unit_ids",
            &[
                vec!["unit-0".to_owned()],
                vec!["unit-1".to_owned()],
                vec!["unit-2".to_owned()],
                vec!["unit-3".to_owned()],
            ],
        ))
        .expect("Dynamic entity text units");
    let mut communities = DataFrame::new(
        4,
        vec![
            Series::new(
                "id".into(),
                ["community-0", "community-1", "community-2", "community-3"],
            )
            .into(),
            Series::new("community".into(), [0_i64, 1, 2, 3]).into(),
            Series::new("level".into(), [0_i64, 0, 0, 0]).into(),
            Series::new(
                "title".into(),
                ["Community 0", "Community 1", "Community 2", "Community 3"],
            )
            .into(),
            Series::new("parent".into(), [-1_i64, -1, -1, -1]).into(),
        ],
    )
    .expect("Dynamic communities");
    communities
        .with_column(string_list_column(
            "entity_ids",
            &[
                vec!["entity-0".to_owned()],
                vec!["entity-1".to_owned()],
                vec!["entity-2".to_owned()],
                vec!["entity-3".to_owned()],
            ],
        ))
        .expect("Dynamic community entities");
    communities
        .with_column(i64_list_column(
            "children",
            &[Vec::new(), Vec::new(), Vec::new(), Vec::new()],
        ))
        .expect("Dynamic community children");
    let reports = DataFrame::new(
        4,
        vec![
            Series::new(
                "id".into(),
                ["report-0", "report-1", "report-2", "report-3"],
            )
            .into(),
            Series::new("community".into(), [0_i64, 1, 2, 3]).into(),
            Series::new("level".into(), [0_i64, 0, 0, 0]).into(),
            Series::new(
                "title".into(),
                ["Report 0", "Report 1", "Report 2", "Report 3"],
            )
            .into(),
            Series::new(
                "summary".into(),
                ["Summary 0", "Summary 1", "Summary 2", "Summary 3"],
            )
            .into(),
            Series::new(
                "full_content".into(),
                [
                    "Full report 0",
                    "Full report 1",
                    "Full report 2",
                    "Full report 3",
                ],
            )
            .into(),
            Series::new("rank".into(), [4.0_f64, 3.0, 2.0, 1.0]).into(),
        ],
    )
    .expect("Dynamic reports");
    for (name, dataframe) in [
        ("entities", entities),
        ("communities", communities),
        ("community_reports", reports),
    ] {
        provider
            .write_dataframe(name, dataframe)
            .await
            .expect("write Dynamic Global table");
    }
    ["entities", "communities", "community_reports"]
        .iter()
        .map(|name| root.join(format!("{name}.parquet")))
        .collect()
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

fn local_options(root: &Path, query_text: &str) -> QueryOptions {
    QueryOptions::new(
        root.to_path_buf(),
        query_text.to_owned(),
        SearchMethod::Local,
    )
}

fn global_options(root: &Path, query_text: &str) -> QueryOptions {
    QueryOptions::new(
        root.to_path_buf(),
        query_text.to_owned(),
        SearchMethod::Global,
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
async fn test_should_run_local_api_and_stream_without_mutating_tables_or_vectors() {
    let server = mount_query_stub().await;
    let mut fixture = fixture(&server).await;
    let paths = write_local_tables(&fixture.project.path().join("output")).await;
    let before = futures_util::future::join_all(paths.iter().map(|path| async move {
        (
            file_hash(path).await,
            tokio::fs::metadata(path)
                .await
                .expect("Local table metadata")
                .modified()
                .expect("Local table mtime"),
        )
    }))
    .await;
    fixture.config.local_search.top_k_entities = 1;
    fixture.config.local_search.max_context_tokens = 4_000;
    let store = LanceDbVectorStore::connect(&fixture.config.vector_store)
        .await
        .expect("connect Local LanceDB");
    let schema = fixture
        .config
        .vector_store
        .schema_for(ENTITY_DESCRIPTION_EMBEDDING);
    store.ensure_index(&schema).await.expect("entity index");
    store
        .upsert_documents(
            &schema,
            &[VectorDocument {
                id: "entity-a".to_owned(),
                vector: vec![0.25, 0.75],
            }],
        )
        .await
        .expect("entity vector");
    let vector_ids = store.ids(&schema).await.expect("entity ids");

    let result = query(
        fixture.config.clone(),
        local_options(fixture.project.path(), "Who is Alice?"),
    )
    .await
    .expect("Local Query");
    assert_eq!(result.response, "Basic answer.");
    let QueryContextText::Text(context) = result.context.text else {
        panic!("expected Local context");
    };
    assert!(context.contains("-----Reports-----"));
    assert!(context.contains("-----Entities-----"));
    assert!(context.contains("-----Relationships-----"));
    assert!(context.contains("-----Sources-----"));
    assert_eq!(result.usage.categories["build_context"].llm_calls, 1);
    assert_eq!(result.usage.categories["response"].llm_calls, 1);

    let callbacks = Arc::new(RecordingQueryCallbacks::default());
    let mut stream_options = local_options(fixture.project.path(), "Who is Alice?");
    stream_options.callbacks.push(callbacks.clone());
    let mut events = query_stream(fixture.config.clone(), stream_options)
        .await
        .expect("Local stream");
    let mut chunks = Vec::new();
    while let Some(event) = events.next().await {
        if let QueryEvent::Token(token) = event.expect("Local stream event") {
            chunks.push(token);
        }
    }
    assert_eq!(chunks, ["Basic ", "answer."]);
    assert_eq!(
        *callbacks.events.lock().expect("callback events"),
        ["context", "token:Basic ", "token:answer."]
    );

    for (path, (hash, modified)) in paths.iter().zip(before) {
        assert_eq!(file_hash(path).await, hash);
        assert_eq!(
            tokio::fs::metadata(path)
                .await
                .expect("metadata after Local Query")
                .modified()
                .expect("mtime after Local Query"),
            modified
        );
    }
    let reopened = LanceDbVectorStore::connect(&fixture.config.vector_store)
        .await
        .expect("reopen Local LanceDB");
    assert_eq!(
        reopened.ids(&schema).await.expect("entity ids after"),
        vector_ids
    );
    assert!(!fixture.project.path().join("cache").exists());
    let requests = server.received_requests().await.expect("Local requests");
    let completions = requests
        .iter()
        .filter_map(|request| request.body_json::<Value>().ok())
        .filter(|body| body.get("messages").is_some())
        .collect::<Vec<_>>();
    assert_eq!(completions.len(), 2);
    assert!(completions.iter().all(|request| request["stream"] == true));
    assert!(completions.iter().all(|request| {
        request["messages"][0]["content"]
            .as_str()
            .is_some_and(|content| content.contains("-----Entities-----"))
    }));
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
async fn test_should_run_fixed_global_api_and_stream_without_vector_io_or_mutation() {
    let server = mount_global_query_stub().await;
    let project = TempDir::new().expect("project");
    let output = project.path().join("output");
    let table_paths = write_local_tables(&output).await;
    let before = futures_util::future::join_all(table_paths.iter().map(|path| async move {
        (
            file_hash(path).await,
            tokio::fs::metadata(path)
                .await
                .expect("Global table metadata")
                .modified()
                .expect("Global table mtime"),
        )
    }))
    .await;
    let mut config = GraphRagConfig::default();
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(&server, "chat-test"),
    );
    config.embedding_models.insert(
        "default_embedding_model".to_owned(),
        serde_json::from_value(json!({
            "model_provider": "unsupported",
            "model": "must-not-be-created",
            "api_key": "unused-secret"
        }))
        .expect("invalid unused embedding"),
    );
    config.vector_store.db_uri = project
        .path()
        .join("must-not-open-lancedb")
        .display()
        .to_string();

    let result = query(
        config.clone(),
        global_options(project.path(), "What are the themes?"),
    )
    .await
    .expect("Global Query");
    assert_eq!(result.response, "Global answer.");
    let QueryContextText::Composite(text) = &result.context.text else {
        panic!("expected composite Global context");
    };
    let QueryContextText::Batches(map_batches) = &text["map"] else {
        panic!("expected Global map batches");
    };
    assert!(matches!(text["dynamic"], QueryContextText::Empty));
    assert_eq!(map_batches.len(), 1);
    assert!(map_batches[0].contains("Alice full report"));
    let QueryContextText::Text(reduce_context) = &text["reduce"] else {
        panic!("expected Global reduce context");
    };
    assert_eq!(
        reduce_context,
        "----Analyst 1----\nImportance Score: 8\nMapped fact"
    );
    assert_eq!(result.usage.categories["build_context"].llm_calls, 0);
    assert_eq!(result.usage.categories["map"].llm_calls, 1);
    assert_eq!(result.usage.categories["reduce"].llm_calls, 1);

    let callbacks = Arc::new(RecordingQueryCallbacks::default());
    let mut stream_options = global_options(project.path(), "What are the themes?");
    stream_options.callbacks.push(callbacks.clone());
    let mut events = query_stream(config, stream_options)
        .await
        .expect("Global stream");
    let mut chunks = Vec::new();
    while let Some(event) = events.next().await {
        if let QueryEvent::Token(token) = event.expect("Global stream event") {
            chunks.push(token);
        }
    }
    assert_eq!(chunks, ["Global ", "answer."]);
    assert_eq!(
        *callbacks.events.lock().expect("Global callback events"),
        [
            "map_start:1",
            "map_end:1",
            "context",
            "reduce_start",
            "token:Global ",
            "token:answer.",
            "reduce_end:Global answer.",
        ]
    );

    for (path, (hash, modified)) in table_paths.iter().zip(before) {
        assert_eq!(file_hash(path).await, hash);
        assert_eq!(
            tokio::fs::metadata(path)
                .await
                .expect("metadata after Global Query")
                .modified()
                .expect("mtime after Global Query"),
            modified
        );
    }
    assert!(!project.path().join("must-not-open-lancedb").exists());
    assert!(!project.path().join("cache").exists());
    let requests = server.received_requests().await.expect("Global requests");
    assert_eq!(requests.len(), 4);
    let bodies = requests
        .iter()
        .map(|request| request.body_json::<Value>().expect("request JSON"))
        .collect::<Vec<_>>();
    assert!(
        !requests
            .iter()
            .any(|request| request.url.path().contains("embeddings"))
    );
    assert_eq!(
        bodies.iter().filter(|body| body["stream"] == false).count(),
        2
    );
    assert!(
        bodies
            .iter()
            .filter(|body| body["stream"] == false)
            .all(|body| body["response_format"] == json!({"type": "json_object"}))
    );
    assert_eq!(
        bodies.iter().filter(|body| body["stream"] == true).count(),
        2
    );
    assert!(bodies.iter().all(|body| {
        body["temperature"] == 0.0
            && body["top_p"] == 1.0
            && body["max_tokens"] == 64
            && body["max_completion_tokens"] == 128
            && body["seed"] == 42
            && body["stop"] == json!(["END"])
            && body["presence_penalty"] == 0.1
            && body["frequency_penalty"] == 0.2
            && body["custom_query_arg"] == json!({"enabled": true})
            && body["messages"][1]["content"] == "What are the themes?"
    }));
}

#[tokio::test]
async fn test_should_stream_global_no_data_without_reduce_call_or_callbacks() {
    let server = mount_global_no_data_stub().await;
    let project = TempDir::new().expect("project");
    write_local_tables(&project.path().join("output")).await;
    let mut config = GraphRagConfig::default();
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(&server, "chat-test"),
    );
    let callbacks = Arc::new(RecordingQueryCallbacks::default());
    let mut options = global_options(project.path(), "Unknown?");
    options.callbacks.push(callbacks.clone());

    let mut events = query_stream(config, options)
        .await
        .expect("no-data Global stream");
    let mut chunks = Vec::new();
    let mut completed = None;
    while let Some(event) = events.next().await {
        match event.expect("no-data event") {
            QueryEvent::Token(token) => chunks.push(token),
            QueryEvent::Completed(result) => completed = Some(result),
            QueryEvent::Context(_) => {}
            _ => panic!("unexpected future Query event"),
        }
    }
    let answer = "I am sorry but I am unable to answer this question given the provided data.";
    assert_eq!(chunks, [answer]);
    let result = completed.expect("completed result");
    assert_eq!(result.response, answer);
    assert_eq!(result.usage.categories["map"].llm_calls, 1);
    assert_eq!(result.usage.categories["reduce"].llm_calls, 0);
    assert_eq!(
        *callbacks.events.lock().expect("no-data callback events"),
        ["map_start:1", "map_end:1", "context"]
    );
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("no-data requests")
            .len(),
        1
    );
}

#[tokio::test]
async fn test_should_run_dynamic_global_with_rating_metadata_and_shared_map_reduce() {
    let server = mount_global_query_stub().await;
    let project = TempDir::new().expect("project");
    let table_paths = write_dynamic_global_tables(&project.path().join("output")).await;
    let before = futures_util::future::join_all(table_paths.iter().map(|path| async move {
        (
            file_hash(path).await,
            tokio::fs::metadata(path)
                .await
                .expect("Dynamic table metadata")
                .modified()
                .expect("Dynamic table mtime"),
        )
    }))
    .await;
    let mut config = GraphRagConfig::default();
    config.global_search.max_context_tokens = 1;
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(&server, "chat-test"),
    );
    config.embedding_models.insert(
        "default_embedding_model".to_owned(),
        serde_json::from_value(json!({
            "model_provider": "unsupported",
            "model": "must-not-be-created"
        }))
        .expect("invalid unused embedding"),
    );
    config.vector_store.db_uri = project
        .path()
        .join("must-not-open-dynamic-lancedb")
        .display()
        .to_string();
    let mut options = global_options(project.path(), "What are the themes?");
    options.dynamic_community_selection = true;

    let result = query(config.clone(), options)
        .await
        .expect("Dynamic Global Query");
    assert_eq!(result.response, "Global answer.");
    assert_eq!(result.usage.categories["build_context"].llm_calls, 4);
    assert_eq!(result.usage.categories["map"].llm_calls, 4);
    assert_eq!(result.usage.categories["reduce"].llm_calls, 1);
    let QueryContextText::Composite(text) = &result.context.text else {
        panic!("expected Dynamic Global composite context");
    };
    let QueryContextText::Named(dynamic_text) = &text["dynamic"] else {
        panic!("expected Dynamic rating text");
    };
    assert_eq!(dynamic_text.len(), 4);
    assert!(dynamic_text["0"].contains("rating=1"));
    assert!(dynamic_text["0"].contains("selected=true"));
    let QueryContextText::Batches(map_batches) = &text["map"] else {
        panic!("expected Dynamic Global map batches");
    };
    assert_eq!(map_batches.len(), 4);
    let QueryContextText::Text(reduce_context) = &text["reduce"] else {
        panic!("expected Dynamic Global reduce context");
    };
    for analyst in 1..=4 {
        assert!(reduce_context.contains(&format!("----Analyst {analyst}----")));
    }
    let QueryContextRecords::Named(records) = &result.context.records else {
        panic!("expected named Dynamic records");
    };
    let QueryContextRecords::Batches(dynamic_records) = &records["dynamic"] else {
        panic!("expected Dynamic rating records");
    };
    assert_eq!(dynamic_records[0].height(), 4);
    assert_eq!(
        dynamic_records[0]
            .column("community_id")
            .expect("community id")
            .str()
            .expect("string")
            .get(0),
        Some("0")
    );

    let callbacks = Arc::new(RecordingQueryCallbacks::default());
    let mut stream_options = global_options(project.path(), "What are the themes?");
    stream_options.dynamic_community_selection = true;
    stream_options.callbacks.push(callbacks.clone());
    let mut events = query_stream(config, stream_options)
        .await
        .expect("Dynamic Global stream");
    let mut chunks = Vec::new();
    while let Some(event) = events.next().await {
        if let QueryEvent::Token(token) = event.expect("Dynamic Global event") {
            chunks.push(token);
        }
    }
    assert_eq!(chunks, ["Global ", "answer."]);
    assert_eq!(
        *callbacks.events.lock().expect("Dynamic callbacks"),
        [
            "map_start:4",
            "map_end:4",
            "context",
            "reduce_start",
            "token:Global ",
            "token:answer.",
            "reduce_end:Global answer.",
        ]
    );
    assert!(
        !project
            .path()
            .join("must-not-open-dynamic-lancedb")
            .exists()
    );
    assert!(!project.path().join("cache").exists());
    for (path, (hash, modified)) in table_paths.iter().zip(before) {
        assert_eq!(file_hash(path).await, hash);
        assert_eq!(
            tokio::fs::metadata(path)
                .await
                .expect("metadata after Dynamic Global Query")
                .modified()
                .expect("mtime after Dynamic Global Query"),
            modified
        );
    }
    let requests = server.received_requests().await.expect("Dynamic requests");
    assert_eq!(requests.len(), 18);
    let bodies = requests
        .iter()
        .map(|request| request.body_json::<Value>().expect("request JSON"))
        .collect::<Vec<_>>();
    assert_eq!(
        bodies
            .iter()
            .filter(|body| {
                body["messages"][0]["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("deciding whether"))
            })
            .count(),
        8
    );
    assert!(
        bodies
            .iter()
            .filter(|body| body["stream"] == false)
            .all(|body| body["response_format"] == json!({"type": "json_object"}))
    );
}

#[tokio::test]
async fn test_should_not_emit_map_end_callback_after_provider_failure() {
    let server = mount_global_map_failure_stub().await;
    let project = TempDir::new().expect("project");
    write_local_tables(&project.path().join("output")).await;
    let mut config = GraphRagConfig::default();
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(&server, "chat-test"),
    );
    let callbacks = Arc::new(RecordingQueryCallbacks::default());
    let mut options = global_options(project.path(), "Question?");
    options.callbacks.push(callbacks.clone());

    let error = match query_stream(config, options).await {
        Ok(_) => panic!("map provider failure must fail Query construction"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        GraphLoomError::Query(source)
            if matches!(
                source.as_ref(),
                QueryError::QueryCompletion {
                    operation: "complete Global Search map call",
                    ..
                }
            )
    ));
    assert_eq!(
        *callbacks.events.lock().expect("map failure callbacks"),
        ["map_start:1"]
    );
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
            SearchMethod::Drift,
        ),
    )
    .await
    .expect_err("DRIFT is intentionally unavailable");
    assert!(matches!(
        method_error,
        GraphLoomError::Query(source) if matches!(source.as_ref(), QueryError::QueryMethod {
            method: Some(SearchMethod::Drift),
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
