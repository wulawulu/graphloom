use std::{
    io,
    str::FromStr,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use futures_util::StreamExt;
use graphloom::{
    ENTITY_DESCRIPTION_EMBEDDING, GraphRagConfig,
    api::{
        local_search, local_search_streaming,
        query::local_search_streaming as module_local_search_streaming,
    },
    explainability::{
        ExplainabilityContentMode, ExplainabilityEvent, ExplainabilityRecord, ExplainabilityRunId,
        ExplainabilitySink, ExplainabilitySinkError,
    },
    observability::{field_name, span_name},
    query::{QueryEvent, QueryExplainabilityOptions, QueryOptions, QueryResult, SearchMethod},
};
use graphloom_llm::ModelConfig;
use graphloom_storage::{ParquetTableProvider, TableProvider};
use graphloom_vectors::{LanceDbVectorStore, VectorDocument, VectorStore};
use polars_core::prelude::{Column, DataFrame, NamedFrom, Series};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

mod support;

use support::{
    CanonicalTempDir as TempDir,
    capture::{CaptureState, CapturedSpan, capture_subscriber},
};

const QUERY_SENTINEL: &str = "QUERY_SECRET_SENTINEL";
const PROMPT_SENTINEL: &str = "PROMPT_SECRET_SENTINEL";
const CONTEXT_SENTINEL: &str = "CONTEXT_SECRET_SENTINEL";
const RESPONSE_SENTINEL: &str = "RESPONSE_SECRET_SENTINEL";
const API_KEY_SENTINEL: &str = "API_KEY_SECRET_SENTINEL";
const PATH_SENTINEL: &str = "PATH_SECRET_SENTINEL";

struct LocalFixture {
    project: TempDir,
    config: GraphRagConfig,
}

fn model_config(server: &MockServer, model: &str) -> ModelConfig {
    serde_json::from_value(json!({
        "model_provider": "openai",
        "model": model,
        "api_key": API_KEY_SENTINEL,
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

async fn write_local_tables(root: &std::path::Path) {
    let provider = ParquetTableProvider::new(root).expect("Parquet provider");
    let mut entities = DataFrame::new(
        1,
        vec![
            Series::new("id".into(), ["entity-a"]).into(),
            Series::new("human_readable_id".into(), [0_i64]).into(),
            Series::new("title".into(), ["Alice"]).into(),
            Series::new("description".into(), [CONTEXT_SENTINEL]).into(),
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
            Series::new("full_content".into(), [CONTEXT_SENTINEL]).into(),
            Series::new("rank".into(), [9.0_f64]).into(),
        ],
    )
    .expect("reports");
    let mut units = DataFrame::new(
        2,
        vec![
            Series::new("id".into(), ["A", "B"]).into(),
            Series::new("text".into(), [CONTEXT_SENTINEL, "second source"]).into(),
        ],
    )
    .expect("text units");
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
}

async fn local_fixture(server: &MockServer) -> LocalFixture {
    local_fixture_with_vector_size(server, 2, vec![0.25, 0.75]).await
}

async fn local_fixture_with_vector_size(
    server: &MockServer,
    vector_size: usize,
    entity_vector: Vec<f32>,
) -> LocalFixture {
    let project = TempDir::new().expect("project");
    let output = project.path().join("output");
    tokio::fs::create_dir_all(&output)
        .await
        .expect("output dir");
    write_local_tables(&output).await;
    let mut config = GraphRagConfig::default();
    config.completion_models.insert(
        "default_completion_model".to_owned(),
        model_config(server, "chat-test"),
    );
    config.embedding_models.insert(
        "default_embedding_model".to_owned(),
        model_config(server, "embed-test"),
    );
    config.vector_store.vector_size = vector_size;
    config.vector_store.db_uri = project
        .path()
        .join("output")
        .join("lancedb")
        .display()
        .to_string();
    let store = LanceDbVectorStore::connect(&config.vector_store)
        .await
        .expect("connect LanceDB");
    let schema = config.vector_store.schema_for(ENTITY_DESCRIPTION_EMBEDDING);
    store.ensure_index(&schema).await.expect("entity index");
    store
        .upsert_documents(
            &schema,
            &[VectorDocument {
                id: "entity-a".to_owned(),
                vector: entity_vector,
            }],
        )
        .await
        .expect("entity vector");
    config.local_search.top_k_entities = 1;
    config.local_search.max_context_tokens = 4_000;
    LocalFixture { project, config }
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
         content\":\"Local \"},\"finish_reason\":null}]}\n\n",
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

async fn mount_embedding_failure_stub() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "error": {"message": "embedding provider unavailable"}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({
            "error": {"message": "unused"}
        })))
        .mount(&server)
        .await;
    server
}

async fn mount_handshake_failure_stub() -> MockServer {
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

async fn mount_midstream_failure_stub() -> MockServer {
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
    let body = concat!(
        "data: {\"id\":\"chunk-1\",\"model\":\"chat-test\",\"choices\":[{\"index\":0,\"delta\":{\"\
         content\":\"Local \"},\"finish_reason\":null}]}\n\n",
        "data: {not valid JSON}\n\n"
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;
    server
}

fn local_options(root: &std::path::Path, query_text: &str) -> QueryOptions {
    QueryOptions::new(
        root.to_path_buf(),
        query_text.to_owned(),
        SearchMethod::Local,
    )
}

#[derive(Debug, Default)]
struct RecordingExplainabilitySink {
    records: Mutex<Vec<Arc<ExplainabilityRecord>>>,
    finish_calls: std::sync::atomic::AtomicUsize,
}

impl RecordingExplainabilitySink {
    fn records(&self) -> Vec<Arc<ExplainabilityRecord>> {
        self.records.lock().expect("records").clone()
    }

    fn finish_calls(&self) -> usize {
        self.finish_calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl ExplainabilitySink for RecordingExplainabilitySink {
    async fn emit(&self, record: Arc<ExplainabilityRecord>) -> Result<(), ExplainabilitySinkError> {
        self.records.lock().expect("records").push(record);
        Ok(())
    }

    async fn finish_run(
        &self,
        _run_id: &ExplainabilityRunId,
    ) -> Result<(), ExplainabilitySinkError> {
        self.finish_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

async fn run_with_capture<T, F>(future: F) -> (T, Arc<Mutex<CaptureState>>)
where
    F: std::future::Future<Output = T>,
{
    let state = Arc::new(Mutex::new(CaptureState::default()));
    let subscriber = capture_subscriber(state.clone());
    let _guard = tracing::subscriber::set_default(subscriber);
    let output = future.await;
    drop(_guard);
    (output, state)
}

fn spans_named<'a>(state: &'a CaptureState, name: &str) -> Vec<&'a CapturedSpan> {
    state
        .spans
        .iter()
        .filter(|span| span.name == name)
        .collect()
}

fn single_span<'a>(state: &'a CaptureState, name: &str) -> &'a CapturedSpan {
    let spans = spans_named(state, name);
    assert_eq!(spans.len(), 1, "expected exactly one {name} span");
    spans[0]
}

fn span_parent_name<'a>(state: &'a CaptureState, span: &CapturedSpan) -> Option<&'a str> {
    let parent = span.parent.as_ref()?;
    state
        .spans
        .iter()
        .find(|candidate| candidate.id == *parent)
        .map(|candidate| candidate.name.as_str())
}

fn assert_query_results_equal(left: &QueryResult, right: &QueryResult) {
    assert_eq!(left.response, right.response);
    assert_eq!(left.usage, right.usage);
    assert_eq!(
        format!("{:?}", left.context.text),
        format!("{:?}", right.context.text)
    );
    assert_eq!(
        format!("{:?}", left.context.records),
        format!("{:?}", right.context.records)
    );
}

fn assert_no_content_in_capture(state: &CaptureState, forbidden: &[&str]) {
    let mut haystack = String::new();
    for span in &state.spans {
        haystack.push_str(&span.name);
        for (field, value) in &span.fields {
            haystack.push_str(field);
            haystack.push('=');
            haystack.push_str(value);
        }
    }
    for event in &state.events {
        haystack.push_str(&event.name);
        for (field, value) in &event.fields {
            haystack.push_str(field);
            haystack.push('=');
            haystack.push_str(value);
        }
    }
    for sentinel in forbidden {
        assert!(
            !haystack.contains(sentinel),
            "tracing capture leaked {sentinel}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn test_should_capture_complete_local_span_tree_on_success() {
    let server = mount_query_stub().await;
    let fixture = local_fixture(&server).await;
    let (result, state) = run_with_capture(async {
        local_search(
            fixture.config.clone(),
            local_options(fixture.project.path(), "Who is Alice?"),
        )
        .await
        .expect("Local Query")
    })
    .await;
    let state = state.lock().expect("capture state");

    for name in [
        span_name::QUERY_LOCAL,
        span_name::QUERY_RUNTIME,
        span_name::QUERY_CONTEXT,
        span_name::QUERY_ENTITY_MAPPING,
        span_name::EMBEDDING_REQUEST,
        span_name::VECTOR_SEARCH,
        span_name::QUERY_GRAPH_EXPANSION,
        span_name::QUERY_PROMPT,
        span_name::LLM_REQUEST,
    ] {
        assert_eq!(spans_named(&state, name).len(), 1, "missing {name}");
    }

    let root = single_span(&state, span_name::QUERY_LOCAL);
    let runtime = single_span(&state, span_name::QUERY_RUNTIME);
    let context = single_span(&state, span_name::QUERY_CONTEXT);
    let mapping = single_span(&state, span_name::QUERY_ENTITY_MAPPING);
    let embedding = single_span(&state, span_name::EMBEDDING_REQUEST);
    let vector = single_span(&state, span_name::VECTOR_SEARCH);
    let graph_expansion = single_span(&state, span_name::QUERY_GRAPH_EXPANSION);
    let prompt = single_span(&state, span_name::QUERY_PROMPT);
    let llm = single_span(&state, span_name::LLM_REQUEST);

    assert_eq!(
        span_parent_name(&state, runtime),
        Some(span_name::QUERY_LOCAL)
    );
    assert_eq!(
        span_parent_name(&state, context),
        Some(span_name::QUERY_LOCAL)
    );
    assert_eq!(
        span_parent_name(&state, mapping),
        Some(span_name::QUERY_CONTEXT)
    );
    assert_eq!(
        span_parent_name(&state, embedding),
        Some(span_name::QUERY_ENTITY_MAPPING)
    );
    assert_eq!(
        span_parent_name(&state, vector),
        Some(span_name::QUERY_ENTITY_MAPPING)
    );
    assert_eq!(
        span_parent_name(&state, graph_expansion),
        Some(span_name::QUERY_CONTEXT)
    );
    assert_eq!(
        span_parent_name(&state, prompt),
        Some(span_name::QUERY_LOCAL)
    );
    assert_eq!(span_parent_name(&state, llm), Some(span_name::QUERY_LOCAL));

    assert_eq!(root.field(field_name::STATUS), Some("\"ok\""));
    assert_eq!(root.field(field_name::OPERATION), Some("\"query\""));
    assert_eq!(root.field(field_name::QUERY_METHOD), Some("\"local\""));
    assert_eq!(root.field(field_name::QUERY_STREAMING), Some("false"));
    assert_eq!(
        root.field(field_name::EXPLAINABILITY_ENABLED),
        Some("false")
    );
    assert_eq!(root.field(field_name::OBSERVABILITY_VERSION), Some("1"));
    assert!(root.field(field_name::RUN_ID).is_none());
    assert_eq!(
        root.field(field_name::INPUT_TOKENS),
        Some(
            u64::try_from(result.usage.prompt_tokens)
                .expect("prompt tokens")
                .to_string()
                .as_str()
        )
    );
    assert_eq!(
        root.field(field_name::OUTPUT_TOKENS),
        Some(
            u64::try_from(result.usage.output_tokens)
                .expect("output tokens")
                .to_string()
                .as_str()
        )
    );
    assert_eq!(
        root.field(field_name::LLM_CALLS),
        Some(
            u64::try_from(result.usage.llm_calls)
                .expect("llm calls")
                .to_string()
                .as_str()
        )
    );
    assert!(root.field(field_name::ELAPSED_MS).is_some());

    assert_eq!(runtime.field(field_name::STATUS), Some("\"ok\""));
    assert_eq!(
        runtime.field(field_name::OPERATION),
        Some("\"runtime_load\"")
    );
    assert_eq!(context.field(field_name::STATUS), Some("\"ok\""));
    assert_eq!(
        context.field(field_name::OPERATION),
        Some("\"context_build\"")
    );
    assert_eq!(
        root.field(field_name::CONTEXT_TOKENS),
        context.field(field_name::CONTEXT_TOKENS)
    );
    assert!(context.field(field_name::CONTEXT_TOKENS).is_some());

    assert_eq!(mapping.field(field_name::STATUS), Some("\"ok\""));
    assert_eq!(mapping.field(field_name::CANDIDATE_COUNT), Some("1"));
    assert_eq!(mapping.field(field_name::SELECTED_COUNT), Some("1"));
    assert_eq!(embedding.field(field_name::STATUS), Some("\"ok\""));
    assert_eq!(
        embedding.field(field_name::MODEL_INSTANCE),
        Some("\"default_embedding_model\"")
    );
    assert_eq!(
        embedding.field(field_name::MODEL_PROVIDER),
        Some("\"openai\"")
    );
    assert_eq!(embedding.field(field_name::INPUT_COUNT), Some("1"));
    assert_eq!(embedding.field(field_name::INPUT_TOKENS), Some("2"));
    assert_eq!(embedding.field(field_name::EMBEDDING_DIMENSIONS), Some("2"));
    assert_eq!(vector.field(field_name::STATUS), Some("\"ok\""));
    assert_eq!(
        vector.field(field_name::VECTOR_INDEX),
        Some("\"entity_description\"")
    );
    assert_eq!(vector.field(field_name::RETRIEVAL_TOP_K), Some("2"));
    assert_eq!(vector.field(field_name::CANDIDATE_COUNT), Some("1"));
    assert_eq!(graph_expansion.field(field_name::STATUS), Some("\"ok\""));
    assert_eq!(
        graph_expansion.field(field_name::CANDIDATE_COUNT),
        Some("1")
    );
    assert_eq!(graph_expansion.field(field_name::SELECTED_COUNT), Some("1"));
    assert_eq!(prompt.field(field_name::STATUS), Some("\"ok\""));
    assert!(prompt.field(field_name::INPUT_TOKENS).is_some());
    assert_eq!(llm.field(field_name::STATUS), Some("\"ok\""));
    assert_eq!(
        llm.field(field_name::MODEL_INSTANCE),
        Some("\"default_completion_model\"")
    );
    assert_eq!(llm.field(field_name::MODEL_PROVIDER), Some("\"openai\""));
    assert_eq!(llm.field(field_name::QUERY_STREAMING), Some("true"));
    assert!(llm.field(field_name::ELAPSED_MS).is_some());

    for span in &state.spans {
        assert!(span.closed, "span {} must be closed", span.name);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn test_should_correlate_root_run_id_with_explainability_envelope() {
    let server = mount_query_stub().await;
    let fixture = local_fixture(&server).await;
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let run_id = ExplainabilityRunId::from_str("run-observability-test").expect("run id");
    let explainability = QueryExplainabilityOptions::new(
        run_id.clone(),
        ExplainabilityContentMode::Metadata,
        sink.clone(),
    );
    let mut options = local_options(fixture.project.path(), "Who is Alice?");
    options.explainability = Some(explainability);

    let (_result, state) = run_with_capture(async {
        local_search(fixture.config, options)
            .await
            .expect("Local Query")
    })
    .await;
    let state = state.lock().expect("capture state");
    let root = single_span(&state, span_name::QUERY_LOCAL);
    assert_eq!(
        root.field(field_name::RUN_ID),
        Some("\"run-observability-test\"")
    );
    assert_eq!(root.field(field_name::EXPLAINABILITY_ENABLED), Some("true"));

    let records = sink.records();
    assert!(!records.is_empty());
    assert!(
        records
            .iter()
            .all(|record| record.run_id.as_str() == "run-observability-test")
    );
    assert_eq!(
        root.field(field_name::RUN_ID),
        Some("\"run-observability-test\"")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn test_should_not_leak_content_into_tracing_capture() {
    let server = mount_query_stub().await;
    let mut fixture = local_fixture(&server).await;
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let mut options = local_options(
        fixture.project.path(),
        &format!("Who is Alice? {QUERY_SENTINEL}"),
    );
    fixture.config.local_search.prompt = Some(format!("{PROMPT_SENTINEL} {{{{ context_data }}}}"));
    options.explainability = Some(QueryExplainabilityOptions::new(
        ExplainabilityRunId::from_str("run-content-safety").expect("run id"),
        ExplainabilityContentMode::Content,
        sink.clone(),
    ));

    let (result, state) = run_with_capture(async {
        local_search(fixture.config, options)
            .await
            .expect("Local Query")
    })
    .await;
    assert!(result.response.contains("Local"));
    let state = state.lock().expect("capture state");
    assert_no_content_in_capture(
        &state,
        &[
            QUERY_SENTINEL,
            PROMPT_SENTINEL,
            CONTEXT_SENTINEL,
            RESPONSE_SENTINEL,
            API_KEY_SENTINEL,
            PATH_SENTINEL,
        ],
    );
    let records = sink.records();
    let content = records
        .iter()
        .filter_map(|record| serde_json::to_string(&record.event).ok())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(content.contains(QUERY_SENTINEL));
    assert!(content.contains(PROMPT_SENTINEL));
    assert!(content.contains(CONTEXT_SENTINEL));
}

#[tokio::test(flavor = "current_thread")]
async fn test_should_record_runtime_build_failure_without_extra_spans() {
    let server = mount_query_stub().await;
    let mut fixture = local_fixture(&server).await;
    fixture.config.local_search.embedding_model_id = "missing-model".to_owned();
    let (result, state) = run_with_capture(async {
        local_search(
            fixture.config.clone(),
            local_options(fixture.project.path(), "Who is Alice?"),
        )
        .await
    })
    .await;
    assert!(result.is_err());
    let state = state.lock().expect("capture state");
    let root = single_span(&state, span_name::QUERY_LOCAL);
    let runtime = single_span(&state, span_name::QUERY_RUNTIME);
    assert_eq!(root.field(field_name::STATUS), Some("\"error\""));
    assert_eq!(
        root.field(field_name::ERROR_KIND),
        Some("\"invalid_query_config\"")
    );
    assert_eq!(runtime.field(field_name::STATUS), Some("\"error\""));
    assert_eq!(
        runtime.field(field_name::ERROR_KIND),
        Some("\"invalid_query_config\"")
    );
    for name in [
        span_name::QUERY_CONTEXT,
        span_name::QUERY_ENTITY_MAPPING,
        span_name::EMBEDDING_REQUEST,
        span_name::VECTOR_SEARCH,
        span_name::QUERY_GRAPH_EXPANSION,
        span_name::QUERY_PROMPT,
        span_name::LLM_REQUEST,
    ] {
        assert!(spans_named(&state, name).is_empty(), "unexpected {name}");
    }
    assert_eq!(spans_named(&state, span_name::QUERY_LOCAL).len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn test_should_record_embedding_failure_at_the_right_stage() {
    let server = mount_embedding_failure_stub().await;
    let fixture = local_fixture(&server).await;
    let (result, state) = run_with_capture(async {
        local_search(
            fixture.config,
            local_options(fixture.project.path(), "Who is Alice?"),
        )
        .await
    })
    .await;
    assert!(result.is_err());
    let state = state.lock().expect("capture state");
    let root = single_span(&state, span_name::QUERY_LOCAL);
    let context = single_span(&state, span_name::QUERY_CONTEXT);
    let mapping = single_span(&state, span_name::QUERY_ENTITY_MAPPING);
    let embedding = single_span(&state, span_name::EMBEDDING_REQUEST);
    assert_eq!(root.field(field_name::STATUS), Some("\"error\""));
    assert_eq!(
        root.field(field_name::ERROR_KIND),
        Some("\"query_embedding\"")
    );
    assert_eq!(context.field(field_name::STATUS), Some("\"error\""));
    assert_eq!(
        context.field(field_name::ERROR_KIND),
        Some("\"query_embedding\"")
    );
    assert_eq!(mapping.field(field_name::STATUS), Some("\"error\""));
    assert_eq!(embedding.field(field_name::STATUS), Some("\"error\""));
    assert_eq!(
        embedding.field(field_name::ERROR_KIND),
        Some("\"query_embedding\"")
    );
    assert!(spans_named(&state, span_name::VECTOR_SEARCH).is_empty());
    assert!(spans_named(&state, span_name::LLM_REQUEST).is_empty());
    assert!(spans_named(&state, span_name::QUERY_PROMPT).is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn test_should_record_vector_search_failure() {
    let server = mount_query_stub().await;
    let fixture = local_fixture_with_vector_size(&server, 3, vec![0.25, 0.75, 0.0]).await;

    let (result, state) = run_with_capture(async {
        local_search(
            fixture.config,
            local_options(fixture.project.path(), "Who is Alice?"),
        )
        .await
    })
    .await;
    assert!(result.is_err());
    let state = state.lock().expect("capture state");
    let root = single_span(&state, span_name::QUERY_LOCAL);
    let vector = single_span(&state, span_name::VECTOR_SEARCH);
    assert_eq!(root.field(field_name::STATUS), Some("\"error\""));
    assert_eq!(
        root.field(field_name::ERROR_KIND),
        Some("\"invalid_vector_index\"")
    );
    assert_eq!(vector.field(field_name::STATUS), Some("\"error\""));
    assert_eq!(
        vector.field(field_name::ERROR_KIND),
        Some("\"invalid_vector_index\"")
    );
    assert_eq!(spans_named(&state, span_name::LLM_REQUEST).len(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn test_should_record_prompt_render_failure() {
    let server = mount_query_stub().await;
    let mut fixture = local_fixture(&server).await;
    fixture.config.local_search.prompt =
        Some("{{ context_data | graphloom_missing }}\n".to_owned());
    let (result, state) = run_with_capture(async {
        local_search(
            fixture.config,
            local_options(fixture.project.path(), "Who is Alice?"),
        )
        .await
    })
    .await;
    assert!(result.is_err());
    let state = state.lock().expect("capture state");
    let root = single_span(&state, span_name::QUERY_LOCAL);
    let prompt = single_span(&state, span_name::QUERY_PROMPT);
    assert_eq!(root.field(field_name::STATUS), Some("\"error\""));
    assert_eq!(root.field(field_name::ERROR_KIND), Some("\"query_prompt\""));
    assert_eq!(prompt.field(field_name::STATUS), Some("\"error\""));
    assert_eq!(
        prompt.field(field_name::ERROR_KIND),
        Some("\"query_prompt\"")
    );
    assert_eq!(spans_named(&state, span_name::LLM_REQUEST).len(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn test_should_record_completion_handshake_failure() {
    let server = mount_handshake_failure_stub().await;
    let fixture = local_fixture(&server).await;
    let baseline = module_local_search_streaming(
        fixture.config.clone(),
        local_options(fixture.project.path(), "Who is Alice?"),
    )
    .await;
    let Err(baseline_error) = baseline else {
        panic!("expected handshake failure");
    };

    let (result, state) = run_with_capture(async {
        module_local_search_streaming(
            fixture.config,
            local_options(fixture.project.path(), "Who is Alice?"),
        )
        .await
    })
    .await;
    let Err(error) = result else {
        panic!("expected handshake failure with capture");
    };
    assert_eq!(error.to_string(), baseline_error.to_string());
    let state = state.lock().expect("capture state");
    let root = single_span(&state, span_name::QUERY_LOCAL);
    let llm = single_span(&state, span_name::LLM_REQUEST);
    assert_eq!(root.field(field_name::STATUS), Some("\"error\""));
    assert_eq!(
        root.field(field_name::ERROR_KIND),
        Some("\"query_completion\"")
    );
    assert_eq!(llm.field(field_name::STATUS), Some("\"error\""));
    assert_eq!(
        llm.field(field_name::ERROR_KIND),
        Some("\"query_completion\"")
    );
    assert_eq!(spans_named(&state, span_name::QUERY_LOCAL).len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn test_should_record_completion_stream_midway_failure() {
    let server = mount_midstream_failure_stub().await;
    let fixture = local_fixture(&server).await;
    let baseline = module_local_search_streaming(
        fixture.config.clone(),
        local_options(fixture.project.path(), "Who is Alice?"),
    )
    .await
    .expect("baseline stream");
    let mut baseline_events = baseline;
    let mut baseline_error = None;
    while let Some(event) = baseline_events.next().await {
        if let Err(error) = event {
            baseline_error = Some(error.to_string());
            break;
        }
    }
    assert!(baseline_error.is_some());

    let state = Arc::new(Mutex::new(CaptureState::default()));
    let subscriber = capture_subscriber(state.clone());
    let _guard = tracing::subscriber::set_default(subscriber);
    let mut events = module_local_search_streaming(
        fixture.config,
        local_options(fixture.project.path(), "Who is Alice?"),
    )
    .await
    .expect("stream");
    let mut observed = Vec::new();
    let mut captured_error = None;
    while let Some(event) = events.next().await {
        match event {
            Ok(event) => observed.push(event),
            Err(error) => {
                captured_error = Some(error.to_string());
                break;
            }
        }
    }
    assert_eq!(captured_error.as_deref(), baseline_error.as_deref());
    assert!(
        observed
            .iter()
            .any(|event| matches!(event, QueryEvent::Token(_)))
    );
    assert!(
        !observed
            .iter()
            .any(|event| matches!(event, QueryEvent::Completed(_)))
    );

    let state = state.lock().expect("capture state");
    let root = single_span(&state, span_name::QUERY_LOCAL);
    let llm = single_span(&state, span_name::LLM_REQUEST);
    assert!(root.closed);
    assert!(llm.closed);
    assert_eq!(root.field(field_name::STATUS), Some("\"error\""));
    assert_eq!(
        root.field(field_name::ERROR_KIND),
        Some("\"query_completion\"")
    );
    assert_eq!(llm.field(field_name::STATUS), Some("\"error\""));
    assert_eq!(
        llm.field(field_name::ERROR_KIND),
        Some("\"query_completion\"")
    );
    drop(_guard);
}

#[tokio::test(flavor = "current_thread")]
async fn test_should_keep_llm_span_open_until_completed() {
    let server = mount_query_stub().await;
    let fixture = local_fixture(&server).await;
    let state = Arc::new(Mutex::new(CaptureState::default()));
    let subscriber = capture_subscriber(state.clone());
    let _guard = tracing::subscriber::set_default(subscriber);
    let mut stream = local_search_streaming(
        fixture.config,
        local_options(fixture.project.path(), "Who is Alice?"),
    )
    .await
    .expect("stream");
    let first = stream.next().await.expect("first event").expect("event");
    assert!(matches!(first, QueryEvent::Context(_)));

    {
        let state_guard = state.lock().expect("capture state");
        let llm = single_span(&state_guard, span_name::LLM_REQUEST);
        assert!(!llm.closed, "LLM span must stay open during consumption");
        assert_eq!(llm.field(field_name::STATUS), None);
    }

    let mut events = vec![first];
    while let Some(event) = stream.next().await {
        events.push(event.expect("event"));
    }
    assert!(
        events
            .iter()
            .any(|event| matches!(event, QueryEvent::Completed(_)))
    );
    let expected = ["Context", "Token", "Token", "Completed"];
    let actual = events
        .iter()
        .map(|event| match event {
            QueryEvent::Context(_) => "Context",
            QueryEvent::Token(_) => "Token",
            QueryEvent::Completed(_) => "Completed",
            _ => "Other",
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);

    let state = state.lock().expect("capture state");
    let root = single_span(&state, span_name::QUERY_LOCAL);
    let llm = single_span(&state, span_name::LLM_REQUEST);
    assert!(llm.closed);
    assert!(root.closed);
    assert_eq!(root.field(field_name::STATUS), Some("\"ok\""));
    assert_eq!(llm.field(field_name::STATUS), Some("\"ok\""));
    drop(_guard);
}

#[tokio::test(flavor = "current_thread")]
async fn test_should_record_abandoned_on_early_stream_drop() {
    let server = mount_query_stub().await;
    let fixture = local_fixture(&server).await;
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let mut options = local_options(fixture.project.path(), "Who is Alice?");
    options.explainability = Some(QueryExplainabilityOptions::new(
        ExplainabilityRunId::from_str("run-abandoned").expect("run id"),
        ExplainabilityContentMode::Metadata,
        sink.clone(),
    ));
    let state = Arc::new(Mutex::new(CaptureState::default()));
    let subscriber = capture_subscriber(state.clone());
    let _guard = tracing::subscriber::set_default(subscriber);
    let mut stream = local_search_streaming(fixture.config, options)
        .await
        .expect("stream");
    let first = stream.next().await.expect("first event").expect("event");
    assert!(matches!(first, QueryEvent::Context(_)));
    drop(stream);

    let state = state.lock().expect("capture state");
    let root = single_span(&state, span_name::QUERY_LOCAL);
    let llm = single_span(&state, span_name::LLM_REQUEST);
    assert_eq!(root.field(field_name::STATUS), Some("\"abandoned\""));
    assert_eq!(llm.field(field_name::STATUS), Some("\"abandoned\""));
    assert!(root.closed);
    assert!(llm.closed);
    assert!(
        !sink
            .records()
            .iter()
            .any(|record| matches!(record.event, ExplainabilityEvent::RunCompleted(_)))
    );
    assert!(
        !sink
            .records()
            .iter()
            .any(|record| matches!(record.event, ExplainabilityEvent::RunFailed(_)))
    );
    assert_eq!(sink.finish_calls(), 0);
    drop(_guard);
}

#[tokio::test(flavor = "current_thread")]
async fn test_should_skip_embedding_and_vector_spans_on_rank_fallback() {
    let server = mount_query_stub().await;
    let fixture = local_fixture(&server).await;
    let baseline = local_search(
        fixture.config.clone(),
        local_options(fixture.project.path(), ""),
    )
    .await
    .expect("baseline rank fallback");
    let (result, state) = run_with_capture(async {
        local_search(fixture.config, local_options(fixture.project.path(), ""))
            .await
            .expect("rank fallback with capture")
    })
    .await;
    assert_query_results_equal(&baseline, &result);
    let state = state.lock().expect("capture state");
    assert_eq!(
        spans_named(&state, span_name::QUERY_ENTITY_MAPPING).len(),
        1
    );
    assert!(spans_named(&state, span_name::EMBEDDING_REQUEST).is_empty());
    assert!(spans_named(&state, span_name::VECTOR_SEARCH).is_empty());
    let mapping = single_span(&state, span_name::QUERY_ENTITY_MAPPING);
    assert_eq!(mapping.field(field_name::STATUS), Some("\"ok\""));
    assert_eq!(mapping.field(field_name::CANDIDATE_COUNT), Some("1"));
    assert_eq!(mapping.field(field_name::SELECTED_COUNT), Some("1"));
}

#[tokio::test(flavor = "current_thread")]
async fn test_should_preserve_behavior_across_subscriber_configs() {
    let server = mount_query_stub().await;
    let fixture = local_fixture(&server).await;
    let vector_ids_before = vector_ids(&fixture.config).await;

    let baseline = local_search(
        fixture.config.clone(),
        local_options(fixture.project.path(), "Who is Alice?"),
    )
    .await
    .expect("baseline");
    let baseline_requests = request_bodies(&server).await;
    let baseline_offset = baseline_requests.len();

    let (captured, state) = run_with_capture(async {
        local_search(
            fixture.config.clone(),
            local_options(fixture.project.path(), "Who is Alice?"),
        )
        .await
        .expect("captured")
    })
    .await;
    assert_query_results_equal(&baseline, &captured);
    let captured_requests = request_bodies_since(&server, baseline_offset).await;
    assert_eq!(captured_requests, baseline_requests);
    {
        let state = state.lock().expect("capture state");
        assert_eq!(spans_named(&state, span_name::QUERY_LOCAL).len(), 1);
    }

    let fmt_offset = server.received_requests().await.expect("requests").len();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(io::sink)
        .with_max_level(tracing::Level::INFO)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let formatted = local_search(
        fixture.config.clone(),
        local_options(fixture.project.path(), "Who is Alice?"),
    )
    .await
    .expect("fmt subscriber");
    drop(_guard);
    assert_query_results_equal(&baseline, &formatted);
    assert_eq!(
        request_bodies_since(&server, fmt_offset).await,
        baseline_requests
    );

    let explained_offset = server.received_requests().await.expect("requests").len();
    let sink = Arc::new(RecordingExplainabilitySink::default());
    let mut explained_options = local_options(fixture.project.path(), "Who is Alice?");
    explained_options.explainability = Some(QueryExplainabilityOptions::new(
        ExplainabilityRunId::from_str("run-equivalence").expect("run id"),
        ExplainabilityContentMode::Metadata,
        sink.clone(),
    ));
    let (explained, state) = run_with_capture(async {
        local_search(fixture.config.clone(), explained_options)
            .await
            .expect("explained with capture")
    })
    .await;
    assert_query_results_equal(&baseline, &explained);
    assert_eq!(
        request_bodies_since(&server, explained_offset).await,
        baseline_requests
    );
    {
        let state = state.lock().expect("capture state");
        assert_eq!(spans_named(&state, span_name::QUERY_LOCAL).len(), 1);
        assert!(!sink.records().is_empty());
    }

    assert_eq!(vector_ids(&fixture.config).await, vector_ids_before);
}

async fn vector_ids(config: &GraphRagConfig) -> Vec<String> {
    let store = LanceDbVectorStore::connect(&config.vector_store)
        .await
        .expect("vector store");
    let schema = config.vector_store.schema_for(ENTITY_DESCRIPTION_EMBEDDING);
    store.ids(&schema).await.expect("vector ids")
}

async fn request_bodies(server: &MockServer) -> Vec<Value> {
    request_bodies_since(server, 0).await
}

async fn request_bodies_since(server: &MockServer, offset: usize) -> Vec<Value> {
    server
        .received_requests()
        .await
        .expect("requests")
        .iter()
        .skip(offset)
        .map(|request| request.body_json::<Value>().expect("request JSON"))
        .collect()
}
