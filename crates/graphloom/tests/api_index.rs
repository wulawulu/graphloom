use std::sync::{Arc, Mutex};

use graphloom::{
    ALL_EMBEDDINGS, GraphRagConfig, PipelineRunStats, WorkflowCallbacks,
    api::{BuildIndexOptions, CacheMode, IndexingMethod, build_index},
};
use graphloom_storage::{ParquetTableProvider, TableProvider};
use graphloom_vectors::{LanceDbVectorStore, VectorStore};
use serde_json::{Value, json};
use tempfile::TempDir;
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test]
async fn test_should_build_standard_index_via_public_api() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(chat_responder)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(embedding_responder)
        .mount(&server)
        .await;

    let tempdir = TempDir::new().expect("tempdir");
    tokio::fs::create_dir(tempdir.path().join("input"))
        .await
        .expect("input dir");
    tokio::fs::write(
        tempdir.path().join("input").join("document.txt"),
        "Alice works for Acme. Bob manages Acme. Alice and Bob collaborated on Project Atlas.",
    )
    .await
    .expect("input");
    let config = test_config(&server.uri());
    let callbacks = Arc::new(RecordingCallbacks::default());

    let result = build_index(
        config,
        BuildIndexOptions {
            project_root: tempdir.path().to_path_buf(),
            method: IndexingMethod::Standard,
            cache_mode: CacheMode::Configured,
            callbacks: vec![callbacks.clone()],
        },
    )
    .await
    .expect("build index");

    assert!(!result.workflow_outputs.is_empty());
    assert!(result.stats.document_count > 0);
    assert!(
        callbacks
            .started()
            .contains(&"load_input_documents".to_owned())
    );
    assert!(
        callbacks
            .completed()
            .contains(&"generate_text_embeddings".to_owned())
    );
    assert_standard_outputs(tempdir.path()).await;
}

#[derive(Debug, Default)]
struct RecordingCallbacks {
    started: Mutex<Vec<String>>,
    completed: Mutex<Vec<String>>,
}

impl RecordingCallbacks {
    fn started(&self) -> Vec<String> {
        self.started.lock().expect("started lock").clone()
    }

    fn completed(&self) -> Vec<String> {
        self.completed.lock().expect("completed lock").clone()
    }
}

impl WorkflowCallbacks for RecordingCallbacks {
    fn workflow_started(&self, workflow_name: &str) {
        self.started
            .lock()
            .expect("started lock")
            .push(workflow_name.to_owned());
    }

    fn workflow_completed(&self, workflow_name: &str, _stats: &PipelineRunStats) {
        self.completed
            .lock()
            .expect("completed lock")
            .push(workflow_name.to_owned());
    }
}

fn test_config(server_uri: &str) -> GraphRagConfig {
    serde_yaml::from_str(&format!(
        r#"
completion_models:
  default_completion_model:
    model_provider: openai
    model: gpt-test
    auth_method: api_key
    api_key: test-key
    api_base: {server_uri}/v1
embedding_models:
  default_embedding_model:
    model_provider: openai
    model: embed-test
    auth_method: api_key
    api_key: test-key
    api_base: {server_uri}/v1
input:
  type: text
  file_pattern: ".*\\.txt$"
input_storage:
  type: file
  base_dir: input
output_storage:
  type: file
  base_dir: output
reporting:
  type: file
  base_dir: logs
cache:
  type: json
  storage:
    type: file
    base_dir: cache
chunking:
  type: tokens
  size: 1200
  overlap: 100
  encoding_model: o200k_base
vector_store:
  type: lancedb
  db_uri: output/lancedb
  vector_size: 4
extract_graph:
  completion_model_id: default_completion_model
  max_gleanings: 0
summarize_descriptions:
  completion_model_id: default_completion_model
community_reports:
  completion_model_id: default_completion_model
embed_text:
  embedding_model_id: default_embedding_model
extract_claims:
  enabled: false
"#
    ))
    .expect("config")
}

async fn assert_standard_outputs(root: &std::path::Path) {
    let provider = ParquetTableProvider::new(root.join("output")).expect("provider");
    for table in [
        "documents",
        "text_units",
        "entities",
        "relationships",
        "communities",
        "community_reports",
    ] {
        let dataframe = provider.read_dataframe(table).await.expect("read table");
        assert!(dataframe.height() > 0, "{table} should be non-empty");
        assert!(dataframe.column("id").is_ok(), "{table} should have id");
    }

    let config = test_config("http://127.0.0.1:1");
    let mut config = config;
    config.vector_store.db_uri = root
        .join("output")
        .join("lancedb")
        .to_string_lossy()
        .to_string();
    let store = LanceDbVectorStore::connect(&config.vector_store)
        .await
        .expect("lancedb");
    for embedding in ALL_EMBEDDINGS {
        let schema = config.vector_store.schema_for(embedding);
        assert!(store.count(&schema).await.expect("count") > 0);
    }
}

fn chat_responder(request: &Request) -> ResponseTemplate {
    let body = request.body_json::<Value>().expect("chat request json");
    let messages = body["messages"].as_array().expect("messages");
    let last = messages
        .last()
        .and_then(|message| message["content"].as_str())
        .unwrap_or_default();
    let content = if last.contains("Given a text document") {
        "(\"entity\"<|>ALICE<|>person<|>Alice works for Acme and collaborates with \
         Bob)##(\"entity\"<|>BOB<|>person<|>Bob manages Acme and collaborates with \
         Alice)##(\"entity\"<|>ACME<|>organization<|>Acme is managed by Bob and employs \
         Alice)##(\"relationship\"<|>ALICE<|>ACME<|>Alice works for \
         Acme<|>5)##(\"relationship\"<|>BOB<|>ACME<|>Bob manages \
         Acme<|>5)##(\"relationship\"<|>ALICE<|>BOB<|>Alice and Bob collaborated on Project \
         Atlas<|>4)##<|COMPLETE|>"
            .to_owned()
    } else if last.contains("Return output as a well-formed JSON-formatted string") {
        json!({
            "title": "Alice, Bob, and Acme",
            "summary": "Alice, Bob, and Acme form a connected work community.",
            "rating": 5.0,
            "rating_explanation": "The community is small but coherent.",
            "findings": [{"summary": "Collaboration", "explanation": "Alice and Bob collaborate through Acme [Data: Entities (0, 1); Relationships (0)]."}]
        })
        .to_string()
    } else {
        "A concise summary of the provided descriptions.".to_owned()
    };
    ResponseTemplate::new(200).set_body_json(json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 0,
        "model": "gpt-test",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
    }))
}

fn embedding_responder(request: &Request) -> ResponseTemplate {
    let body = request
        .body_json::<Value>()
        .expect("embedding request json");
    let inputs = body["input"].as_array().expect("input");
    let data = inputs
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let tail = if index == 0 { 0.0 } else { 1.0 };
            json!({
                "object": "embedding",
                "index": index,
                "embedding": [1.0, 0.0, 0.0, tail]
            })
        })
        .collect::<Vec<_>>();
    ResponseTemplate::new(200).set_body_json(json!({
        "object": "list",
        "data": data,
        "model": "embed-test",
        "usage": {"prompt_tokens": inputs.len(), "total_tokens": inputs.len()}
    }))
}
