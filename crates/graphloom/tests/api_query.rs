use std::{
    collections::{HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Instant, SystemTime},
};

use async_trait::async_trait;
use futures_util::StreamExt;
use graphloom::{
    COMMUNITY_FULL_CONTENT_EMBEDDING, ENTITY_DESCRIPTION_EMBEDDING, GraphLoomError, GraphRagConfig,
    TEXT_UNIT_TEXT_EMBEDDING,
    api::{
        basic_search, basic_search_streaming, drift_search, drift_search_streaming, global_search,
        global_search_streaming, local_search, local_search_streaming, query,
        query::{
            basic_search as module_basic_search,
            basic_search_streaming as module_basic_search_streaming,
            drift_search as module_drift_search,
            drift_search_streaming as module_drift_search_streaming,
            global_search as module_global_search,
            global_search_streaming as module_global_search_streaming,
            local_search as module_local_search,
            local_search_streaming as module_local_search_streaming, query as module_query,
            query_stream as module_query_stream,
        },
        query_stream,
    },
    explainability::{
        ContextSectionKind, ExplainabilityContentMode, ExplainabilityEvent,
        ExplainabilityQueryMethod, ExplainabilityRecord, ExplainabilityRecordType,
        ExplainabilityRunId, ExplainabilitySink, ExplainabilitySinkChain, ExplainabilitySinkError,
        GlobalMapPointDecisionReason, GlobalReduceSkipReason, NoopExplainabilitySink,
        SelectionReason,
    },
    query::{
        MapSearchResult, QueryCallbacks, QueryContext, QueryContextRecords, QueryContextText,
        QueryEngine, QueryError, QueryEvent, QueryEventStream, QueryExplainabilityOptions,
        QueryOptions, SearchMethod,
    },
};
use graphloom_llm::ModelConfig;
use graphloom_storage::{ParquetTableProvider, TableProvider};
use graphloom_vectors::{LanceDbVectorStore, VectorDocument, VectorIndexSchema, VectorStore};
use polars_core::prelude::{Column, DataFrame, NamedFrom, Series};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

mod support;

use support::CanonicalTempDir as TempDir;

fn completion_response(content: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "id": "drift-response",
        "object": "chat.completion",
        "created": 0,
        "model": "chat-test",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content,
                "refusal": null
            },
            "finish_reason": "stop"
        }]
    }))
}

fn drift_chat_responder(request: &Request) -> ResponseTemplate {
    let body = request.body_json::<Value>().expect("DRIFT request JSON");
    let messages = body["messages"].as_array().expect("DRIFT messages");
    let benchmark = messages.iter().any(|message| {
        message["content"]
            .as_str()
            .is_some_and(|content| content.contains("benchmark-depth"))
    });
    let first = messages
        .first()
        .and_then(|message| message["content"].as_str())
        .unwrap_or_default();
    let streaming = body["stream"].as_bool() == Some(true);
    if streaming {
        let content = if first.contains("follow_up_queries") {
            if benchmark {
                r#"{"response":"Action answer.","score":80,"follow_up_queries":["Next benchmark-depth?"]}"#
            } else {
                r#"{"response":"Action answer.","score":80,"follow_up_queries":[]}"#
            }
        } else {
            "DRIFT final."
        };
        let midpoint = content.len() / 2;
        let (first_chunk, second_chunk) = content.split_at(midpoint);
        let first_event = serde_json::json!({
            "id": "drift-1",
            "model": "chat-test",
            "choices": [{
                "index": 0,
                "delta": {"content": first_chunk},
                "finish_reason": null,
            }],
        });
        let second_event = serde_json::json!({
            "id": "drift-2",
            "model": "chat-test",
            "choices": [{
                "index": 0,
                "delta": {"content": second_chunk},
                "finish_reason": "stop",
            }],
        });
        let stream = format!("data: {first_event}\n\ndata: {second_event}\n\ndata: [DONE]\n\n",);
        return ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(stream);
    }
    if first.starts_with("Create a hypothetical answer") {
        completion_response("Expanded question")
    } else if first.starts_with("You are a helpful agent designed to reason") {
        if benchmark {
            completion_response(
                r#"{"intermediate_answer":"Primer answer.","score":70,"follow_up_queries":["Who benchmark-depth?","What benchmark-depth?","Where benchmark-depth?"]}"#,
            )
        } else {
            completion_response(
                r#"{"intermediate_answer":"Primer answer.","score":70,"follow_up_queries":["Who?"]}"#,
            )
        }
    } else {
        completion_response("DRIFT final.")
    }
}

async fn mount_drift_query_stub() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [{"object": "embedding", "index": 0, "embedding": [1.0, 0.0]}],
            "model": "embed-test",
            "usage": {"prompt_tokens": 0, "total_tokens": 0}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(drift_chat_responder)
        .mount(&server)
        .await;
    server
}

struct QueryFixture {
    project: TempDir,
    config: GraphRagConfig,
    text_units_path: std::path::PathBuf,
    text_units_hash: u64,
    text_units_modified: SystemTime,
    vector_ids: Vec<String>,
}

#[tokio::test]
async fn test_should_ignore_update_only_storage_validation_when_loading_query_engine() {
    for unsupported in [false, true] {
        let project = TempDir::new().expect("project");
        let mut config = GraphRagConfig::default();
        if unsupported {
            config.update_output_storage.storage_type = "unsupported".to_owned();
        } else {
            config.update_output_storage.base_dir = "output/update".to_owned();
        }

        QueryEngine::load(config, project.path())
            .await
            .expect("unused update config must not block query engine loading");
    }
}

#[derive(Debug, Default)]
struct RecordingQueryCallbacks {
    events: Mutex<Vec<String>>,
    reduce_start_payloads: Mutex<Vec<String>>,
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

    fn on_reduce_response_start(&self, context: &str) {
        self.reduce_start_payloads
            .lock()
            .expect("callback payload mutex")
            .push(context.to_owned());
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

#[derive(Debug, Default)]
struct RecordingExplainabilitySink {
    records: Mutex<Vec<Arc<ExplainabilityRecord>>>,
    emit_calls: AtomicUsize,
    finish_calls: AtomicUsize,
    fail_emit: bool,
    fail_finish: bool,
}

impl RecordingExplainabilitySink {
    fn failing() -> Self {
        Self {
            fail_emit: true,
            ..Self::default()
        }
    }

    fn records(&self) -> Vec<Arc<ExplainabilityRecord>> {
        self.records.lock().expect("Explainability records").clone()
    }
}

#[async_trait]
impl ExplainabilitySink for RecordingExplainabilitySink {
    async fn emit(&self, record: Arc<ExplainabilityRecord>) -> Result<(), ExplainabilitySinkError> {
        self.emit_calls.fetch_add(1, Ordering::SeqCst);
        self.records
            .lock()
            .expect("Explainability records")
            .push(record);
        if self.fail_emit {
            Err(ExplainabilitySinkError::RecordNotAccepted)
        } else {
            Ok(())
        }
    }

    async fn finish_run(
        &self,
        _run_id: &ExplainabilityRunId,
    ) -> Result<(), ExplainabilitySinkError> {
        self.finish_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_finish {
            Err(ExplainabilitySinkError::RunFinalizationFailed)
        } else {
            Ok(())
        }
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

async fn mount_local_handshake_failure_stub() -> MockServer {
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
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({
            "error": {"message": "provider unavailable"}
        })))
        .mount(&server)
        .await;
    server
}

async fn mount_basic_embedding_failure_stub() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({
            "error": {"message": "embedding provider unavailable"}
        })))
        .mount(&server)
        .await;
    server
}

async fn mount_basic_midstream_failure_stub() -> MockServer {
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
         content\":\"partial\"},\"finish_reason\":null}]}\n\n",
        "data: not-json\n\n"
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

async fn mount_dynamic_rating_failure_stub() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({
            "error": {"message": "rating provider unavailable"}
        })))
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
                    "content": "not valid map JSON",
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

async fn mount_global_reduce_failure_stub(midstream: bool) -> MockServer {
    use wiremock::matchers::body_partial_json;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({"stream": false})))
        .respond_with(completion_response(
            r#"{"points":[{"description":"Mapped fact","score":8}]}"#,
        ))
        .mount(&server)
        .await;
    let reduce = Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({"stream": true})));
    if midstream {
        reduce
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(concat!(
                        r#"data: {"id":"reduce-1","model":"chat-test","choices":[{"index":0,"delta":{"content":"partial"},"finish_reason":null}]}"#,
                        "\n\n",
                        "data: {invalid-json}\n\n"
                    )),
            )
            .mount(&server)
            .await;
    } else {
        reduce
            .respond_with(
                ResponseTemplate::new(503)
                    .set_body_json(json!({"error": {"message": "reduce unavailable"}})),
            )
            .mount(&server)
            .await;
    }
    server
}

fn global_explainability_responder(request: &Request) -> ResponseTemplate {
    let body = request.body_json::<Value>().expect("Global request JSON");
    let system_prompt = body["messages"][0]["content"]
        .as_str()
        .expect("Global system prompt");
    if body["stream"] == false {
        let (content, delay_ms) = if system_prompt.contains("Full report 3") {
            (
                r#"{"points":[{"description":"POINT_ANSWER_SECRET REDUCE_CONTEXT_SECRET highest","score":9},{"description":"MAP_RESPONSE_SECRET zero","score":0}]}"#,
                40,
            )
        } else {
            (
                r#"{"points":[{"description":"second selected","score":8},{"description":"budget excluded","score":7}]}"#,
                5,
            )
        };
        return ResponseTemplate::new(200)
            .set_delay(std::time::Duration::from_millis(delay_ms))
            .set_body_json(json!({
                "id": "map-explainability",
                "object": "chat.completion",
                "created": 0,
                "model": "chat-test",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": content,
                        "refusal": null
                    },
                    "finish_reason": "stop"
                }]
            }));
    }
    let response = "FINAL_RESPONSE_SECRET";
    let stream = format!(
        "data: {{\"id\":\"reduce\",\"model\":\"chat-test\",\"choices\":[{{\"index\":0,\"delta\":\
         {{\"content\":\"{response}\"}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
    );
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(stream)
}

async fn mount_global_explainability_stub() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(global_explainability_responder)
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

async fn seed_drift_vectors(config: &GraphRagConfig) -> (Vec<String>, Vec<String>) {
    let store = LanceDbVectorStore::connect(&config.vector_store)
        .await
        .expect("connect DRIFT LanceDB");
    let entity_schema = config.vector_store.schema_for(ENTITY_DESCRIPTION_EMBEDDING);
    let report_schema = config
        .vector_store
        .schema_for(COMMUNITY_FULL_CONTENT_EMBEDDING);
    for schema in [&entity_schema, &report_schema] {
        store
            .ensure_index(schema)
            .await
            .expect("DRIFT vector index");
    }
    store
        .upsert_documents(
            &entity_schema,
            &[VectorDocument {
                id: "entity-a".to_owned(),
                vector: vec![1.0, 0.0],
            }],
        )
        .await
        .expect("entity vector");
    store
        .upsert_documents(
            &report_schema,
            &[VectorDocument {
                id: "report-a".to_owned(),
                vector: vec![1.0, 0.0],
            }],
        )
        .await
        .expect("report vector");
    (
        store.ids(&entity_schema).await.expect("entity ids"),
        store.ids(&report_schema).await.expect("report ids"),
    )
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
                ["Shared", "Shared", "Community 2", "Community 3"],
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
                    "Full report 0 MAP_CONTEXT_SECRET",
                    "Full report 1 MAP_CONTEXT_SECRET",
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

async fn local_fixture(server: &MockServer) -> QueryFixture {
    let mut fixture = fixture(server).await;
    write_local_tables(&fixture.project.path().join("output")).await;
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
    fixture
}

async fn recorded_request_bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .expect("recorded requests")
        .iter()
        .map(|request| request.body_json::<Value>().expect("request JSON"))
        .collect()
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

#[derive(Debug)]
struct GlobalSnapshotOutcome {
    result: graphloom::query::QueryResult,
    callbacks: Vec<String>,
    report_ids: Vec<String>,
    batches: Vec<String>,
}

async fn run_global_snapshot(
    engine: &QueryEngine,
    root: &Path,
    dynamic: bool,
) -> GlobalSnapshotOutcome {
    let callbacks = Arc::new(RecordingQueryCallbacks::default());
    let mut options = global_options(root, "What are the themes?");
    options.dynamic_community_selection = dynamic;
    options.callbacks.push(callbacks.clone());
    let result = engine.query(options).await.expect("Global snapshot query");
    let report_ids = global_report_ids(&result);
    let QueryContextText::Composite(text) = &result.context.text else {
        panic!("expected Global composite context");
    };
    let QueryContextText::Batches(batches) = &text["map"] else {
        panic!("expected Global map batches");
    };
    let batches = batches.clone();
    let callback_events = callbacks.events.lock().expect("Global callbacks").clone();
    GlobalSnapshotOutcome {
        result,
        callbacks: callback_events,
        report_ids,
        batches,
    }
}

fn global_report_ids(result: &graphloom::query::QueryResult) -> Vec<String> {
    let QueryContextRecords::Named(records) = &result.context.records else {
        panic!("expected named Global records");
    };
    let QueryContextRecords::Batches(batches) = &records["map"] else {
        panic!("expected Global report batches");
    };
    batches
        .iter()
        .flat_map(|batch| {
            batch
                .column("id")
                .expect("Global report id column")
                .str()
                .expect("Global report string ids")
                .iter()
                .flatten()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn assert_global_snapshot_eq(actual: &GlobalSnapshotOutcome, expected: &GlobalSnapshotOutcome) {
    assert_eq!(actual.result.response, expected.result.response);
    assert_eq!(actual.result.usage, expected.result.usage);
    assert_eq!(
        format!("{:?}", actual.result.context.text),
        format!("{:?}", expected.result.context.text)
    );
    assert_eq!(
        format!("{:?}", actual.result.context.records),
        format!("{:?}", expected.result.context.records)
    );
    assert_eq!(actual.report_ids, expected.report_ids);
    assert_eq!(actual.batches, expected.batches);
    assert_eq!(actual.callbacks, expected.callbacks);
}

fn expect_stream_error(result: graphloom::Result<QueryEventStream>) -> GraphLoomError {
    match result {
        Ok(_) => panic!("expected Query stream startup error"),
        Err(error) => error,
    }
}

#[tokio::test]
async fn test_should_export_all_query_apis_and_force_method_specific_dispatch() {
    let project = TempDir::new().expect("project");
    let config = GraphRagConfig::default();
    let wrong_options =
        |method| QueryOptions::new(project.path().to_path_buf(), "question".to_owned(), method);

    let errors = [
        basic_search(config.clone(), wrong_options(SearchMethod::Global))
            .await
            .expect_err("Basic method must load Basic resources"),
        module_basic_search(config.clone(), wrong_options(SearchMethod::Local))
            .await
            .expect_err("module Basic method must load Basic resources"),
        local_search(config.clone(), wrong_options(SearchMethod::Global))
            .await
            .expect_err("Local method must load Local resources"),
        module_local_search(config.clone(), wrong_options(SearchMethod::Basic))
            .await
            .expect_err("module Local method must load Local resources"),
        global_search(config.clone(), wrong_options(SearchMethod::Local))
            .await
            .expect_err("Global method must load Global resources"),
        module_global_search(config.clone(), wrong_options(SearchMethod::Drift))
            .await
            .expect_err("module Global method must load Global resources"),
        drift_search(config.clone(), wrong_options(SearchMethod::Basic))
            .await
            .expect_err("DRIFT method must load DRIFT resources"),
        module_drift_search(config.clone(), wrong_options(SearchMethod::Global))
            .await
            .expect_err("module DRIFT method must load DRIFT resources"),
    ];
    for (error, method) in errors.into_iter().zip([
        "basic", "basic", "local", "local", "global", "global", "drift", "drift",
    ]) {
        assert!(
            error.to_string().starts_with(method)
                || error.to_string().contains(&format!("for {method}")),
            "{error}"
        );
    }

    let stream_errors = [
        expect_stream_error(
            basic_search_streaming(config.clone(), wrong_options(SearchMethod::Global)).await,
        ),
        expect_stream_error(
            module_basic_search_streaming(config.clone(), wrong_options(SearchMethod::Local)).await,
        ),
        expect_stream_error(
            local_search_streaming(config.clone(), wrong_options(SearchMethod::Global)).await,
        ),
        expect_stream_error(
            module_local_search_streaming(config.clone(), wrong_options(SearchMethod::Basic)).await,
        ),
        expect_stream_error(
            global_search_streaming(config.clone(), wrong_options(SearchMethod::Local)).await,
        ),
        expect_stream_error(
            module_global_search_streaming(config.clone(), wrong_options(SearchMethod::Drift))
                .await,
        ),
        expect_stream_error(
            drift_search_streaming(config.clone(), wrong_options(SearchMethod::Basic)).await,
        ),
        expect_stream_error(
            module_drift_search_streaming(config.clone(), wrong_options(SearchMethod::Global))
                .await,
        ),
    ];
    for (error, method) in stream_errors.into_iter().zip([
        "basic", "basic", "local", "local", "global", "global", "drift", "drift",
    ]) {
        assert!(
            error.to_string().starts_with(method)
                || error.to_string().contains(&format!("for {method}")),
            "{error}"
        );
    }

    module_query(config.clone(), wrong_options(SearchMethod::Basic))
        .await
        .expect_err("unified module API must dispatch Basic");
    let _ =
        expect_stream_error(module_query_stream(config, wrong_options(SearchMethod::Basic)).await);
}

#[tokio::test]
async fn test_should_make_unified_and_method_specific_basic_requests_identical() {
    let server = mount_query_stub().await;
    let fixture = fixture(&server).await;
    let options = basic_options(fixture.project.path(), "What are the facts?");

    let method_result = basic_search(fixture.config.clone(), options.clone())
        .await
        .expect("method-specific Basic Query");
    let unified_result = query(fixture.config.clone(), options.clone())
        .await
        .expect("unified Basic Query");
    assert_eq!(method_result.response, unified_result.response);
    let (QueryContextText::Text(method_context), QueryContextText::Text(unified_context)) =
        (method_result.context.text, unified_result.context.text)
    else {
        panic!("expected Basic text contexts");
    };
    assert_eq!(method_context, unified_context);
    assert_eq!(method_result.usage, unified_result.usage);

    let mut method_events = basic_search_streaming(fixture.config.clone(), options.clone())
        .await
        .expect("method-specific Basic stream");
    let mut unified_events = query_stream(fixture.config, options)
        .await
        .expect("unified Basic stream");
    let mut method_tokens = Vec::new();
    let mut unified_tokens = Vec::new();
    while let Some(event) = method_events.next().await {
        if let QueryEvent::Token(token) = event.expect("method stream event") {
            method_tokens.push(token);
        }
    }
    while let Some(event) = unified_events.next().await {
        if let QueryEvent::Token(token) = event.expect("unified stream event") {
            unified_tokens.push(token);
        }
    }
    assert_eq!(method_tokens, unified_tokens);

    let requests = server.received_requests().await.expect("recorded requests");
    assert_eq!(requests.len(), 8);
    let bodies = requests
        .iter()
        .map(|request| request.body_json::<Value>().expect("request JSON"))
        .collect::<Vec<_>>();
    assert_eq!(&bodies[0..2], &bodies[2..4]);
    assert_eq!(&bodies[4..6], &bodies[6..8]);
}

#[tokio::test]
async fn test_should_reuse_query_engine_snapshot_and_isolate_concurrent_requests() {
    let server = mount_query_stub().await;
    let fixture = fixture(&server).await;
    let engine = Arc::new(
        QueryEngine::load(fixture.config.clone(), fixture.project.path())
            .await
            .expect("Query engine"),
    );
    let first_callbacks = Arc::new(RecordingQueryCallbacks::default());
    let mut first = basic_options(fixture.project.path(), "first engine query");
    first.callbacks.push(first_callbacks.clone());
    let first_result = engine.query(first).await.expect("first engine query");

    let hidden_table = fixture.text_units_path.with_extension("parquet.hidden");
    tokio::fs::rename(&fixture.text_units_path, &hidden_table)
        .await
        .expect("hide table after snapshot preparation");

    let second_callbacks = Arc::new(RecordingQueryCallbacks::default());
    let third_callbacks = Arc::new(RecordingQueryCallbacks::default());
    let mut second = basic_options(fixture.project.path(), "second engine query");
    second.callbacks.push(second_callbacks.clone());
    let mut third = basic_options(fixture.project.path(), "third engine query is longer");
    third.callbacks.push(third_callbacks.clone());
    let (second_result, third_result) = tokio::join!(engine.query(second), engine.query(third));
    let second_result = second_result.expect("second engine query");
    let third_result = third_result.expect("third engine query");

    assert_eq!(first_result.response, second_result.response);
    assert_eq!(second_result.response, third_result.response);
    assert_ne!(
        second_result.usage.categories["response"].prompt_tokens,
        third_result.usage.categories["response"].prompt_tokens
    );
    for callbacks in [&first_callbacks, &second_callbacks, &third_callbacks] {
        let events = callbacks.events.lock().expect("callback events");
        assert_eq!(events.first().map(String::as_str), Some("context"));
        assert_eq!(
            events
                .iter()
                .filter(|event| event.starts_with("token:"))
                .count(),
            2
        );
    }
    assert_eq!(
        first_callbacks
            .events
            .lock()
            .expect("first callback events")
            .len(),
        3
    );
    assert_eq!(
        second_callbacks
            .events
            .lock()
            .expect("second callback events")
            .len(),
        3
    );
    assert_eq!(
        third_callbacks
            .events
            .lock()
            .expect("third callback events")
            .len(),
        3
    );

    tokio::fs::rename(hidden_table, &fixture.text_units_path)
        .await
        .expect("restore table fixture");
}

#[tokio::test]
#[ignore = "repeatable local performance probe; run through make bench-query"]
async fn test_performance_cold_and_warm_basic_queries() {
    let server = mount_query_stub().await;
    let fixture = fixture(&server).await;
    let options = basic_options(fixture.project.path(), "benchmark query");

    let cold_started = Instant::now();
    query(fixture.config.clone(), options.clone())
        .await
        .expect("cold one-shot query");
    let cold_elapsed = cold_started.elapsed();

    let engine = QueryEngine::load(fixture.config, fixture.project.path())
        .await
        .expect("warm engine");
    let warm_started = Instant::now();
    for _ in 0..10 {
        engine
            .query(options.clone())
            .await
            .expect("warm engine query");
    }
    let warm_elapsed = warm_started.elapsed();

    eprintln!(
        "query performance: cold_once={cold_elapsed:?}, warm_10={warm_elapsed:?}, \
         warm_average={:?}",
        warm_elapsed / 10
    );
}

#[tokio::test]
#[ignore = "repeatable DRIFT performance probe; run through make bench-query"]
async fn test_performance_drift_multiple_depths_and_concurrent_actions() {
    let server = mount_drift_query_stub().await;
    let mut fixture = fixture(&server).await;
    write_local_tables(&fixture.project.path().join("output")).await;
    seed_drift_vectors(&fixture.config).await;
    fixture.config.drift_search.primer_folds = 1;
    fixture.config.drift_search.drift_k_followups = 3;
    fixture.config.drift_search.n_depth = 3;
    fixture.config.drift_search.concurrency = 2;
    fixture.config.drift_search.local_search_max_data_tokens = 4_000;

    let engine = QueryEngine::load(fixture.config, fixture.project.path())
        .await
        .expect("DRIFT benchmark engine");
    let options = QueryOptions::new(
        fixture.project.path().to_path_buf(),
        "What changed benchmark-depth?".to_owned(),
        SearchMethod::Drift,
    );
    let started = Instant::now();
    let result = engine
        .query(options)
        .await
        .expect("multi-depth DRIFT benchmark query");

    eprintln!(
        "DRIFT performance: depth=3, followups=3, concurrency=2, elapsed={:?}, llm_calls={}",
        started.elapsed(),
        result.usage.llm_calls
    );
    assert!(result.usage.categories["action"].llm_calls > 3);
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
    assert_eq!(result.usage.llm_calls, 2);
    assert_eq!(result.usage.categories["build_context"].llm_calls, 1);
    assert_eq!(result.usage.categories["response"].llm_calls, 1);

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
async fn test_should_explain_basic_search_without_changing_business_requests_or_result() {
    let server = mount_query_stub().await;
    let fixture = fixture(&server).await;
    let query_text = "BASIC_USER_QUERY_SECRET";
    let baseline = query(
        fixture.config.clone(),
        basic_options(fixture.project.path(), query_text),
    )
    .await
    .expect("baseline Basic Query");
    let baseline_requests = recorded_request_bodies(&server).await;

    let metadata_sink = Arc::new(RecordingExplainabilitySink::default());
    let metadata = query(
        fixture.config.clone(),
        basic_options(fixture.project.path(), query_text).with_explainability(
            QueryExplainabilityOptions::new(
                "basic-metadata".parse().expect("run id"),
                ExplainabilityContentMode::Metadata,
                metadata_sink.clone(),
            ),
        ),
    )
    .await
    .expect("metadata Basic Query");
    assert_query_results_equal(&baseline, &metadata);

    let content_sink = Arc::new(RecordingExplainabilitySink::default());
    let content = query(
        fixture.config,
        basic_options(fixture.project.path(), query_text).with_explainability(
            QueryExplainabilityOptions::new(
                "basic-content".parse().expect("run id"),
                ExplainabilityContentMode::Content,
                content_sink.clone(),
            ),
        ),
    )
    .await
    .expect("content Basic Query");
    assert_query_results_equal(&baseline, &content);

    let requests = recorded_request_bodies(&server).await;
    assert_eq!(baseline_requests.len(), 2);
    assert_eq!(&requests[2..4], baseline_requests.as_slice());
    assert_eq!(&requests[4..6], baseline_requests.as_slice());

    let metadata_json = serde_json::to_string(&metadata_sink.records()).expect("metadata JSON");
    for secret in [query_text, "first source", "second source", "Basic answer."] {
        assert!(!metadata_json.contains(secret), "metadata leaked {secret}");
    }
    let records = content_sink.records();
    let root = records
        .iter()
        .find_map(|record| {
            matches!(record.event, ExplainabilityEvent::QueryStarted(_))
                .then(|| record.span_id.clone())
        })
        .expect("Basic root span");
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::QueryStarted(event)
            if event.method == ExplainabilityQueryMethod::Basic
                && event.query.as_deref() == Some(query_text)
    )));
    assert!(records.iter().any(|record| {
        record.parent_span_id.as_ref() == Some(&root)
            && matches!(record.event, ExplainabilityEvent::EmbeddingStarted(_))
    }));
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::ContextCompleted(event)
            if event.context.as_deref() == Some("id|text\n0|first source\n1|second source\n")
    )));
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::LlmRequestStarted(event)
            if event.prompt.as_deref().is_some_and(|prompt| prompt.contains("first source"))
    )));
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::LlmRequestCompleted(event)
            if event.response.as_deref() == Some("Basic answer.")
    )));
    assert!(matches!(
        records.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunCompleted(_))
    ));
    assert_eq!(content_sink.finish_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_explain_empty_basic_query_as_intentional_retrieval_skip() {
    let server = mount_query_stub().await;
    let fixture = fixture(&server).await;
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let result = query(
        fixture.config,
        basic_options(fixture.project.path(), "").with_explainability(
            QueryExplainabilityOptions::new(
                "basic-empty".parse().expect("run id"),
                ExplainabilityContentMode::Content,
                sink.clone(),
            ),
        ),
    )
    .await
    .expect("empty Basic Query");

    assert!(matches!(result.context.text, QueryContextText::Text(ref text) if text == "id|text\n"));
    let records = sink.records();
    assert!(
        records
            .iter()
            .any(|record| matches!(record.event, ExplainabilityEvent::BasicRetrievalSkipped(_)))
    );
    assert!(!records.iter().any(|record| matches!(
        record.event,
        ExplainabilityEvent::EmbeddingStarted(_)
            | ExplainabilityEvent::EmbeddingCompleted(_)
            | ExplainabilityEvent::CandidatesRetrieved(_)
    )));
    let requests = recorded_request_bodies(&server).await;
    assert_eq!(requests.len(), 1);
    assert!(requests[0].get("messages").is_some());
    assert!(matches!(
        records.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunCompleted(_))
    ));
}

#[tokio::test]
async fn test_should_isolate_concurrent_basic_explainability_runs_and_sink_failure() {
    let server = mount_query_stub().await;
    let fixture = fixture(&server).await;
    let engine = Arc::new(
        QueryEngine::load(fixture.config, fixture.project.path())
            .await
            .expect("Basic engine"),
    );
    let sink_a = Arc::new(RecordingExplainabilitySink::default());
    let sink_b = Arc::new(RecordingExplainabilitySink::failing());
    let options_a = basic_options(fixture.project.path(), "Basic A").with_explainability(
        QueryExplainabilityOptions::new(
            "basic-concurrent-a".parse().expect("run id"),
            ExplainabilityContentMode::Content,
            sink_a.clone(),
        ),
    );
    let options_b = basic_options(fixture.project.path(), "Basic B").with_explainability(
        QueryExplainabilityOptions::new(
            "basic-concurrent-b".parse().expect("run id"),
            ExplainabilityContentMode::Metadata,
            sink_b.clone(),
        ),
    );

    let (result_a, result_b) = tokio::join!(engine.query(options_a), engine.query(options_b));
    assert_eq!(result_a.expect("Basic A").response, "Basic answer.");
    assert_eq!(result_b.expect("Basic B").response, "Basic answer.");
    assert!(
        sink_a
            .records()
            .iter()
            .all(|record| record.run_id.as_str() == "basic-concurrent-a")
    );
    assert!(
        sink_b
            .records()
            .iter()
            .all(|record| record.run_id.as_str() == "basic-concurrent-b")
    );
    assert!(matches!(
        sink_a.records().last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunCompleted(_))
    ));
    assert!(matches!(
        sink_b.records().last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event))
            if event.error_kind == "explainability_delivery"
    ));
    assert_eq!(sink_a.finish_calls.load(Ordering::SeqCst), 1);
    assert_eq!(sink_b.finish_calls.load(Ordering::SeqCst), 1);
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

    let mut method_options = local_options(fixture.project.path(), "Who is Alice?");
    method_options.method = SearchMethod::Global;
    let result = local_search(fixture.config.clone(), method_options)
        .await
        .expect("method-specific Local Query");
    let method_request_bodies = recorded_request_bodies(&server).await;
    let unified_result = query(
        fixture.config.clone(),
        local_options(fixture.project.path(), "Who is Alice?"),
    )
    .await
    .expect("unified Local Query");
    let all_non_stream_request_bodies = recorded_request_bodies(&server).await;
    assert_eq!(
        all_non_stream_request_bodies
            .get(method_request_bodies.len()..)
            .expect("unified Local request bodies"),
        method_request_bodies.as_slice()
    );
    assert_eq!(unified_result.response, result.response);
    assert_eq!(unified_result.usage, result.usage);
    assert_eq!(
        format!("{:?}", unified_result.context),
        format!("{:?}", result.context)
    );
    assert_eq!(result.response, "Basic answer.");
    let QueryContextText::Text(context) = &result.context.text else {
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
    let mut completed = None;
    while let Some(event) = events.next().await {
        match event.expect("Local stream event") {
            QueryEvent::Token(token) => chunks.push(token),
            QueryEvent::Completed(stream_result) => completed = Some(stream_result),
            QueryEvent::Context(_) => {}
            _ => panic!("unexpected future Query event"),
        }
    }
    assert_eq!(chunks, ["Basic ", "answer."]);
    let completed = completed.expect("Local completed result");
    assert_eq!(completed.response, result.response);
    assert_eq!(completed.usage, result.usage);
    assert_eq!(
        format!("{:?}", completed.context),
        format!("{:?}", result.context)
    );
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
    assert_eq!(completions.len(), 3);
    assert!(completions.iter().all(|request| request["stream"] == true));
    assert!(completions.iter().all(|request| {
        request["messages"][0]["content"]
            .as_str()
            .is_some_and(|content| content.contains("-----Entities-----"))
    }));
}

fn assert_query_results_equal(
    expected: &graphloom::query::QueryResult,
    actual: &graphloom::query::QueryResult,
) {
    assert_eq!(actual.response, expected.response);
    assert_eq!(actual.usage, expected.usage);
    assert_eq!(
        format!("{:?}", actual.context),
        format!("{:?}", expected.context)
    );
}

fn explainability_event_name(event: &ExplainabilityEvent) -> String {
    serde_json::to_value(event)
        .expect("Explainability event JSON")
        .get("type")
        .and_then(Value::as_str)
        .expect("Explainability discriminator")
        .to_owned()
}

fn options_with_explainability(
    root: &Path,
    run_id: &str,
    mode: ExplainabilityContentMode,
    sink: Arc<dyn ExplainabilitySink>,
) -> QueryOptions {
    local_options(root, "Who is Alice?").with_explainability(QueryExplainabilityOptions::new(
        run_id.parse().expect("valid test run id"),
        mode,
        sink,
    ))
}

#[tokio::test]
async fn test_should_finalize_local_runs_when_one_shot_runtime_loading_fails() {
    let server = mount_query_stub().await;
    let mut fixture = local_fixture(&server).await;
    fixture.config.local_search.embedding_model_id = "missing-model".to_owned();
    let baseline = local_search(
        fixture.config.clone(),
        local_options(fixture.project.path(), "Who is Alice?"),
    )
    .await;
    let Err(baseline_error) = baseline else {
        panic!("invalid Local embedding configuration must fail runtime loading");
    };
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let explained_options = options_with_explainability(
        fixture.project.path(),
        "run-runtime-failure",
        ExplainabilityContentMode::Metadata,
        sink.clone(),
    );

    let explained = local_search(fixture.config, explained_options).await;
    let Err(explained_error) = explained else {
        panic!("explained invalid Local embedding configuration must fail runtime loading");
    };

    assert_eq!(explained_error.to_string(), baseline_error.to_string());
    assert_eq!(
        format!("{explained_error:?}"),
        format!("{baseline_error:?}")
    );
    assert!(matches!(
        explained_error,
        GraphLoomError::Query(error)
            if matches!(error.as_ref(), QueryError::InvalidQueryConfig { .. })
    ));
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
    let records = sink.records();
    assert_eq!(
        records
            .iter()
            .map(|record| explainability_event_name(&record.event))
            .collect::<Vec<_>>(),
        ["run_started", "query_started", "run_failed"]
    );
    assert!(matches!(
        records.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event))
            if event.error_kind == "invalid_query_config"
    ));
    assert!(
        records
            .iter()
            .all(|record| record.run_id.as_str() == "run-runtime-failure")
    );
    assert!(recorded_request_bodies(&server).await.is_empty());
}

#[tokio::test]
async fn test_should_finalize_local_stream_when_runtime_loading_and_sink_fail() {
    let server = mount_query_stub().await;
    let mut fixture = local_fixture(&server).await;
    fixture.config.local_search.embedding_model_id = "missing-model".to_owned();
    let baseline = local_search_streaming(
        fixture.config.clone(),
        local_options(fixture.project.path(), "Who is Alice?"),
    )
    .await;
    let Err(baseline_error) = baseline else {
        panic!("invalid Local embedding configuration must fail before producing a stream");
    };
    let sink = Arc::new(RecordingExplainabilitySink {
        fail_emit: true,
        fail_finish: true,
        ..RecordingExplainabilitySink::default()
    });

    let explained_options = options_with_explainability(
        fixture.project.path(),
        "run-stream-runtime-failure",
        ExplainabilityContentMode::Metadata,
        sink.clone(),
    );
    let explained = local_search_streaming(fixture.config, explained_options).await;
    let Err(explained_error) = explained else {
        panic!(
            "explained invalid Local embedding configuration must fail before producing a stream"
        );
    };

    assert_eq!(explained_error.to_string(), baseline_error.to_string());
    assert_eq!(
        format!("{explained_error:?}"),
        format!("{baseline_error:?}")
    );
    assert_eq!(sink.emit_calls.load(Ordering::SeqCst), 3);
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        sink.records()
            .iter()
            .filter(|record| matches!(record.event, ExplainabilityEvent::RunFailed(_)))
            .count(),
        1
    );
    assert!(matches!(
        sink.records().last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event))
            if event.error_kind == "invalid_query_config"
    ));
    assert!(recorded_request_bodies(&server).await.is_empty());
}

#[tokio::test]
async fn test_should_finalize_and_isolate_warm_engine_root_mismatch() {
    let server = mount_query_stub().await;
    let fixture = local_fixture(&server).await;
    let engine = QueryEngine::load(fixture.config, fixture.project.path())
        .await
        .expect("Local Query engine");
    let sink_a = Arc::new(RecordingExplainabilitySink::default());
    engine
        .query(options_with_explainability(
            fixture.project.path(),
            "run-warm-a",
            ExplainabilityContentMode::Metadata,
            sink_a.clone(),
        ))
        .await
        .expect("warm Local run");
    let sink_b = Arc::new(RecordingExplainabilitySink::default());
    let mismatch_root = fixture.project.path().join("other");

    let failed = engine
        .query_stream(options_with_explainability(
            &mismatch_root,
            "run-warm-b",
            ExplainabilityContentMode::Metadata,
            sink_b.clone(),
        ))
        .await;

    assert!(matches!(
        failed,
        Err(GraphLoomError::Query(error))
            if matches!(error.as_ref(), QueryError::InvalidQueryConfig { .. })
    ));
    assert_eq!(sink_a.finish_calls.load(Ordering::SeqCst), 1);
    assert_eq!(sink_b.finish_calls.load(Ordering::SeqCst), 1);
    assert!(
        sink_a
            .records()
            .iter()
            .all(|record| record.run_id.as_str() == "run-warm-a")
    );
    assert!(
        sink_b
            .records()
            .iter()
            .all(|record| record.run_id.as_str() == "run-warm-b")
    );
    assert_eq!(
        sink_b
            .records()
            .iter()
            .filter(|record| matches!(record.event, ExplainabilityEvent::RunFailed(_)))
            .count(),
        1
    );
    assert!(matches!(
        sink_b.records().last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event))
            if event.error_kind == "invalid_query_config"
    ));
}

#[tokio::test]
async fn test_should_instrument_local_query_without_changing_business_behavior() {
    let server = mount_query_stub().await;
    let fixture = local_fixture(&server).await;

    let baseline = local_search(
        fixture.config.clone(),
        local_options(fixture.project.path(), "Who is Alice?"),
    )
    .await
    .expect("baseline Local Query");
    let all_requests = recorded_request_bodies(&server).await;
    let baseline_requests = all_requests.clone();

    let noop: Arc<dyn ExplainabilitySink> = Arc::new(NoopExplainabilitySink::new());
    let noop_result = module_local_search(
        fixture.config.clone(),
        options_with_explainability(
            fixture.project.path(),
            "run-noop",
            ExplainabilityContentMode::Metadata,
            noop,
        ),
    )
    .await
    .expect("Noop Explainability Local Query");
    assert_query_results_equal(&baseline, &noop_result);

    let metadata = Arc::new(RecordingExplainabilitySink::default());
    let metadata_result = local_search(
        fixture.config.clone(),
        options_with_explainability(
            fixture.project.path(),
            "run-metadata",
            ExplainabilityContentMode::Metadata,
            metadata.clone(),
        ),
    )
    .await
    .expect("Recording Explainability Local Query");
    assert_query_results_equal(&baseline, &metadata_result);

    let content = Arc::new(RecordingExplainabilitySink::default());
    let content_result = query(
        fixture.config.clone(),
        options_with_explainability(
            fixture.project.path(),
            "run-content",
            ExplainabilityContentMode::Content,
            content.clone(),
        ),
    )
    .await
    .expect("Content Explainability Local Query");
    assert_query_results_equal(&baseline, &content_result);

    let debug = Arc::new(RecordingExplainabilitySink::default());
    let debug_result = module_query(
        fixture.config.clone(),
        options_with_explainability(
            fixture.project.path(),
            "run-debug",
            ExplainabilityContentMode::Debug,
            debug.clone(),
        ),
    )
    .await
    .expect("Debug Explainability Local Query");
    assert_query_results_equal(&baseline, &debug_result);

    let failing = Arc::new(RecordingExplainabilitySink::failing());
    let failing_result = local_search(
        fixture.config.clone(),
        options_with_explainability(
            fixture.project.path(),
            "run-failing",
            ExplainabilityContentMode::Metadata,
            failing.clone(),
        ),
    )
    .await
    .expect("Failing Explainability Local Query");
    assert_query_results_equal(&baseline, &failing_result);

    let chain_success = Arc::new(RecordingExplainabilitySink::default());
    let chain_failure = Arc::new(RecordingExplainabilitySink::failing());
    let chain: Arc<dyn ExplainabilitySink> = Arc::new(ExplainabilitySinkChain::new(vec![
        chain_success.clone(),
        chain_failure.clone(),
    ]));
    let chain_result = local_search(
        fixture.config.clone(),
        options_with_explainability(
            fixture.project.path(),
            "run-chain",
            ExplainabilityContentMode::Metadata,
            chain,
        ),
    )
    .await
    .expect("partially failing Explainability chain Local Query");
    assert_query_results_equal(&baseline, &chain_result);

    let requests = recorded_request_bodies(&server).await;
    let requests_per_query = baseline_requests.len();
    assert!(requests_per_query > 0);
    assert_eq!(requests.len(), requests_per_query.saturating_mul(7));
    for request_batch in requests.chunks(requests_per_query) {
        assert_eq!(request_batch, baseline_requests.as_slice());
    }

    let metadata_records = metadata.records();
    let names = metadata_records
        .iter()
        .map(|record| explainability_event_name(&record.event))
        .collect::<Vec<_>>();
    assert_eq!(names.first().map(String::as_str), Some("run_started"));
    assert_eq!(names.last().map(String::as_str), Some("run_completed"));
    assert_eq!(
        names.iter().filter(|name| name.starts_with("run_")).count(),
        2
    );
    assert_eq!(
        names,
        [
            "run_started",
            "query_started",
            "mapping_query_built",
            "embedding_started",
            "embedding_completed",
            "candidates_retrieved",
            "candidates_filtered",
            "entities_selected",
            "graph_expansion_started",
            "context_budget_allocated",
            "community_reports_selected",
            "relationships_selected",
            "covariates_selected",
            "text_units_selected",
            "context_section_built",
            "context_section_built",
            "context_section_built",
            "context_section_built",
            "context_section_built",
            "context_completed",
            "llm_request_started",
            "llm_request_completed",
            "run_completed",
        ]
    );
    assert!(
        metadata_records
            .iter()
            .all(|record| record.run_id.as_str() == "run-metadata")
    );
    assert!(metadata_records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::QueryStarted(event)
            if event.method == ExplainabilityQueryMethod::Local
    )));
    assert_content_fields(&metadata_records, false);
    let content_records = content.records();
    let debug_records = debug.records();
    assert_content_fields(&content_records, true);
    assert_content_fields(&debug_records, true);
    let QueryContextText::Text(baseline_context) = &baseline.context.text else {
        panic!("expected Local context text");
    };
    assert_content_values(&content_records, baseline_context);
    assert_content_values(&debug_records, baseline_context);

    let budget = metadata_records
        .iter()
        .find_map(|record| match &record.event {
            ExplainabilityEvent::ContextBudgetAllocated(event) => Some(event),
            _ => None,
        });
    assert!(budget.is_some_and(|event| {
        event
            .sections
            .iter()
            .any(|section| section.section == ContextSectionKind::LocalGraph)
            && !event.sections.iter().any(|section| {
                matches!(
                    section.section,
                    ContextSectionKind::Entities
                        | ContextSectionKind::Relationships
                        | ContextSectionKind::Covariates
                )
            })
    }));
    assert_local_decision_records(&metadata_records);
    assert_span_tree(&metadata_records);

    assert_eq!(failing.finish_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        failing.emit_calls.load(Ordering::SeqCst),
        failing.records().len()
    );
    assert_eq!(failing.records().len(), metadata_records.len());
    assert!(matches!(
        failing.records().last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event))
            if event.error_kind == "explainability_delivery"
    ));
    assert_eq!(chain_success.finish_calls.load(Ordering::SeqCst), 1);
    assert_eq!(chain_failure.finish_calls.load(Ordering::SeqCst), 1);
    assert_eq!(chain_success.records().len(), chain_failure.records().len());
    assert_eq!(chain_success.records().len(), metadata_records.len());
    assert_eq!(
        chain_success.emit_calls.load(Ordering::SeqCst),
        chain_success.records().len()
    );
    assert!(matches!(
        chain_success.records().last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event))
            if event.error_kind == "explainability_delivery"
    ));
}

fn assert_content_fields(records: &[Arc<ExplainabilityRecord>], expected: bool) {
    for record in records {
        let present = match &record.event {
            ExplainabilityEvent::QueryStarted(event) => Some(event.query.is_some()),
            ExplainabilityEvent::MappingQueryBuilt(event) => Some(event.mapping_query.is_some()),
            ExplainabilityEvent::EmbeddingStarted(event) => Some(event.input.is_some()),
            ExplainabilityEvent::ContextCompleted(event) => Some(event.context.is_some()),
            ExplainabilityEvent::LlmRequestStarted(event) => Some(event.prompt.is_some()),
            ExplainabilityEvent::LlmRequestCompleted(event) => Some(event.response.is_some()),
            _ => None,
        };
        if let Some(present) = present {
            assert_eq!(present, expected);
        }
    }
}

fn assert_content_values(records: &[Arc<ExplainabilityRecord>], expected_context: &str) {
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::QueryStarted(event)
            if event.query.as_deref() == Some("Who is Alice?")
    )));
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::MappingQueryBuilt(event)
            if event.mapping_query.as_deref() == Some("Who is Alice?")
    )));
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::EmbeddingStarted(event)
            if event.input.as_deref() == Some("Who is Alice?")
    )));
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::ContextCompleted(event)
            if event.context.as_deref() == Some(expected_context)
    )));
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::LlmRequestStarted(event)
            if event.prompt.as_deref().is_some_and(|prompt| prompt.contains(expected_context))
    )));
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::LlmRequestCompleted(event)
            if event.response.as_deref() == Some("Basic answer.")
    )));
}

fn assert_local_decision_records(records: &[Arc<ExplainabilityRecord>]) {
    let retrieved = records.iter().find_map(|record| match &record.event {
        ExplainabilityEvent::CandidatesRetrieved(event) => Some(event),
        _ => None,
    });
    assert!(retrieved.is_some_and(|event| {
        event.candidates().first().is_some_and(|candidate| {
            candidate.id == "entity-a"
                && candidate.rank == Some(1)
                && candidate.score.is_some()
                && candidate.reason == Some(SelectionReason::AnnResult)
        })
    }));
    let filtered = records.iter().find_map(|record| match &record.event {
        ExplainabilityEvent::CandidatesFiltered(event) => Some(event),
        _ => None,
    });
    assert!(filtered.is_some_and(|event| {
        event.candidates().first().is_some_and(|candidate| {
            candidate.id == "entity-a"
                && candidate.selected
                && candidate.reason == Some(SelectionReason::AnnResult)
        })
    }));
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::GraphExpansionStarted(event)
            if event.seed_entity_ids == ["entity-a"]
    )));
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::CommunityReportsSelected(event)
            if event.community_reports().iter().any(|candidate| {
                candidate.id == "report-a" && candidate.selected
            })
    )));
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::RelationshipsSelected(event)
            if event.relationships().iter().any(|candidate| {
                candidate.id == "relationship-a" && candidate.selected
            })
    )));
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::CovariatesSelected(event) if event.covariates().is_empty()
    )));
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::TextUnitsSelected(event)
            if event.text_units().iter().map(|candidate| candidate.id.as_str()).collect::<Vec<_>>()
                == ["A", "B"]
    )));
    let section_ids = records
        .iter()
        .filter_map(|record| match &record.event {
            ExplainabilityEvent::ContextSectionBuilt(event) => Some((
                event.section.section,
                event.section.selected_record_ids.clone(),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(section_ids.contains(&(
        ContextSectionKind::CommunityReports,
        vec!["report-a".to_owned()]
    )));
    assert!(section_ids.contains(&(ContextSectionKind::Entities, vec!["entity-a".to_owned()])));
    assert!(section_ids.contains(&(
        ContextSectionKind::Relationships,
        vec!["relationship-a".to_owned()]
    )));
    assert!(section_ids.contains(&(
        ContextSectionKind::Sources,
        vec!["A".to_owned(), "B".to_owned()]
    )));
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::ContextSectionBuilt(event)
            if event.section.section == ContextSectionKind::Covariates
                && event.section.name.as_deref() == Some("claims")
    )));
}

fn assert_span_tree(records: &[Arc<ExplainabilityRecord>]) {
    let root = records
        .first()
        .map(|record| record.span_id.clone())
        .expect("root record");
    let mapping = records
        .iter()
        .find_map(|record| {
            matches!(record.event, ExplainabilityEvent::MappingQueryBuilt(_))
                .then(|| record.span_id.clone())
        })
        .expect("mapping span");
    for record in records {
        match &record.event {
            ExplainabilityEvent::RunStarted(_)
            | ExplainabilityEvent::RunCompleted(_)
            | ExplainabilityEvent::RunFailed(_)
            | ExplainabilityEvent::QueryStarted(_) => {
                assert_eq!(record.span_id, root);
                assert!(record.parent_span_id.is_none());
            }
            ExplainabilityEvent::EmbeddingStarted(_)
            | ExplainabilityEvent::EmbeddingCompleted(_)
            | ExplainabilityEvent::CandidatesRetrieved(_) => {
                assert_eq!(record.parent_span_id.as_ref(), Some(&mapping));
            }
            _ => assert_eq!(record.parent_span_id.as_ref(), Some(&root)),
        }
    }
    let stage_spans = [
        records.iter().find_map(|record| {
            matches!(record.event, ExplainabilityEvent::RunStarted(_))
                .then(|| record.span_id.as_str())
        }),
        records.iter().find_map(|record| {
            matches!(record.event, ExplainabilityEvent::MappingQueryBuilt(_))
                .then(|| record.span_id.as_str())
        }),
        records.iter().find_map(|record| {
            matches!(record.event, ExplainabilityEvent::EmbeddingStarted(_))
                .then(|| record.span_id.as_str())
        }),
        records.iter().find_map(|record| {
            matches!(record.event, ExplainabilityEvent::CandidatesRetrieved(_))
                .then(|| record.span_id.as_str())
        }),
        records.iter().find_map(|record| {
            matches!(record.event, ExplainabilityEvent::GraphExpansionStarted(_))
                .then(|| record.span_id.as_str())
        }),
        records.iter().find_map(|record| {
            matches!(record.event, ExplainabilityEvent::ContextBudgetAllocated(_))
                .then(|| record.span_id.as_str())
        }),
        records.iter().find_map(|record| {
            matches!(record.event, ExplainabilityEvent::LlmRequestStarted(_))
                .then(|| record.span_id.as_str())
        }),
    ]
    .into_iter()
    .flatten()
    .collect::<HashSet<_>>();
    assert_eq!(stage_spans.len(), 7);
}

#[tokio::test]
async fn test_should_explain_rank_fallback_without_embedding() {
    let server = mount_query_stub().await;
    let fixture = local_fixture(&server).await;
    let baseline = local_search(
        fixture.config.clone(),
        local_options(fixture.project.path(), ""),
    )
    .await
    .expect("empty-query Local baseline");
    let baseline_requests = recorded_request_bodies(&server).await;
    assert_eq!(baseline_requests.len(), 1);
    assert!(
        baseline_requests
            .first()
            .is_some_and(|request| request.get("messages").is_some())
    );

    let sink = Arc::new(RecordingExplainabilitySink::default());
    let result = local_search(
        fixture.config,
        options_with_explainability(
            fixture.project.path(),
            "run-fallback",
            ExplainabilityContentMode::Metadata,
            sink.clone(),
        )
        .with_query(""),
    )
    .await
    .expect("empty-query Local Explainability");
    assert_query_results_equal(&baseline, &result);
    let records = sink.records();
    assert!(!records.iter().any(|record| {
        matches!(
            record.event,
            ExplainabilityEvent::EmbeddingStarted(_) | ExplainabilityEvent::EmbeddingCompleted(_)
        )
    }));
    let retrieved = records.iter().find_map(|record| match &record.event {
        ExplainabilityEvent::CandidatesRetrieved(event) => Some(event),
        _ => None,
    });
    assert!(retrieved.is_some_and(|event| {
        event.record_type() == ExplainabilityRecordType::Entity
            && !event.candidates().is_empty()
            && event
                .candidates()
                .iter()
                .all(|candidate| candidate.score.is_none() && candidate.reason.is_none())
    }));
}

trait QueryOptionsTestExt {
    fn with_query(self, query: &str) -> Self;
}

impl QueryOptionsTestExt for QueryOptions {
    fn with_query(mut self, query: &str) -> Self {
        self.query = query.to_owned();
        self
    }
}

#[tokio::test]
async fn test_should_report_stale_ann_candidate_without_changing_local_result() {
    let server = mount_query_stub().await;
    let mut fixture = local_fixture(&server).await;
    fixture.config.local_search.top_k_entities = 2;
    let store = LanceDbVectorStore::connect(&fixture.config.vector_store)
        .await
        .expect("connect Local LanceDB");
    let schema = fixture
        .config
        .vector_store
        .schema_for(ENTITY_DESCRIPTION_EMBEDDING);
    store
        .upsert_documents(
            &schema,
            &[VectorDocument {
                id: "stale-entity".to_owned(),
                vector: vec![0.25, 0.75],
            }],
        )
        .await
        .expect("stale entity vector");

    let baseline = local_search(
        fixture.config.clone(),
        local_options(fixture.project.path(), "Who is Alice?"),
    )
    .await
    .expect("stale-vector Local baseline");
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let explained = local_search(
        fixture.config,
        options_with_explainability(
            fixture.project.path(),
            "run-stale",
            ExplainabilityContentMode::Metadata,
            sink.clone(),
        ),
    )
    .await
    .expect("stale-vector explained Local Query");
    assert_query_results_equal(&baseline, &explained);

    let records = sink.records();
    let retrieved = records.iter().find_map(|record| match &record.event {
        ExplainabilityEvent::CandidatesRetrieved(event) => Some(event),
        _ => None,
    });
    assert!(retrieved.is_some_and(|event| {
        event
            .candidates()
            .iter()
            .any(|candidate| candidate.id == "stale-entity")
    }));
    let filtered = records.iter().find_map(|record| match &record.event {
        ExplainabilityEvent::CandidatesFiltered(event) => Some(event),
        _ => None,
    });
    assert!(filtered.is_some_and(|event| {
        event.candidates().iter().any(|candidate| {
            candidate.id == "stale-entity"
                && !candidate.selected
                && candidate.reason == Some(SelectionReason::StaleReference)
        })
    }));
}

#[tokio::test]
async fn test_should_capture_token_budget_prefix_without_changing_context_bytes() {
    let server = mount_query_stub().await;
    let mut fixture = local_fixture(&server).await;
    fixture.config.local_search.max_context_tokens = 20;
    fixture.config.local_search.community_prop = 0.0;
    fixture.config.local_search.text_unit_prop = 0.8;
    let baseline = local_search(
        fixture.config.clone(),
        local_options(fixture.project.path(), "Who is Alice?"),
    )
    .await
    .expect("token-budget Local baseline");
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let explained = local_search(
        fixture.config,
        options_with_explainability(
            fixture.project.path(),
            "run-budget",
            ExplainabilityContentMode::Metadata,
            sink.clone(),
        ),
    )
    .await
    .expect("token-budget explained Local Query");
    assert_query_results_equal(&baseline, &explained);

    let records = sink.records();
    let source_candidates = records.iter().find_map(|record| match &record.event {
        ExplainabilityEvent::TextUnitsSelected(event) => Some(event.text_units()),
        _ => None,
    });
    assert!(source_candidates.is_some_and(|candidates| {
        candidates.iter().any(|candidate| candidate.selected)
            && candidates.iter().any(|candidate| {
                !candidate.selected && candidate.reason == Some(SelectionReason::TokenBudget)
            })
    }));
    let source_section = records.iter().find_map(|record| match &record.event {
        ExplainabilityEvent::ContextSectionBuilt(event)
            if event.section.section == ContextSectionKind::Sources =>
        {
            Some(&event.section)
        }
        _ => None,
    });
    assert!(source_section.is_some_and(|section| {
        section.truncated
            && section.selected_count < section.candidate_count
            && section.selected_record_ids == ["A"]
    }));
}

#[tokio::test]
async fn test_should_leave_explainability_run_open_when_stream_is_dropped_early() {
    let server = mount_query_stub().await;
    let fixture = local_fixture(&server).await;
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let stream = local_search_streaming(
        fixture.config,
        options_with_explainability(
            fixture.project.path(),
            "run-dropped",
            ExplainabilityContentMode::Metadata,
            sink.clone(),
        ),
    )
    .await
    .expect("Local Explainability stream");
    drop(stream);

    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 0);
    assert!(!sink.records().iter().any(|record| {
        matches!(
            record.event,
            ExplainabilityEvent::RunCompleted(_) | ExplainabilityEvent::RunFailed(_)
        )
    }));
}

async fn query_event_snapshot(mut stream: QueryEventStream) -> Vec<String> {
    let mut snapshot = Vec::new();
    while let Some(event) = stream.next().await {
        match event.expect("Local Query event") {
            QueryEvent::Context(context) => snapshot.push(format!("context:{context:?}")),
            QueryEvent::Token(token) => snapshot.push(format!("token:{token}")),
            QueryEvent::Completed(result) => snapshot.push(format!(
                "completed:{}:{:?}:{:?}",
                result.response, result.context, result.usage
            )),
            _ => panic!("unexpected future Query event"),
        }
    }
    snapshot
}

#[tokio::test]
async fn test_should_preserve_stream_events_with_explainability_enabled() {
    let server = mount_query_stub().await;
    let fixture = local_fixture(&server).await;
    let baseline = local_search_streaming(
        fixture.config.clone(),
        local_options(fixture.project.path(), "Who is Alice?"),
    )
    .await
    .expect("baseline Local stream");
    let baseline = query_event_snapshot(baseline).await;

    let recording = Arc::new(RecordingExplainabilitySink::default());
    let failing = Arc::new(RecordingExplainabilitySink::failing());
    let chain_success = Arc::new(RecordingExplainabilitySink::default());
    let chain_failure = Arc::new(RecordingExplainabilitySink::failing());
    let cases: Vec<(&str, Arc<dyn ExplainabilitySink>)> = vec![
        ("stream-noop", Arc::new(NoopExplainabilitySink::new())),
        ("stream-recording", recording.clone()),
        ("stream-failing", failing.clone()),
        (
            "stream-chain",
            Arc::new(ExplainabilitySinkChain::new(vec![
                chain_success.clone(),
                chain_failure.clone(),
            ])),
        ),
    ];
    for (run_id, sink) in cases {
        let explained = query_stream(
            fixture.config.clone(),
            options_with_explainability(
                fixture.project.path(),
                run_id,
                ExplainabilityContentMode::Metadata,
                sink,
            ),
        )
        .await
        .expect("explained Local stream");
        assert_eq!(query_event_snapshot(explained).await, baseline);
    }

    assert_eq!(recording.finish_calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        recording.records().last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunCompleted(_))
    ));
    assert_eq!(failing.finish_calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        failing.records().last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event))
            if event.error_kind == "explainability_delivery"
    ));
    assert_eq!(chain_success.finish_calls.load(Ordering::SeqCst), 1);
    assert_eq!(chain_failure.finish_calls.load(Ordering::SeqCst), 1);
    assert_eq!(chain_success.records().len(), chain_failure.records().len());
}

#[tokio::test]
async fn test_should_ignore_finish_run_failure_after_successful_local_query() {
    let server = mount_query_stub().await;
    let fixture = local_fixture(&server).await;
    let baseline = local_search(
        fixture.config.clone(),
        local_options(fixture.project.path(), "Who is Alice?"),
    )
    .await
    .expect("baseline Local Query");
    let sink = Arc::new(RecordingExplainabilitySink {
        fail_finish: true,
        ..RecordingExplainabilitySink::default()
    });
    let explained = local_search(
        fixture.config,
        options_with_explainability(
            fixture.project.path(),
            "run-finish-failure",
            ExplainabilityContentMode::Metadata,
            sink.clone(),
        ),
    )
    .await
    .expect("Local Query with finish failure");

    assert_query_results_equal(&baseline, &explained);
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        sink.records().last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunCompleted(_))
    ));
}

#[tokio::test]
async fn test_should_preserve_handshake_error_and_finalize_explainability_run() {
    let server = mount_local_handshake_failure_stub().await;
    let fixture = local_fixture(&server).await;
    let baseline = local_search_streaming(
        fixture.config.clone(),
        local_options(fixture.project.path(), "Who is Alice?"),
    )
    .await;
    let Err(baseline_error) = baseline else {
        panic!("baseline Local stream handshake must fail");
    };
    let baseline_debug = format!("{baseline_error:?}");
    let baseline_display = baseline_error.to_string();

    let sink = Arc::new(RecordingExplainabilitySink::default());
    let result = local_search_streaming(
        fixture.config,
        options_with_explainability(
            fixture.project.path(),
            "run-handshake-error",
            ExplainabilityContentMode::Metadata,
            sink.clone(),
        ),
    )
    .await;
    let Err(explained_error) = result else {
        panic!("explained Local stream handshake must fail");
    };
    assert_eq!(format!("{explained_error:?}"), baseline_debug);
    assert_eq!(explained_error.to_string(), baseline_display);
    assert!(matches!(
        explained_error,
        GraphLoomError::Query(error)
            if matches!(
                error.as_ref(),
                QueryError::QueryCompletion {
                    operation: "start Local Search completion stream",
                    ..
                }
            )
    ));
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
    let records = sink.records();
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(record.event, ExplainabilityEvent::RunFailed(_)))
            .count(),
        1
    );
    assert!(matches!(
        records.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event)) if event.error_kind == "query_completion"
    ));
}

#[tokio::test]
async fn test_should_finalize_basic_handshake_failure_without_fake_completion() {
    let server = mount_local_handshake_failure_stub().await;
    let fixture = fixture(&server).await;
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let result = query_stream(
        fixture.config,
        basic_options(fixture.project.path(), "Basic failure").with_explainability(
            QueryExplainabilityOptions::new(
                "basic-handshake-error".parse().expect("run id"),
                ExplainabilityContentMode::Metadata,
                sink.clone(),
            ),
        ),
    )
    .await;
    let Err(error) = result else {
        panic!("Basic stream handshake must fail");
    };
    assert!(matches!(
        error,
        GraphLoomError::Query(source)
            if matches!(source.as_ref(), QueryError::QueryCompletion {
                method: SearchMethod::Basic,
                operation: "start Basic Search completion stream",
                ..
            })
    ));
    let records = sink.records();
    assert!(
        records
            .iter()
            .any(|record| matches!(record.event, ExplainabilityEvent::LlmRequestStarted(_)))
    );
    assert!(
        !records
            .iter()
            .any(|record| matches!(record.event, ExplainabilityEvent::LlmRequestCompleted(_)))
    );
    assert!(matches!(
        records.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event))
            if event.error_kind == "query_completion"
    ));
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_finalize_basic_embedding_failure_at_the_real_stage() {
    let server = mount_basic_embedding_failure_stub().await;
    let fixture = fixture(&server).await;
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let result = query(
        fixture.config,
        basic_options(fixture.project.path(), "Basic embedding failure").with_explainability(
            QueryExplainabilityOptions::new(
                "basic-embedding-error".parse().expect("run id"),
                ExplainabilityContentMode::Metadata,
                sink.clone(),
            ),
        ),
    )
    .await;
    assert!(matches!(
        result,
        Err(GraphLoomError::Query(source))
            if matches!(source.as_ref(), QueryError::QueryEmbedding { method: SearchMethod::Basic, .. })
    ));
    let records = sink.records();
    assert!(
        records
            .iter()
            .any(|record| matches!(record.event, ExplainabilityEvent::EmbeddingStarted(_)))
    );
    assert!(
        !records
            .iter()
            .any(|record| matches!(record.event, ExplainabilityEvent::EmbeddingCompleted(_)))
    );
    assert!(
        !records
            .iter()
            .any(|record| matches!(record.event, ExplainabilityEvent::CandidatesRetrieved(_)))
    );
    assert!(
        matches!(records.last().map(|record| &record.event), Some(ExplainabilityEvent::RunFailed(event)) if event.error_kind == "query_embedding")
    );
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_finalize_basic_vector_failure_without_fake_candidates() {
    let server = mount_query_stub().await;
    let fixture = fixture(&server).await;
    let engine = QueryEngine::load(fixture.config, fixture.project.path())
        .await
        .expect("Basic Query engine");
    engine
        .query(basic_options(fixture.project.path(), ""))
        .await
        .expect("warm Basic runtime without vector retrieval");
    let index = fixture
        .project
        .path()
        .join("output/lancedb/text_unit_text.lance");
    let hidden_index = fixture
        .project
        .path()
        .join("output/lancedb/text_unit_text.lance.hidden");
    tokio::fs::rename(&index, &hidden_index)
        .await
        .expect("hide Basic vector index after warming runtime");
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let result = engine
        .query(
            basic_options(fixture.project.path(), "Basic vector failure").with_explainability(
                QueryExplainabilityOptions::new(
                    "basic-vector-error".parse().expect("run id"),
                    ExplainabilityContentMode::Metadata,
                    sink.clone(),
                ),
            ),
        )
        .await;
    assert!(result.is_err());
    let records = sink.records();
    assert!(
        records
            .iter()
            .any(|record| matches!(record.event, ExplainabilityEvent::EmbeddingCompleted(_)))
    );
    assert!(
        !records
            .iter()
            .any(|record| matches!(record.event, ExplainabilityEvent::CandidatesRetrieved(_)))
    );
    assert!(matches!(
        records.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(_))
    ));
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_finalize_basic_prompt_failure_after_context_evidence() {
    let server = mount_query_stub().await;
    let mut fixture = fixture(&server).await;
    fixture.config.basic_search.prompt =
        Some("{{ context_data | graphloom_missing }}\n".to_owned());
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let result = query(
        fixture.config,
        basic_options(fixture.project.path(), "Basic prompt failure").with_explainability(
            QueryExplainabilityOptions::new(
                "basic-prompt-error".parse().expect("run id"),
                ExplainabilityContentMode::Metadata,
                sink.clone(),
            ),
        ),
    )
    .await;
    assert!(matches!(
        result,
        Err(GraphLoomError::Query(source))
            if matches!(source.as_ref(), QueryError::QueryPrompt { method: SearchMethod::Basic, .. })
    ));
    let records = sink.records();
    assert!(
        records
            .iter()
            .any(|record| matches!(record.event, ExplainabilityEvent::ContextCompleted(_)))
    );
    assert!(!records.iter().any(|record| matches!(
        record.event,
        ExplainabilityEvent::LlmRequestStarted(_) | ExplainabilityEvent::LlmRequestCompleted(_)
    )));
    assert!(
        matches!(records.last().map(|record| &record.event), Some(ExplainabilityEvent::RunFailed(event)) if event.error_kind == "query_prompt")
    );
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_finalize_basic_stream_consumption_failure_without_fake_completion() {
    let server = mount_basic_midstream_failure_stub().await;
    let fixture = fixture(&server).await;
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let mut events = query_stream(
        fixture.config,
        basic_options(fixture.project.path(), "Basic stream failure").with_explainability(
            QueryExplainabilityOptions::new(
                "basic-stream-error".parse().expect("run id"),
                ExplainabilityContentMode::Metadata,
                sink.clone(),
            ),
        ),
    )
    .await
    .expect("provider handshake");
    let mut observed_error = false;
    while let Some(event) = events.next().await {
        if event.is_err() {
            observed_error = true;
            break;
        }
    }
    assert!(
        observed_error,
        "malformed provider event must fail consumption"
    );
    let records = sink.records();
    assert!(
        records
            .iter()
            .any(|record| matches!(record.event, ExplainabilityEvent::LlmRequestStarted(_)))
    );
    assert!(
        !records
            .iter()
            .any(|record| matches!(record.event, ExplainabilityEvent::LlmRequestCompleted(_)))
    );
    assert!(
        matches!(records.last().map(|record| &record.event), Some(ExplainabilityEvent::RunFailed(event)) if event.error_kind == "query_completion")
    );
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_isolate_concurrent_explainability_requests_on_warm_engine() {
    let server = mount_query_stub().await;
    let fixture = local_fixture(&server).await;
    let engine = QueryEngine::load(fixture.config, fixture.project.path())
        .await
        .expect("Local Query engine");
    let warmed = engine
        .query(local_options(fixture.project.path(), "Who is Alice?"))
        .await
        .expect("warm Local runtime");
    let entity_table = fixture.project.path().join("output/entities.parquet");
    let hidden_entity_table = fixture
        .project
        .path()
        .join("output/entities.parquet.hidden");
    tokio::fs::rename(&entity_table, &hidden_entity_table)
        .await
        .expect("hide Local entity table after warming runtime");
    let sink_a = Arc::new(RecordingExplainabilitySink::failing());
    let sink_b = Arc::new(RecordingExplainabilitySink::default());
    let options_a = options_with_explainability(
        fixture.project.path(),
        "run-a",
        ExplainabilityContentMode::Metadata,
        sink_a.clone(),
    );
    let options_b = options_with_explainability(
        fixture.project.path(),
        "run-b",
        ExplainabilityContentMode::Metadata,
        sink_b.clone(),
    );
    let (result_a, result_b) = tokio::join!(engine.query(options_a), engine.query(options_b));
    tokio::fs::rename(&hidden_entity_table, &entity_table)
        .await
        .expect("restore Local entity table fixture");
    let result_a = result_a.expect("run-a business result");
    let result_b = result_b.expect("run-b business result");
    assert_query_results_equal(&warmed, &result_a);
    assert_query_results_equal(&warmed, &result_b);
    assert_query_results_equal(&result_a, &result_b);

    let records_a = sink_a.records();
    let records_b = sink_b.records();
    assert!(
        records_a
            .iter()
            .all(|record| record.run_id.as_str() == "run-a")
    );
    assert!(
        records_b
            .iter()
            .all(|record| record.run_id.as_str() == "run-b")
    );
    assert!(matches!(
        records_a.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event))
            if event.error_kind == "explainability_delivery"
    ));
    assert!(matches!(
        records_b.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunCompleted(_))
    ));
    let spans_a = records_a
        .iter()
        .map(|record| record.span_id.as_str().to_owned())
        .collect::<HashSet<_>>();
    let spans_b = records_b
        .iter()
        .map(|record| record.span_id.as_str().to_owned())
        .collect::<HashSet<_>>();
    assert!(spans_a.is_disjoint(&spans_b));
    assert_eq!(sink_a.finish_calls.load(Ordering::SeqCst), 1);
    assert_eq!(sink_b.finish_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_run_drift_api_and_stream_only_final_reduce_tokens_read_only() {
    let server = mount_drift_query_stub().await;
    let mut fixture = fixture(&server).await;
    let paths = write_local_tables(&fixture.project.path().join("output")).await;
    let before = futures_util::future::join_all(paths.iter().map(|path| async move {
        (
            file_hash(path).await,
            tokio::fs::metadata(path)
                .await
                .expect("DRIFT table metadata")
                .modified()
                .expect("DRIFT table mtime"),
        )
    }))
    .await;
    fixture.config.drift_search.primer_folds = 1;
    fixture.config.drift_search.drift_k_followups = 1;
    fixture.config.drift_search.n_depth = 1;
    fixture.config.drift_search.concurrency = 2;
    fixture.config.drift_search.local_search_max_data_tokens = 4_000;
    let (entity_ids, report_ids) = seed_drift_vectors(&fixture.config).await;
    let entity_schema = fixture
        .config
        .vector_store
        .schema_for(ENTITY_DESCRIPTION_EMBEDDING);
    let report_schema = fixture
        .config
        .vector_store
        .schema_for(COMMUNITY_FULL_CONTENT_EMBEDDING);
    let options = QueryOptions::new(
        fixture.project.path().to_path_buf(),
        "What changed?".to_owned(),
        SearchMethod::Drift,
    );
    let non_stream_callbacks = Arc::new(RecordingQueryCallbacks::default());
    let mut non_stream_options = options.clone();
    non_stream_options
        .callbacks
        .push(non_stream_callbacks.clone());

    non_stream_options.method = SearchMethod::Basic;
    let result = drift_search(fixture.config.clone(), non_stream_options)
        .await
        .expect("method-specific DRIFT Query");
    let method_request_bodies = recorded_request_bodies(&server).await;
    let non_stream_explainability = Arc::new(RecordingExplainabilitySink::default());
    let explained_options = options
        .clone()
        .with_explainability(QueryExplainabilityOptions::new(
            "drift-non-stream".parse().expect("DRIFT run id"),
            ExplainabilityContentMode::Content,
            non_stream_explainability.clone(),
        ));
    let unified_result = query(fixture.config.clone(), explained_options)
        .await
        .expect("unified DRIFT Query");
    let all_non_stream_request_bodies = recorded_request_bodies(&server).await;
    assert_eq!(
        all_non_stream_request_bodies
            .get(method_request_bodies.len()..)
            .expect("unified DRIFT request bodies"),
        method_request_bodies.as_slice()
    );
    assert_eq!(unified_result.response, result.response);
    assert_eq!(unified_result.usage, result.usage);
    assert_eq!(
        format!("{:?}", unified_result.context),
        format!("{:?}", result.context)
    );
    assert_complete_drift_explainability(&non_stream_explainability);

    assert_eq!(result.response, "DRIFT final.");
    assert_eq!(
        result.usage.categories.keys().cloned().collect::<Vec<_>>(),
        ["action", "build_context", "primer", "reduce"]
    );
    assert_eq!(result.usage.categories["build_context"].llm_calls, 2);
    assert!(result.usage.categories["build_context"].prompt_tokens > 0);
    assert_eq!(result.usage.categories["primer"].llm_calls, 1);
    assert_eq!(result.usage.categories["action"].llm_calls, 2);
    assert!(result.usage.categories["action"].prompt_tokens > 0);
    assert_eq!(result.usage.categories["reduce"].llm_calls, 1);
    assert_eq!(
        result.usage.llm_calls,
        result
            .usage
            .categories
            .values()
            .map(|category| category.llm_calls)
            .sum::<usize>()
    );
    let QueryContextText::Composite(context) = &result.context.text else {
        panic!("expected composite DRIFT context");
    };
    assert_eq!(
        context.keys().cloned().collect::<Vec<_>>(),
        ["actions", "primer", "reduce", "state"]
    );
    let QueryContextText::Text(state_context) = &context["state"] else {
        panic!("expected DRIFT state context");
    };
    let QueryContextText::Text(reduce_context) = &context["reduce"] else {
        panic!("expected DRIFT reduce context");
    };
    assert_eq!(reduce_context, "['Primer answer.', 'Action answer.']");
    let state_value: Value = serde_json::from_str(state_context).expect("DRIFT state JSON");
    assert!(state_value["nodes"].is_array());
    assert!(state_value["edges"].is_array());
    assert_eq!(state_value["nodes"][0]["query"], "What changed?");
    assert_eq!(state_value["nodes"][1]["query"], "Who?");
    let non_stream_payloads = non_stream_callbacks
        .reduce_start_payloads
        .lock()
        .expect("non-stream DRIFT callback payloads")
        .clone();
    assert_eq!(
        non_stream_payloads.as_slice(),
        std::slice::from_ref(state_context)
    );
    assert_ne!(non_stream_payloads[0], *reduce_context);

    let callbacks = Arc::new(RecordingQueryCallbacks::default());
    let stream_explainability = Arc::new(RecordingExplainabilitySink::default());
    let mut stream_options = options;
    stream_options.callbacks.push(callbacks.clone());
    stream_options.explainability = Some(QueryExplainabilityOptions::new(
        "drift-stream".parse().expect("DRIFT stream run id"),
        ExplainabilityContentMode::Content,
        stream_explainability.clone(),
    ));
    let mut events = query_stream(fixture.config.clone(), stream_options)
        .await
        .expect("DRIFT stream");
    let mut order = Vec::new();
    let mut public_chunks = Vec::new();
    let mut completed = None;
    while let Some(event) = events.next().await {
        match event.expect("DRIFT event") {
            QueryEvent::Context(_) => order.push("context"),
            QueryEvent::Token(token) => {
                order.push("token");
                public_chunks.push(token);
            }
            QueryEvent::Completed(stream_result) => {
                order.push("completed");
                assert_eq!(stream_result.response, "DRIFT final.");
                completed = Some(stream_result);
            }
            _ => panic!("unexpected DRIFT event"),
        }
    }
    assert_eq!(public_chunks.concat(), "DRIFT final.");
    assert_eq!(order, ["context", "token", "token", "completed"]);
    let completed = completed.expect("DRIFT completed result");
    assert_eq!(completed.response, result.response);
    assert_eq!(completed.usage, result.usage);
    assert_eq!(
        format!("{:?}", completed.context),
        format!("{:?}", result.context)
    );
    assert_complete_drift_explainability(&stream_explainability);
    {
        let callback_events = callbacks.events.lock().expect("DRIFT callbacks");
        assert_eq!(
            callback_events.as_slice(),
            [
                "token:{\"response\":\"Action answer.\",\"s",
                "token:core\":80,\"follow_up_queries\":[]}",
                "context",
                "reduce_start",
                "token:DRIFT ",
                "token:final.",
                "reduce_end:DRIFT final.",
            ]
        );
    }
    let stream_payloads = callbacks
        .reduce_start_payloads
        .lock()
        .expect("stream DRIFT callback payloads")
        .clone();
    assert_eq!(stream_payloads.as_slice(), non_stream_payloads.as_slice());

    for (path, (hash, modified)) in paths.iter().zip(before) {
        assert_eq!(file_hash(path).await, hash);
        assert_eq!(
            tokio::fs::metadata(path)
                .await
                .expect("DRIFT metadata after")
                .modified()
                .expect("DRIFT mtime after"),
            modified
        );
    }
    let reopened = LanceDbVectorStore::connect(&fixture.config.vector_store)
        .await
        .expect("reopen DRIFT LanceDB");
    assert_eq!(
        reopened
            .ids(&entity_schema)
            .await
            .expect("entity ids after"),
        entity_ids
    );
    assert_eq!(
        reopened
            .ids(&report_schema)
            .await
            .expect("report ids after"),
        report_ids
    );
    assert!(!fixture.project.path().join("cache").exists());
    let requests = server.received_requests().await.expect("DRIFT requests");
    let bodies = requests
        .iter()
        .filter_map(|request| request.body_json::<Value>().ok())
        .collect::<Vec<_>>();
    let embeddings = bodies
        .iter()
        .filter(|body| body.get("input").is_some())
        .collect::<Vec<_>>();
    let completions = bodies
        .iter()
        .filter(|body| body.get("messages").is_some())
        .collect::<Vec<_>>();
    assert_eq!(embeddings.len(), 6);
    assert_eq!(completions.len(), 12);
    let assert_model_call_args = |body: &&Value| {
        assert_eq!(body["max_tokens"], 64);
        assert_eq!(body["seed"], 42);
        assert_eq!(body["stop"], json!(["END"]));
        assert_eq!(body["presence_penalty"], 0.1);
        assert_eq!(body["frequency_penalty"], 0.2);
        assert_eq!(body["custom_query_arg"], json!({"enabled": true}));
    };
    completions.iter().for_each(assert_model_call_args);
    let hyde = completions
        .iter()
        .filter(|body| {
            body["messages"][0]["content"]
                .as_str()
                .is_some_and(|content| content.starts_with("Create a hypothetical answer"))
        })
        .collect::<Vec<_>>();
    assert_eq!(hyde.len(), 3);
    assert!(hyde.iter().all(|body| {
        body["max_completion_tokens"] == 128
            && body["stream"] == false
            && body.get("response_format").is_none()
    }));
    let primer = completions
        .iter()
        .filter(|body| body["response_format"]["type"] == "json_schema")
        .collect::<Vec<_>>();
    assert_eq!(primer.len(), 3);
    assert!(
        primer
            .iter()
            .all(|body| { body["max_completion_tokens"] == 128 && body["stream"] == false })
    );
    let actions = completions
        .iter()
        .filter(|body| {
            body.get("response_format").is_none()
                && body["messages"][0]["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("'follow_up_queries': List[str]"))
        })
        .collect::<Vec<_>>();
    assert_eq!(actions.len(), 3);
    assert!(actions.iter().all(|body| {
        body["max_completion_tokens"]
            == json!(
                fixture
                    .config
                    .drift_search
                    .local_search_llm_max_gen_completion_tokens
            )
            && body["temperature"] == json!(fixture.config.drift_search.local_search_temperature)
            && body["top_p"] == json!(fixture.config.drift_search.local_search_top_p)
            && body["n"] == json!(fixture.config.drift_search.local_search_n)
            && body["stream"] == true
    }));
    let reduce = completions
        .iter()
        .filter(|body| {
            body.get("response_format").is_none()
                && !body["messages"][0]["content"]
                    .as_str()
                    .is_some_and(|content| content.starts_with("Create a hypothetical answer"))
                && !body["messages"][0]["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("'follow_up_queries': List[str]"))
        })
        .collect::<Vec<_>>();
    assert_eq!(reduce.len(), 3);
    assert!(reduce.iter().all(|body| {
        body["max_completion_tokens"]
            == json!(fixture.config.drift_search.reduce_max_completion_tokens)
            && body["temperature"] == json!(fixture.config.drift_search.reduce_temperature)
    }));
    assert_eq!(
        reduce
            .iter()
            .map(|body| body["stream"].as_bool().expect("reduce stream flag"))
            .collect::<Vec<_>>(),
        [false, false, true]
    );
    assert!(reduce.iter().all(|body| {
        body["messages"][0]["content"]
            .as_str()
            .is_some_and(|content| content.contains(reduce_context))
    }));
}

fn assert_complete_drift_explainability(sink: &RecordingExplainabilitySink) {
    let records = sink.records();
    assert!(matches!(
        records.first().map(|record| &record.event),
        Some(ExplainabilityEvent::RunStarted(_))
    ));
    assert!(matches!(
        records.get(1).map(|record| &record.event),
        Some(ExplainabilityEvent::QueryStarted(event))
            if event.method == ExplainabilityQueryMethod::Drift
    ));
    assert!(
        records
            .iter()
            .any(|record| matches!(record.event, ExplainabilityEvent::DriftHydeStarted(_)))
    );
    assert!(
        records
            .iter()
            .any(|record| matches!(record.event, ExplainabilityEvent::DriftPrimerCompleted(_)))
    );
    assert!(records.iter().any(|record| matches!(
        record.event,
        ExplainabilityEvent::DriftDepthActionsSelected(_)
    )));
    assert!(records.iter().any(|record| matches!(
        record.event,
        ExplainabilityEvent::DriftActionAttemptCompleted(_)
    )));
    assert!(records.iter().any(|record| matches!(
        record.event,
        ExplainabilityEvent::DriftReduceContextBuilt(_)
    )));
    assert!(matches!(
        records.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunCompleted(_))
    ));
    let root_span = records
        .iter()
        .find_map(|record| {
            matches!(record.event, ExplainabilityEvent::QueryStarted(_))
                .then(|| record.span_id.clone())
        })
        .expect("DRIFT root span");
    let primer_span = records
        .iter()
        .find_map(|record| {
            matches!(record.event, ExplainabilityEvent::DriftPrimerStarted(_))
                .then(|| record.span_id.clone())
        })
        .expect("DRIFT Primer span");
    let exploration_span = records
        .iter()
        .find_map(|record| {
            matches!(
                record.event,
                ExplainabilityEvent::DriftExplorationStarted(_)
            )
            .then(|| record.span_id.clone())
        })
        .expect("DRIFT Exploration span");
    let reduce = records
        .iter()
        .find_map(|record| {
            let ExplainabilityEvent::DriftReduceContextBuilt(event) = &record.event else {
                return None;
            };
            Some((record, event))
        })
        .expect("DRIFT Reduce context");
    assert_eq!(reduce.0.parent_span_id.as_ref(), Some(&root_span));
    assert_eq!(reduce.1.included_action_ids, [0, 1]);
    assert_eq!(reduce.1.included_answer_count, 2);
    assert_eq!(
        reduce.1.reduce_context.as_deref(),
        Some("['Primer answer.', 'Action answer.']")
    );
    assert!(
        reduce
            .1
            .state_context
            .as_deref()
            .is_some_and(|value| value.contains("\"query\":\"What changed?\""))
    );
    for record in &records {
        match &record.event {
            ExplainabilityEvent::DriftHydeStarted(event) => {
                assert_eq!(record.parent_span_id.as_ref(), Some(&root_span));
                assert!(!event.template_report_id.is_empty());
                assert_eq!(event.template_index, 0);
                assert_eq!(event.report_count, 1);
            }
            ExplainabilityEvent::DriftReportsRanked(event) => {
                assert_eq!(record.parent_span_id.as_ref(), Some(&root_span));
                assert_eq!(event.reports.len(), 1);
                assert_eq!(event.reports[0].rank, 1);
                assert!(!event.reports[0].report_id.is_empty());
            }
            ExplainabilityEvent::DriftPrimerFoldStarted(event) => {
                assert_eq!(record.parent_span_id.as_ref(), Some(&primer_span));
                assert_eq!(event.fold_index, 0);
                assert_eq!(event.fold_count, 1);
                assert_eq!(event.report_ids.len(), 1);
            }
            ExplainabilityEvent::DriftActionAttemptStarted(event) => {
                assert_eq!(record.parent_span_id.as_ref(), Some(&exploration_span));
                assert_eq!(event.depth_index, 0);
                assert_eq!(event.query.as_deref(), Some("Who?"));
            }
            ExplainabilityEvent::DriftActionContextBuilt(event) => {
                assert_eq!(record.parent_span_id.as_ref(), Some(&exploration_span));
                assert!(
                    event
                        .context
                        .as_deref()
                        .is_some_and(|value| !value.is_empty())
                );
            }
            ExplainabilityEvent::DriftActionAttemptCompleted(event) => {
                assert_eq!(record.parent_span_id.as_ref(), Some(&exploration_span));
                assert!(event.answer_present);
                assert!(event.answer_non_empty);
                assert_eq!(event.answer.as_deref(), Some("Action answer."));
            }
            _ => {}
        }
    }
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
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

    let mut method_options = global_options(project.path(), "What are the themes?");
    let global_explainability = Arc::new(RecordingExplainabilitySink::default());
    method_options.explainability = Some(QueryExplainabilityOptions::new(
        "ignored-global".parse().expect("Global run id"),
        ExplainabilityContentMode::Debug,
        global_explainability.clone(),
    ));
    method_options.method = SearchMethod::Local;
    let result = global_search(config.clone(), method_options)
        .await
        .expect("method-specific Global Query");
    assert_eq!(global_explainability.finish_calls.load(Ordering::SeqCst), 1);
    assert!(
        global_explainability
            .records()
            .iter()
            .any(|record| matches!(
                &record.event,
                ExplainabilityEvent::QueryStarted(event)
                    if event.method == ExplainabilityQueryMethod::Global
            ))
    );
    let method_request_bodies = recorded_request_bodies(&server).await;
    let unified_result = query(
        config.clone(),
        global_options(project.path(), "What are the themes?"),
    )
    .await
    .expect("unified Global Query");
    let all_non_stream_request_bodies = recorded_request_bodies(&server).await;
    assert_eq!(
        all_non_stream_request_bodies
            .get(method_request_bodies.len()..)
            .expect("unified Global request bodies"),
        method_request_bodies.as_slice()
    );
    assert_eq!(unified_result.response, result.response);
    assert_eq!(unified_result.usage, result.usage);
    assert_eq!(
        format!("{:?}", unified_result.context),
        format!("{:?}", result.context)
    );
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
    let mut completed = None;
    while let Some(event) = events.next().await {
        match event.expect("Global stream event") {
            QueryEvent::Token(token) => chunks.push(token),
            QueryEvent::Completed(stream_result) => completed = Some(stream_result),
            QueryEvent::Context(_) => {}
            _ => panic!("unexpected future Query event"),
        }
    }
    assert_eq!(chunks, ["Global ", "answer."]);
    let completed = completed.expect("Global completed result");
    assert_eq!(completed.response, result.response);
    assert_eq!(completed.usage, result.usage);
    assert_eq!(
        format!("{:?}", completed.context),
        format!("{:?}", result.context)
    );
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
    assert_eq!(requests.len(), 6);
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
        3
    );
    assert!(
        bodies
            .iter()
            .filter(|body| body["stream"] == false)
            .all(|body| body["response_format"] == json!({"type": "json_object"}))
    );
    assert_eq!(
        bodies.iter().filter(|body| body["stream"] == true).count(),
        3
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
async fn test_should_explain_static_global_map_reduce_without_changing_business_result() {
    let server = mount_global_explainability_stub().await;
    let project = TempDir::new().expect("project");
    write_dynamic_global_tables(&project.path().join("output")).await;
    let mut config = GraphRagConfig::default();
    config.concurrent_requests = 2;
    config.global_search.max_context_tokens = 60;
    config.global_search.data_max_tokens = 32;
    config.global_search.map_prompt = Some("MAP_PROMPT_SECRET\n{{ context_data }}".to_owned());
    config.global_search.reduce_prompt = Some("REDUCE_PROMPT_SECRET\n{{ report_data }}".to_owned());
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(&server, "chat-test"),
    );

    let baseline = query(
        config.clone(),
        global_options(project.path(), "USER_QUERY_SECRET"),
    )
    .await
    .expect("baseline Global Query");
    let baseline_requests = recorded_request_bodies(&server).await;
    assert_eq!(baseline.usage.categories["map"].llm_calls, 2);

    let metadata_sink = Arc::new(RecordingExplainabilitySink::default());
    let metadata = query(
        config.clone(),
        global_options(project.path(), "USER_QUERY_SECRET").with_explainability(
            QueryExplainabilityOptions::new(
                "global-metadata".parse().expect("run id"),
                ExplainabilityContentMode::Metadata,
                metadata_sink.clone(),
            ),
        ),
    )
    .await
    .expect("metadata Global Query");
    assert_query_results_equal(&baseline, &metadata);

    let content_sink = Arc::new(RecordingExplainabilitySink::default());
    let mut content_events = query_stream(
        config,
        global_options(project.path(), "USER_QUERY_SECRET").with_explainability(
            QueryExplainabilityOptions::new(
                "global-content".parse().expect("run id"),
                ExplainabilityContentMode::Content,
                content_sink.clone(),
            ),
        ),
    )
    .await
    .expect("content Global Query stream");
    let mut content = None;
    while let Some(event) = content_events.next().await {
        if let QueryEvent::Completed(result) = event.expect("content Global stream event") {
            content = Some(result);
        }
    }
    let content = content.expect("content Global completed result");
    assert_query_results_equal(&baseline, &content);

    let requests = recorded_request_bodies(&server).await;
    let request_count = baseline_requests.len();
    assert_eq!(request_count, 3);
    assert_eq!(requests.len(), request_count * 3);
    let sorted_requests = |values: &[Value]| {
        let mut values = values.iter().map(Value::to_string).collect::<Vec<_>>();
        values.sort();
        values
    };
    assert_eq!(
        sorted_requests(&requests[request_count..request_count * 2]),
        sorted_requests(&baseline_requests)
    );
    assert_eq!(
        sorted_requests(&requests[request_count * 2..]),
        sorted_requests(&baseline_requests)
    );

    let metadata_records = metadata_sink.records();
    let content_records = content_sink.records();
    assert_eq!(metadata_sink.finish_calls.load(Ordering::SeqCst), 1);
    assert_eq!(content_sink.finish_calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        metadata_records.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunCompleted(_))
    ));
    assert!(matches!(
        content_records.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunCompleted(_))
    ));
    assert!(content_records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::QueryStarted(event)
            if event.method == ExplainabilityQueryMethod::Global
                && event.query.as_deref() == Some("USER_QUERY_SECRET")
    )));

    let batches = content_records
        .iter()
        .filter_map(|record| match &record.event {
            ExplainabilityEvent::GlobalMapBatchBuilt(event) => Some((record, event)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(batches.len(), 2);
    for (record, batch) in &batches {
        assert_eq!(
            record.parent_span_id.as_ref().map(|id| id.as_str()),
            content_records.iter().find_map(|candidate| matches!(
                candidate.event,
                ExplainabilityEvent::GlobalMapStarted(_)
            )
            .then(|| candidate.span_id.as_str()))
        );
        assert!(batch.report_ids.iter().all(|id| id.starts_with("report-")));
        assert_eq!(
            usize::try_from(batch.report_count).expect("report count"),
            batch.report_ids.len()
        );
        let context = batch.context.as_deref().expect("Map content context");
        assert!(baseline_requests.iter().any(|request| {
            request["stream"] == false
                && request["messages"][0]["content"]
                    == Value::String(format!("MAP_PROMPT_SECRET\n{context}"))
        }));
        let same_span = content_records
            .iter()
            .filter(|candidate| candidate.span_id == record.span_id)
            .map(|candidate| explainability_event_name(&candidate.event))
            .collect::<Vec<_>>();
        assert_eq!(
            same_span,
            [
                "global_map_batch_built",
                "llm_request_started",
                "llm_request_completed",
                "global_map_points_produced",
            ]
        );
    }
    assert_ne!(batches[0].0.span_id, batches[1].0.span_id);

    let points = content_records
        .iter()
        .filter_map(|record| match &record.event {
            ExplainabilityEvent::GlobalMapPointsProduced(event) => Some(event),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(points.len(), 2);
    assert!(points.iter().flat_map(|event| &event.points).any(|point| {
        point.score == 9
            && point
                .answer
                .as_deref()
                .is_some_and(|answer| answer.contains("POINT_ANSWER_SECRET"))
    }));

    let reduce = content_records
        .iter()
        .find_map(|record| match &record.event {
            ExplainabilityEvent::GlobalReduceContextBuilt(event) => Some((record, event)),
            _ => None,
        })
        .expect("Reduce context event");
    assert_eq!(reduce.1.candidate_point_count, 4);
    assert_eq!(reduce.1.positive_point_count, 3);
    assert!(reduce.1.selected_point_count < reduce.1.positive_point_count);
    assert!(reduce.1.truncated);
    assert!(reduce.1.points.iter().any(|point| {
        point.score == 0
            && !point.selected
            && point.reason == GlobalMapPointDecisionReason::NonPositiveScore
    }));
    assert!(reduce.1.points.iter().any(|point| {
        point.score == 7
            && !point.selected
            && point.reason == GlobalMapPointDecisionReason::TokenBudget
    }));
    let reduce_context = reduce.1.context.as_deref().expect("Reduce content context");
    let QueryContextText::Composite(context_text) = &content.context.text else {
        panic!("Global composite context");
    };
    let QueryContextText::Text(result_reduce_context) = &context_text["reduce"] else {
        panic!("Global Reduce context");
    };
    assert_eq!(reduce_context, result_reduce_context);
    assert!(baseline_requests.iter().any(|request| {
        request["stream"] == true
            && request["messages"][0]["content"]
                == Value::String(format!("REDUCE_PROMPT_SECRET\n{reduce_context}"))
    }));
    assert!(content_records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::LlmRequestCompleted(event)
            if record.span_id == reduce.0.span_id
                && event.response.as_deref() == Some("FINAL_RESPONSE_SECRET")
    )));

    let metadata_json = serde_json::to_string(&metadata_records).expect("metadata JSON");
    for secret in [
        "USER_QUERY_SECRET",
        "MAP_CONTEXT_SECRET",
        "MAP_PROMPT_SECRET",
        "MAP_RESPONSE_SECRET",
        "POINT_ANSWER_SECRET",
        "REDUCE_CONTEXT_SECRET",
        "REDUCE_PROMPT_SECRET",
        "FINAL_RESPONSE_SECRET",
    ] {
        assert!(!metadata_json.contains(secret), "metadata leaked {secret}");
    }
    let content_json = serde_json::to_string(&content_records).expect("content JSON");
    for secret in [
        "USER_QUERY_SECRET",
        "MAP_CONTEXT_SECRET",
        "MAP_PROMPT_SECRET",
        "MAP_RESPONSE_SECRET",
        "POINT_ANSWER_SECRET",
        "REDUCE_CONTEXT_SECRET",
        "REDUCE_PROMPT_SECRET",
        "FINAL_RESPONSE_SECRET",
    ] {
        assert!(content_json.contains(secret), "content omitted {secret}");
    }
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
    let explainability = Arc::new(RecordingExplainabilitySink::default());
    let mut options = global_options(project.path(), "Unknown?");
    options.callbacks.push(callbacks.clone());
    options.explainability = Some(QueryExplainabilityOptions::new(
        "global-no-data".parse().expect("run id"),
        ExplainabilityContentMode::Content,
        explainability.clone(),
    ));

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
    let records = explainability.records();
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::GlobalMapPointsProduced(event)
            if event.points.len() == 1
                && event.points.first().is_some_and(|point| {
                    point.score == 0 && point.answer.as_deref() == Some("")
                })
    )));
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::GlobalReduceContextBuilt(event)
            if event.positive_point_count == 0 && event.selected_point_count == 0
    )));
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::GlobalReduceSkipped(event)
            if event.reason == GlobalReduceSkipReason::NoPositivePoints
    )));
    let reduce_span = records
        .iter()
        .find_map(|record| {
            matches!(
                record.event,
                ExplainabilityEvent::GlobalReduceContextBuilt(_)
            )
            .then(|| record.span_id.clone())
        })
        .expect("Reduce span");
    assert!(!records.iter().any(|record| {
        record.span_id == reduce_span
            && matches!(
                record.event,
                ExplainabilityEvent::LlmRequestStarted(_)
                    | ExplainabilityEvent::LlmRequestCompleted(_)
            )
    }));
    assert!(matches!(
        records.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunCompleted(_))
    ));
    assert_eq!(explainability.finish_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_isolate_static_and_dynamic_global_snapshots_in_both_query_orders() {
    let server = mount_global_query_stub().await;
    let project = TempDir::new().expect("project");
    write_dynamic_global_tables(&project.path().join("output")).await;
    let mut config = GraphRagConfig::default();
    config.global_search.max_context_tokens = 10_000;
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(&server, "chat-test"),
    );

    let fresh_static_engine = QueryEngine::load(config.clone(), project.path())
        .await
        .expect("fresh static engine");
    let fresh_static = run_global_snapshot(&fresh_static_engine, project.path(), false).await;
    let fresh_dynamic_engine = QueryEngine::load(config.clone(), project.path())
        .await
        .expect("fresh dynamic engine");
    let fresh_dynamic = run_global_snapshot(&fresh_dynamic_engine, project.path(), true).await;
    assert_ne!(fresh_static.report_ids, fresh_dynamic.report_ids);
    assert_eq!(fresh_static.report_ids.len(), 3);
    assert_eq!(fresh_dynamic.report_ids.len(), 4);

    let static_first_engine = QueryEngine::load(config.clone(), project.path())
        .await
        .expect("static-first engine");
    let static_first = run_global_snapshot(&static_first_engine, project.path(), false).await;
    let dynamic_second = run_global_snapshot(&static_first_engine, project.path(), true).await;
    assert_global_snapshot_eq(&static_first, &fresh_static);
    assert_global_snapshot_eq(&dynamic_second, &fresh_dynamic);

    let dynamic_first_engine = QueryEngine::load(config, project.path())
        .await
        .expect("dynamic-first engine");
    let dynamic_first = run_global_snapshot(&dynamic_first_engine, project.path(), true).await;
    let static_second = run_global_snapshot(&dynamic_first_engine, project.path(), false).await;
    assert_global_snapshot_eq(&dynamic_first, &fresh_dynamic);
    assert_global_snapshot_eq(&static_second, &fresh_static);
}

#[tokio::test]
async fn test_should_explain_dynamic_global_selection_and_reuse_map_reduce() {
    let server = mount_global_query_stub().await;
    let project = TempDir::new().expect("project");
    write_dynamic_global_tables(&project.path().join("output")).await;
    let mut config = GraphRagConfig::default();
    config.global_search.max_context_tokens = 10_000;
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(&server, "chat-test"),
    );
    let baseline_options = {
        let mut options = global_options(project.path(), "Dynamic question?");
        options.dynamic_community_selection = true;
        options
    };
    let baseline = query(config.clone(), baseline_options)
        .await
        .expect("baseline Dynamic Global Query");
    let baseline_requests = recorded_request_bodies(&server).await;

    let sink = Arc::new(RecordingExplainabilitySink::default());
    let mut options = global_options(project.path(), "Dynamic question?");
    options.dynamic_community_selection = true;
    options.explainability = Some(QueryExplainabilityOptions::new(
        "dynamic-global-content".parse().expect("run id"),
        ExplainabilityContentMode::Debug,
        sink.clone(),
    ));

    let result = query(config, options)
        .await
        .expect("explained Dynamic Global Query");

    assert_query_results_equal(&baseline, &result);
    let requests = recorded_request_bodies(&server).await;
    assert_eq!(requests.len(), baseline_requests.len().saturating_mul(2));
    let sorted_requests = |values: &[Value]| {
        let mut values = values.iter().map(Value::to_string).collect::<Vec<_>>();
        values.sort();
        values
    };
    assert_eq!(
        sorted_requests(&requests[baseline_requests.len()..]),
        sorted_requests(&baseline_requests)
    );

    let records = sink.records();
    assert!(matches!(
        records.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunCompleted(_))
    ));
    let selection_span = records
        .iter()
        .find_map(|record| {
            matches!(
                record.event,
                ExplainabilityEvent::DynamicCommunitySelectionStarted(_)
            )
            .then(|| record.span_id.clone())
        })
        .expect("Dynamic selection span");
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::DynamicCommunityTraversalWaveStarted(event)
            if event.wave_index == 0
                && event.source == graphloom::explainability::DynamicTraversalWaveSource::Initial
                && event.community_ids == ["0", "1", "2", "3"]
    )));
    let attempts = records
        .iter()
        .filter_map(|record| match &record.event {
            ExplainabilityEvent::DynamicCommunityRatingAttemptStarted(event) => {
                Some((record, event))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(attempts.len(), 4);
    assert!(attempts.iter().all(|(record, attempt)| {
        record.parent_span_id.as_ref() == Some(&selection_span)
            && attempt.repeat_index == 0
            && attempt.repeat_count == 1
            && attempt.report_id == format!("report-{}", attempt.community_id)
            && records
                .iter()
                .filter(|candidate| candidate.span_id == record.span_id)
                .map(|candidate| explainability_event_name(&candidate.event))
                .collect::<Vec<_>>()
                == [
                    "dynamic_community_rating_attempt_started",
                    "llm_request_started",
                    "llm_request_completed",
                ]
    }));
    let completed = records
        .iter()
        .find_map(|record| match &record.event {
            ExplainabilityEvent::DynamicCommunitySelectionCompleted(event) => Some(event),
            _ => None,
        })
        .expect("Dynamic selection completed");
    assert_eq!(completed.visited_count, 4);
    assert_eq!(completed.threshold_passed_count, 4);
    assert_eq!(completed.selected_count, 4);
    assert_eq!(
        completed.selected_report_ids,
        ["report-0", "report-1", "report-2", "report-3"]
    );
    assert!(records.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::GlobalMapBatchBuilt(event)
            if !event.report_ids.is_empty()
                && event.report_ids.iter().all(|id| id.starts_with("report-"))
    )));
    assert!(records.iter().any(|record| matches!(
        record.event,
        ExplainabilityEvent::GlobalReduceContextBuilt(_)
    )));
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_finalize_dynamic_global_when_rating_provider_fails() {
    let server = mount_dynamic_rating_failure_stub().await;
    let project = TempDir::new().expect("project");
    write_dynamic_global_tables(&project.path().join("output")).await;
    let mut config = GraphRagConfig::default();
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(&server, "chat-test"),
    );
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let mut options = global_options(project.path(), "Dynamic failure?");
    options.dynamic_community_selection = true;
    options.explainability = Some(QueryExplainabilityOptions::new(
        "dynamic-rating-failure".parse().expect("run id"),
        ExplainabilityContentMode::Content,
        sink.clone(),
    ));

    let error = query(config, options)
        .await
        .expect_err("rating provider failure");
    assert!(matches!(
        error,
        GraphLoomError::Query(source)
            if matches!(
                *source,
                QueryError::QueryCompletion {
                    operation: "complete dynamic community rating",
                    ..
                }
            )
    ));
    let records = sink.records();
    assert!(records.iter().any(|record| matches!(
        record.event,
        ExplainabilityEvent::DynamicCommunityRatingAttemptStarted(_)
    )));
    assert!(
        records
            .iter()
            .any(|record| matches!(record.event, ExplainabilityEvent::LlmRequestStarted(_)))
    );
    assert!(!records.iter().any(|record| matches!(
        record.event,
        ExplainabilityEvent::LlmRequestCompleted(_)
            | ExplainabilityEvent::DynamicCommunitySelectionCompleted(_)
    )));
    assert!(matches!(
        records.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event))
            if event.error_kind == "query_completion"
    ));
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_finalize_dynamic_global_without_report_backed_level_zero() {
    let server = mount_global_query_stub().await;
    let project = TempDir::new().expect("project");
    let output = project.path().join("output");
    write_dynamic_global_tables(&output).await;
    let provider = ParquetTableProvider::new(&output).expect("Parquet provider");
    let mut communities = provider
        .read_dataframe("communities")
        .await
        .expect("communities");
    communities
        .replace(
            "level",
            Series::new("level".into(), [1_i64, 1, 1, 1]).into(),
        )
        .expect("replace levels");
    provider
        .write_dataframe("communities", communities)
        .await
        .expect("write communities");
    let mut config = GraphRagConfig::default();
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(&server, "chat-test"),
    );
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let mut options = global_options(project.path(), "Missing root?");
    options.dynamic_community_selection = true;
    options.explainability = Some(QueryExplainabilityOptions::new(
        "dynamic-no-level-zero".parse().expect("run id"),
        ExplainabilityContentMode::Metadata,
        sink.clone(),
    ));

    let error = query(config, options)
        .await
        .expect_err("missing report-backed level zero");
    assert!(matches!(
        error,
        GraphLoomError::Query(source)
            if matches!(
                *source,
                QueryError::QueryContext {
                    operation: "initialize dynamic community selection",
                    ..
                }
            )
    ));
    let records = sink.records();
    assert!(matches!(
        records.first().map(|record| &record.event),
        Some(ExplainabilityEvent::RunStarted(_))
    ));
    assert!(matches!(
        records.get(1).map(|record| &record.event),
        Some(ExplainabilityEvent::QueryStarted(_))
    ));
    assert!(!records.iter().any(|record| matches!(
        record.event,
        ExplainabilityEvent::DynamicCommunitySelectionStarted(_)
            | ExplainabilityEvent::DynamicCommunitySelectionCompleted(_)
    )));
    assert!(matches!(
        records.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event)) if event.error_kind == "query_context"
    ));
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_keep_dynamic_global_result_when_explainability_sink_fails() {
    let server = mount_global_query_stub().await;
    let project = TempDir::new().expect("project");
    write_dynamic_global_tables(&project.path().join("output")).await;
    let mut config = GraphRagConfig::default();
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(&server, "chat-test"),
    );
    let mut baseline_options = global_options(project.path(), "Dynamic sink?");
    baseline_options.dynamic_community_selection = true;
    let baseline = query(config.clone(), baseline_options)
        .await
        .expect("baseline Dynamic Global Query");
    let sink = Arc::new(RecordingExplainabilitySink::failing());
    let mut explained_options = global_options(project.path(), "Dynamic sink?");
    explained_options.dynamic_community_selection = true;
    explained_options.explainability = Some(QueryExplainabilityOptions::new(
        "dynamic-sink-failure".parse().expect("run id"),
        ExplainabilityContentMode::Metadata,
        sink.clone(),
    ));

    let explained = query(config, explained_options)
        .await
        .expect("Dynamic Query survives Explainability delivery failure");
    assert_query_results_equal(&baseline, &explained);
    assert!(matches!(
        sink.records().last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event))
            if event.error_kind == "explainability_delivery"
    ));
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_keep_global_business_result_when_explainability_sink_fails() {
    let server = mount_global_query_stub().await;
    let project = TempDir::new().expect("project");
    write_local_tables(&project.path().join("output")).await;
    let mut config = GraphRagConfig::default();
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(&server, "chat-test"),
    );
    let baseline = query(config.clone(), global_options(project.path(), "Question?"))
        .await
        .expect("baseline Global Query");
    let sink = Arc::new(RecordingExplainabilitySink::failing());
    let explained = query(
        config,
        global_options(project.path(), "Question?").with_explainability(
            QueryExplainabilityOptions::new(
                "global-sink-failure".parse().expect("run id"),
                ExplainabilityContentMode::Metadata,
                sink.clone(),
            ),
        ),
    )
    .await
    .expect("Global Query survives Explainability delivery failure");

    assert_query_results_equal(&baseline, &explained);
    assert!(matches!(
        sink.records().last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event))
            if event.error_kind == "explainability_delivery"
    ));
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_isolate_concurrent_global_explainability_runs() {
    let server = mount_global_query_stub().await;
    let project = TempDir::new().expect("project");
    write_dynamic_global_tables(&project.path().join("output")).await;
    let mut config = GraphRagConfig::default();
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(&server, "chat-test"),
    );
    let engine = QueryEngine::load(config, project.path())
        .await
        .expect("Global Query engine");
    let sink_a = Arc::new(RecordingExplainabilitySink::failing());
    let sink_b = Arc::new(RecordingExplainabilitySink::default());
    let mut options_a = global_options(project.path(), "Global Run A").with_explainability(
        QueryExplainabilityOptions::new(
            "global-run-a".parse().expect("run id"),
            ExplainabilityContentMode::Content,
            sink_a.clone(),
        ),
    );
    options_a.dynamic_community_selection = true;
    let options_b = global_options(project.path(), "Global Run B").with_explainability(
        QueryExplainabilityOptions::new(
            "global-run-b".parse().expect("run id"),
            ExplainabilityContentMode::Content,
            sink_b.clone(),
        ),
    );

    let (result_a, result_b) = tokio::join!(engine.query(options_a), engine.query(options_b));
    assert!(result_a.is_ok());
    assert!(result_b.is_ok());
    let records_a = sink_a.records();
    let records_b = sink_b.records();
    assert!(
        records_a
            .iter()
            .all(|record| record.run_id.as_str() == "global-run-a")
    );
    assert!(
        records_b
            .iter()
            .all(|record| record.run_id.as_str() == "global-run-b")
    );
    assert!(records_a.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::QueryStarted(event)
            if event.query.as_deref() == Some("Global Run A")
    )));
    assert!(records_a.iter().any(|record| matches!(
        record.event,
        ExplainabilityEvent::DynamicCommunitySelectionCompleted(_)
    )));
    assert!(!records_b.iter().any(|record| matches!(
        record.event,
        ExplainabilityEvent::DynamicCommunitySelectionStarted(_)
            | ExplainabilityEvent::DynamicCommunitySelectionCompleted(_)
    )));
    assert!(records_b.iter().any(|record| matches!(
        &record.event,
        ExplainabilityEvent::QueryStarted(event)
            if event.query.as_deref() == Some("Global Run B")
    )));
    let spans_a = records_a
        .iter()
        .map(|record| record.span_id.as_str())
        .collect::<HashSet<_>>();
    assert!(
        records_b
            .iter()
            .all(|record| !spans_a.contains(record.span_id.as_str()))
    );
    assert!(matches!(
        records_a.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event))
            if event.error_kind == "explainability_delivery"
    ));
    assert!(matches!(
        records_b.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunCompleted(_))
    ));
    assert_eq!(sink_a.finish_calls.load(Ordering::SeqCst), 1);
    assert_eq!(sink_b.finish_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_revalidate_dynamic_schema_after_static_snapshot() {
    let server = mount_global_query_stub().await;
    let project = TempDir::new().expect("project");
    let output = project.path().join("output");
    write_dynamic_global_tables(&output).await;
    let provider = ParquetTableProvider::new(&output).expect("Parquet provider");
    let mut reports = provider
        .read_dataframe("community_reports")
        .await
        .expect("community reports");
    reports
        .replace(
            "title",
            Series::new(
                "title".into(),
                [None, Some("Report 1"), Some("Report 2"), Some("Report 3")],
            )
            .into(),
        )
        .expect("dynamic-only invalid report field");
    provider
        .write_dataframe("community_reports", reports)
        .await
        .expect("write dynamic-only invalid reports");

    let mut config = GraphRagConfig::default();
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(&server, "chat-test"),
    );
    let engine = QueryEngine::load(config, project.path())
        .await
        .expect("Global engine");
    run_global_snapshot(&engine, project.path(), false).await;

    let mut dynamic = global_options(project.path(), "What are the themes?");
    dynamic.dynamic_community_selection = true;
    let error = engine
        .query(dynamic)
        .await
        .expect_err("dynamic schema must be revalidated");
    assert!(matches!(
        error,
        GraphLoomError::Query(source)
            if matches!(
                *source,
                QueryError::InvalidQueryTable {
                    method: SearchMethod::Global,
                    ..
                }
            )
    ));
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
    let explainability = Arc::new(RecordingExplainabilitySink::default());
    let mut options = global_options(project.path(), "Question?");
    options.callbacks.push(callbacks.clone());
    options.explainability = Some(QueryExplainabilityOptions::new(
        "global-map-failure".parse().expect("run id"),
        ExplainabilityContentMode::Metadata,
        explainability.clone(),
    ));

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
    let records = explainability.records();
    let failed_span = records
        .iter()
        .find_map(|record| {
            matches!(record.event, ExplainabilityEvent::LlmRequestStarted(_))
                .then(|| record.span_id.clone())
        })
        .expect("failed Map request span");
    assert!(!records.iter().any(|record| {
        record.span_id == failed_span
            && matches!(record.event, ExplainabilityEvent::LlmRequestCompleted(_))
    }));
    assert!(matches!(
        records.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event))
            if event.error_kind == "query_completion"
    ));
    assert_eq!(explainability.finish_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_finalize_global_reduce_handshake_failure_without_fake_completion() {
    let server = mount_global_reduce_failure_stub(false).await;
    let project = TempDir::new().expect("project");
    write_local_tables(&project.path().join("output")).await;
    let mut config = GraphRagConfig::default();
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(&server, "chat-test"),
    );
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let options = global_options(project.path(), "Question?").with_explainability(
        QueryExplainabilityOptions::new(
            "global-reduce-handshake".parse().expect("run id"),
            ExplainabilityContentMode::Metadata,
            sink.clone(),
        ),
    );

    let error = match query_stream(config, options).await {
        Ok(_) => panic!("Reduce handshake failure must fail stream construction"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        GraphLoomError::Query(source)
            if matches!(
                source.as_ref(),
                QueryError::QueryCompletion {
                    operation: "start Global Search reduce stream",
                    ..
                }
            )
    ));
    assert_failed_reduce_explainability(&sink);
}

#[tokio::test]
async fn test_should_finalize_global_reduce_prompt_construction_failure() {
    let server = mount_global_query_stub().await;
    let project = TempDir::new().expect("project");
    write_local_tables(&project.path().join("output")).await;
    let mut config = GraphRagConfig::default();
    config.global_search.reduce_prompt = Some("{{ missing_variable }}".to_owned());
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(&server, "chat-test"),
    );
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let options = global_options(project.path(), "Question?").with_explainability(
        QueryExplainabilityOptions::new(
            "global-reduce-prompt".parse().expect("run id"),
            ExplainabilityContentMode::Metadata,
            sink.clone(),
        ),
    );

    let error = match query_stream(config, options).await {
        Ok(_) => panic!("Reduce prompt failure must fail stream construction"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        GraphLoomError::Query(source)
            if matches!(
                source.as_ref(),
                QueryError::QueryPrompt {
                    operation: "render Global Search reduce prompt",
                    ..
                }
            )
    ));
    let records = sink.records();
    let reduce_span = records
        .iter()
        .find_map(|record| {
            matches!(
                record.event,
                ExplainabilityEvent::GlobalReduceContextBuilt(_)
            )
            .then(|| record.span_id.clone())
        })
        .expect("Reduce context span");
    assert!(!records.iter().any(|record| {
        record.span_id == reduce_span
            && matches!(record.event, ExplainabilityEvent::LlmRequestStarted(_))
    }));
    assert!(matches!(
        records.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event)) if event.error_kind == "query_prompt"
    ));
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_finalize_global_reduce_stream_consumption_failure() {
    let server = mount_global_reduce_failure_stub(true).await;
    let project = TempDir::new().expect("project");
    write_local_tables(&project.path().join("output")).await;
    let mut config = GraphRagConfig::default();
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(&server, "chat-test"),
    );
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let options = global_options(project.path(), "Question?").with_explainability(
        QueryExplainabilityOptions::new(
            "global-reduce-stream".parse().expect("run id"),
            ExplainabilityContentMode::Content,
            sink.clone(),
        ),
    );
    let mut events = query_stream(config, options)
        .await
        .expect("Reduce stream handshake");
    let mut saw_error = false;
    while let Some(event) = events.next().await {
        if event.is_err() {
            saw_error = true;
            break;
        }
    }

    assert!(saw_error);
    assert_failed_reduce_explainability(&sink);
}

fn assert_failed_reduce_explainability(sink: &RecordingExplainabilitySink) {
    let records = sink.records();
    let reduce_span = records
        .iter()
        .find_map(|record| {
            matches!(
                record.event,
                ExplainabilityEvent::GlobalReduceContextBuilt(_)
            )
            .then(|| record.span_id.clone())
        })
        .expect("Reduce span");
    assert!(records.iter().any(|record| {
        record.span_id == reduce_span
            && matches!(record.event, ExplainabilityEvent::LlmRequestStarted(_))
    }));
    assert!(!records.iter().any(|record| {
        record.span_id == reduce_span
            && matches!(record.event, ExplainabilityEvent::LlmRequestCompleted(_))
    }));
    assert!(matches!(
        records.last().map(|record| &record.event),
        Some(ExplainabilityEvent::RunFailed(event))
            if event.error_kind == "query_completion"
    ));
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_return_typed_errors_for_missing_resources() {
    let server = mount_query_stub().await;
    let fixture = fixture(&server).await;

    let drift_error = query(
        fixture.config.clone(),
        QueryOptions::new(
            fixture.project.path().to_path_buf(),
            "facts".to_owned(),
            SearchMethod::Drift,
        ),
    )
    .await
    .expect_err("fixture does not contain the DRIFT community vector index");
    assert!(matches!(
        drift_error,
        GraphLoomError::Query(source) if matches!(source.as_ref(), QueryError::MissingQueryTable {
            method: SearchMethod::Drift,
            ..
        })
    ));

    let empty_drift_error = query(
        fixture.config.clone(),
        QueryOptions::new(
            fixture.project.path().to_path_buf(),
            String::new(),
            SearchMethod::Drift,
        ),
    )
    .await
    .expect_err("empty DRIFT query must fail before resource loading");
    assert!(matches!(
        empty_drift_error,
        GraphLoomError::Query(source)
            if matches!(
                source.as_ref(),
                QueryError::InvalidQueryConfig {
                    method: SearchMethod::Drift,
                    message,
                    ..
                } if message.contains("DRIFT Search query cannot be empty")
            )
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
async fn test_should_finalize_global_context_build_failure() {
    let project = TempDir::new().expect("project");
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let options = global_options(project.path(), "Question?").with_explainability(
        QueryExplainabilityOptions::new(
            "global-context-failure".parse().expect("run id"),
            ExplainabilityContentMode::Metadata,
            sink.clone(),
        ),
    );

    let result = query(GraphRagConfig::default(), options).await;

    assert!(result.is_err());
    assert_eq!(
        sink.records()
            .iter()
            .map(|record| explainability_event_name(&record.event))
            .collect::<Vec<_>>(),
        ["run_started", "query_started", "run_failed"]
    );
    assert_eq!(sink.finish_calls.load(Ordering::SeqCst), 1);
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
