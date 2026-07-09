use std::sync::{Arc, Mutex};

use graphloom::{
    ALL_EMBEDDINGS, GraphRagConfig, PipelineRunStats, WorkflowCallbacks,
    api::{BuildIndexOptions, CacheMode, IndexingMethod, build_index},
};
use graphloom_storage::{ParquetTableProvider, TableProvider};
use graphloom_vectors::{LanceDbVectorStore, VectorDocument, VectorStore};
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
    let expected_order = test_config(&server.uri()).workflow_order();
    assert_eq!(callbacks.started(), expected_order);
    assert_eq!(callbacks.completed(), expected_order);
    assert_standard_outputs(tempdir.path()).await;
}

#[tokio::test]
async fn test_should_validate_before_destructive_reset() {
    for case in [
        InvalidConfigCase::InvalidRegex,
        InvalidConfigCase::MissingModel,
        InvalidConfigCase::MissingPrompt,
        InvalidConfigCase::MissingInput,
        InvalidConfigCase::UnsupportedProvider,
        InvalidConfigCase::InvalidTokenizer,
    ] {
        let tempdir = TempDir::new().expect("tempdir");
        let mut config = test_config("http://127.0.0.1:1");
        prepare_old_output_and_vector(tempdir.path(), &mut config).await;
        apply_invalid_case(tempdir.path(), &mut config, case).await;

        let error = build_index(
            config.clone(),
            BuildIndexOptions {
                project_root: tempdir.path().to_path_buf(),
                method: IndexingMethod::Standard,
                cache_mode: CacheMode::Configured,
                callbacks: Vec::new(),
            },
        )
        .await
        .expect_err("invalid config should fail");

        assert!(
            !error.to_string().is_empty(),
            "case {case:?} should return a useful error"
        );
        assert!(
            tempdir.path().join("output").join("sentinel.txt").is_file(),
            "case {case:?} must not clear output before validation"
        );
        assert_old_vector_still_exists(tempdir.path(), &config).await;
    }
}

#[tokio::test]
async fn test_should_run_api_without_callbacks() {
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
        "Alice works for Acme.",
    )
    .await
    .expect("input");

    let result = build_index(
        test_config(&server.uri()),
        BuildIndexOptions {
            project_root: tempdir.path().to_path_buf(),
            method: IndexingMethod::Standard,
            cache_mode: CacheMode::Disabled,
            callbacks: Vec::new(),
        },
    )
    .await
    .expect("build index");

    assert!(!result.workflow_outputs.is_empty());
    assert!(!tempdir.path().join("cache").exists());
}

#[tokio::test]
async fn test_should_fan_out_callbacks_in_api() {
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
        "Alice works for Acme.",
    )
    .await
    .expect("input");
    let first = Arc::new(RecordingCallbacks::default());
    let second = Arc::new(RecordingCallbacks::default());

    let result = build_index(
        test_config(&server.uri()),
        BuildIndexOptions {
            project_root: tempdir.path().to_path_buf(),
            method: IndexingMethod::Standard,
            cache_mode: CacheMode::Configured,
            callbacks: vec![first.clone(), second.clone()],
        },
    )
    .await
    .expect("build index");
    let expected = result.workflow_outputs.iter().map(|_| ()).count();
    assert_eq!(first.started().len(), expected);
    assert_eq!(second.started(), first.started());
    assert_eq!(second.completed(), first.completed());
}

#[derive(Debug, Clone, Copy)]
enum InvalidConfigCase {
    InvalidRegex,
    MissingModel,
    MissingPrompt,
    MissingInput,
    UnsupportedProvider,
    InvalidTokenizer,
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

async fn prepare_old_output_and_vector(root: &std::path::Path, config: &mut GraphRagConfig) {
    tokio::fs::create_dir_all(root.join("input"))
        .await
        .expect("input");
    tokio::fs::write(root.join("input").join("document.txt"), "Alice")
        .await
        .expect("input file");
    tokio::fs::create_dir_all(root.join("output"))
        .await
        .expect("output");
    tokio::fs::write(root.join("output").join("sentinel.txt"), "keep")
        .await
        .expect("sentinel");
    config.vector_store.db_uri = root
        .join("output")
        .join("lancedb")
        .to_string_lossy()
        .to_string();
    let store = LanceDbVectorStore::connect(&config.vector_store)
        .await
        .expect("lancedb");
    let schema = config.vector_store.schema_for("entity_description");
    store
        .upsert_documents(
            &schema,
            &[VectorDocument {
                id: "old-id".to_owned(),
                vector: vec![1.0, 0.0, 0.0, 0.0],
            }],
        )
        .await
        .expect("old vector");
}

async fn assert_old_vector_still_exists(root: &std::path::Path, config: &GraphRagConfig) {
    let mut config = config.clone();
    config.vector_store.db_uri = root
        .join("output")
        .join("lancedb")
        .to_string_lossy()
        .to_string();
    let store = LanceDbVectorStore::connect(&config.vector_store)
        .await
        .expect("lancedb");
    let schema = config.vector_store.schema_for("entity_description");
    assert!(
        store
            .get_by_id(&schema, "old-id")
            .await
            .expect("get old")
            .is_some()
    );
}

async fn apply_invalid_case(
    root: &std::path::Path,
    config: &mut GraphRagConfig,
    case: InvalidConfigCase,
) {
    match case {
        InvalidConfigCase::InvalidRegex => {
            "[".clone_into(&mut config.input.file_pattern);
        }
        InvalidConfigCase::MissingModel => {
            config.completion_models.remove("default_completion_model");
        }
        InvalidConfigCase::MissingPrompt => {
            config.extract_graph.prompt = Some("prompts/missing.txt".to_owned());
        }
        InvalidConfigCase::MissingInput => {
            tokio::fs::remove_dir_all(root.join("input"))
                .await
                .expect("remove input");
        }
        InvalidConfigCase::UnsupportedProvider => {
            "azure".clone_into(
                &mut config
                    .completion_models
                    .get_mut("default_completion_model")
                    .expect("model")
                    .provider_type,
            );
        }
        InvalidConfigCase::InvalidTokenizer => {
            "definitely-not-an-encoding".clone_into(&mut config.chunking.encoding_model);
        }
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
