use std::{collections::BTreeSet, sync::Arc};

use assert_cmd::Command;
use graphloom::{ALL_EMBEDDINGS, GraphRagConfig};
use graphloom_storage::{ParquetTableProvider, TableProvider};
use graphloom_vectors::{LanceDbVectorStore, VectorStore};
use predicates::prelude::*;
use serde_json::{Value, json};
use serde_yaml::Mapping;
use tempfile::TempDir;
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test]
async fn test_should_run_binary_init_dry_run_and_standard_index_with_openai_stub() {
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
    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "init",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--model",
            "gpt-test",
            "--embedding",
            "embed-test",
        ])
        .assert()
        .success();

    tokio::fs::write(
        tempdir.path().join("input").join("document.txt"),
        "Alice works for Acme. Bob manages Acme. Alice and Bob collaborated on Project Atlas.",
    )
    .await
    .expect("write input");
    tokio::fs::write(
        tempdir.path().join(".env"),
        "GRAPHRAG_API_KEY=super-secret-key\n",
    )
    .await
    .expect("write env");
    patch_settings(tempdir.path(), &server.uri()).await;

    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "index",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Workflows:"))
        .stdout(predicate::str::contains("super-secret-key").not());
    assert!(!tempdir.path().join("output").exists());
    assert!(!tempdir.path().join("cache").exists());
    assert!(!tempdir.path().join("logs").exists());
    assert_eq!(
        server.received_requests().await.expect("requests").len(),
        0,
        "dry-run must not call the OpenAI-compatible server",
    );

    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "index",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Index completed successfully"));

    assert_standard_outputs(tempdir.path()).await;
    assert_log_redaction_and_success(tempdir.path()).await;
    let first_document_ids = table_ids(tempdir.path(), "documents").await;
    let first_entity_count = lancedb_count(tempdir.path(), "entity_description").await;
    assert!(tempdir.path().join("cache").exists());

    tokio::fs::write(
        tempdir.path().join("input").join("document.txt"),
        "Carol founded Beta.",
    )
    .await
    .expect("replace input");
    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "index",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Index completed successfully"));

    let second_document_ids = table_ids(tempdir.path(), "documents").await;
    assert!(
        first_document_ids.is_disjoint(&second_document_ids),
        "full rerun should replace document output instead of appending"
    );
    assert_eq!(
        lancedb_count(tempdir.path(), "entity_description").await,
        first_entity_count,
        "managed LanceDB reset should prevent count doubling"
    );
    assert!(tempdir.path().join("cache").exists());
}

#[tokio::test]
async fn test_should_fail_dry_run_when_api_key_is_placeholder() {
    let tempdir = TempDir::new().expect("tempdir");
    init_project(tempdir.path());
    tokio::fs::write(tempdir.path().join("input").join("document.txt"), "Alice")
        .await
        .expect("input");

    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "index",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--dry-run",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("api_key is required"))
        .stderr(predicate::str::contains("<API_KEY>").not());
    assert!(!tempdir.path().join("output").exists());
    assert!(!tempdir.path().join("cache").exists());
    assert!(!tempdir.path().join("logs").exists());
}

#[tokio::test]
async fn test_should_disable_cache_only_for_current_run() {
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
    init_project(tempdir.path());
    tokio::fs::write(
        tempdir.path().join("input").join("document.txt"),
        "Alice works for Acme.",
    )
    .await
    .expect("input");
    tokio::fs::write(
        tempdir.path().join(".env"),
        "GRAPHRAG_API_KEY=super-secret-key\n",
    )
    .await
    .expect("env");
    patch_settings(tempdir.path(), &server.uri()).await;
    tokio::fs::create_dir(tempdir.path().join("cache"))
        .await
        .expect("cache dir");
    tokio::fs::write(tempdir.path().join("cache").join("sentinel"), "keep")
        .await
        .expect("sentinel");

    run_index(tempdir.path(), &["--no-cache"]);
    let first_no_cache_requests = server.received_requests().await.expect("requests").len();
    assert!(tempdir.path().join("cache").join("sentinel").is_file());

    run_index(tempdir.path(), &["--no-cache"]);
    let second_no_cache_requests = server.received_requests().await.expect("requests").len();
    let no_cache_repeat_requests = second_no_cache_requests.saturating_sub(first_no_cache_requests);
    assert!(
        no_cache_repeat_requests > 0,
        "second no-cache run should still call the model server"
    );
    assert!(tempdir.path().join("cache").join("sentinel").is_file());

    run_index(tempdir.path(), &[]);
    let first_cached_requests = server.received_requests().await.expect("requests").len();
    run_index(tempdir.path(), &[]);
    let second_cached_requests = server.received_requests().await.expect("requests").len();
    let cached_repeat_requests = second_cached_requests.saturating_sub(first_cached_requests);
    assert!(
        cached_repeat_requests < no_cache_repeat_requests,
        "default cache mode should reduce repeat model calls for identical input"
    );
}

#[tokio::test]
async fn test_should_run_graph_extraction_gleaning_end_to_end() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(chat_responder_with_gleaning)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(embedding_responder)
        .mount(&server)
        .await;

    let tempdir = TempDir::new().expect("tempdir");
    init_project(tempdir.path());
    tokio::fs::write(
        tempdir.path().join("input").join("document.txt"),
        "Alice works for Acme. Bob manages Acme.",
    )
    .await
    .expect("input");
    tokio::fs::write(
        tempdir.path().join(".env"),
        "GRAPHRAG_API_KEY=super-secret-key\n",
    )
    .await
    .expect("env");
    patch_settings_with_max_gleanings(tempdir.path(), &server.uri(), 2).await;

    run_index(tempdir.path(), &[]);
    let requests = server.received_requests().await.expect("requests");
    assert!(
        requests.iter().any(request_last_message_contains(
            "MANY entities and relationships"
        )),
        "graph extraction should request a continuation"
    );
    assert!(
        requests
            .iter()
            .any(request_last_message_contains("single letter Y or N")),
        "graph extraction should run the gleaning loop check"
    );
    assert!(
        entity_titles(tempdir.path()).await.contains("BOB"),
        "entity discovered during continuation should be merged into output",
    );
}

#[tokio::test]
async fn test_should_fail_dry_run_when_prompt_is_missing() {
    let tempdir = TempDir::new().expect("tempdir");
    init_project(tempdir.path());
    tokio::fs::write(
        tempdir.path().join(".env"),
        "GRAPHRAG_API_KEY=super-secret-key\n",
    )
    .await
    .expect("env");
    tokio::fs::write(tempdir.path().join("input").join("document.txt"), "Alice")
        .await
        .expect("input");
    tokio::fs::remove_file(tempdir.path().join("prompts").join("extract_graph.txt"))
        .await
        .expect("remove prompt");

    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "index",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--dry-run",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("extract_graph.txt"));
}

#[tokio::test]
async fn test_should_fail_dry_run_when_no_input_matches_pattern() {
    let tempdir = TempDir::new().expect("tempdir");
    init_project(tempdir.path());
    tokio::fs::write(
        tempdir.path().join(".env"),
        "GRAPHRAG_API_KEY=super-secret-key\n",
    )
    .await
    .expect("env");

    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "index",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--dry-run",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no matching input files"));
}

#[tokio::test]
async fn test_should_report_common_preflight_errors_without_resetting_output() {
    for (case, expected) in [
        (CliPreflightCase::InvalidRegex, "input.file_pattern"),
        (
            CliPreflightCase::UnsupportedProvider,
            "unsupported provider azure",
        ),
        (CliPreflightCase::UnsupportedAuth, "unsupported auth method"),
        (
            CliPreflightCase::UnsupportedRetry,
            "unsupported retry strategy constant",
        ),
        (
            CliPreflightCase::UnsupportedInput,
            "unsupported input type csv",
        ),
        (
            CliPreflightCase::UnsupportedInputStorage,
            "unsupported input storage blob",
        ),
        (
            CliPreflightCase::UnsupportedOutputStorage,
            "unsupported output storage blob",
        ),
        (
            CliPreflightCase::UnsupportedCacheStorage,
            "unsupported cache storage memory",
        ),
        (
            CliPreflightCase::UnsupportedReportingStorage,
            "unsupported reporting storage memory",
        ),
        (
            CliPreflightCase::UnsafeOutputRoot,
            "output directory must not be project root",
        ),
        (
            CliPreflightCase::OutputAncestorOfLogs,
            "output directory must not be an ancestor",
        ),
        (CliPreflightCase::UnknownWorkflow, "not_a_workflow"),
        (CliPreflightCase::MissingClaimsModel, "missing_claim_model"),
        (
            CliPreflightCase::InvalidTokenizer,
            "definitely-not-an-encoding",
        ),
    ] {
        let tempdir = TempDir::new().expect("tempdir");
        init_project(tempdir.path());
        tokio::fs::write(
            tempdir.path().join(".env"),
            "GRAPHRAG_API_KEY=super-secret-key\n",
        )
        .await
        .expect("env");
        tokio::fs::write(tempdir.path().join("input").join("document.txt"), "Alice")
            .await
            .expect("input");
        tokio::fs::create_dir(tempdir.path().join("output"))
            .await
            .expect("output");
        tokio::fs::write(tempdir.path().join("output").join("sentinel.txt"), "keep")
            .await
            .expect("sentinel");
        apply_cli_preflight_case(tempdir.path(), case).await;

        Command::cargo_bin("graphloom")
            .expect("binary")
            .args([
                "index",
                "--root",
                tempdir.path().to_str().expect("utf8 root"),
                "--dry-run",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(expected))
            .stderr(predicate::str::contains("super-secret-key").not());
        assert!(
            tempdir.path().join("output").join("sentinel.txt").is_file(),
            "preflight error {expected} should not clear output",
        );
    }
}

#[tokio::test]
async fn test_should_log_workflow_failure_without_secret() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("server error"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(embedding_responder)
        .mount(&server)
        .await;

    let tempdir = TempDir::new().expect("tempdir");
    init_project(tempdir.path());
    tokio::fs::write(
        tempdir.path().join("input").join("document.txt"),
        "Alice works for Acme.",
    )
    .await
    .expect("input");
    tokio::fs::write(
        tempdir.path().join(".env"),
        "GRAPHRAG_API_KEY=super-secret-key\n",
    )
    .await
    .expect("env");
    patch_settings(tempdir.path(), &server.uri()).await;

    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "index",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("super-secret-key").not());

    let log = tokio::fs::read_to_string(tempdir.path().join("logs").join("indexing-engine.log"))
        .await
        .expect("log");
    assert!(log.contains("workflow error") || log.contains("event=\"failed\""));
    assert!(!log.contains("super-secret-key"));
}

#[derive(Debug, Clone, Copy)]
enum CliPreflightCase {
    InvalidRegex,
    UnsupportedProvider,
    UnsupportedAuth,
    UnsupportedRetry,
    UnsupportedInput,
    UnsupportedInputStorage,
    UnsupportedOutputStorage,
    UnsupportedCacheStorage,
    UnsupportedReportingStorage,
    UnsafeOutputRoot,
    OutputAncestorOfLogs,
    UnknownWorkflow,
    MissingClaimsModel,
    InvalidTokenizer,
}

async fn assert_standard_outputs(root: &std::path::Path) {
    for table in [
        "documents",
        "text_units",
        "entities",
        "relationships",
        "communities",
        "community_reports",
    ] {
        assert!(
            root.join("output")
                .join(format!("{table}.parquet"))
                .is_file(),
            "{table} parquet should exist",
        );
    }
    assert!(!root.join("output").join("covariates.parquet").exists());
    assert!(root.join("cache").exists());
    assert!(root.join("logs").join("indexing-engine.log").is_file());

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

    let settings = tokio::fs::read_to_string(root.join("settings.yaml"))
        .await
        .expect("settings");
    let config: GraphRagConfig = serde_yaml::from_str(&settings).expect("config");
    let mut config = config;
    config.vector_store.db_uri = root
        .join("output")
        .join("lancedb")
        .to_string_lossy()
        .to_string();
    let store = Arc::new(
        LanceDbVectorStore::connect(&config.vector_store)
            .await
            .expect("lancedb"),
    );
    for embedding in ALL_EMBEDDINGS {
        let schema = config.vector_store.schema_for(embedding);
        assert!(store.count(&schema).await.expect("count") > 0);
    }
}

async fn assert_log_redaction_and_success(root: &std::path::Path) {
    let log = tokio::fs::read_to_string(root.join("logs").join("indexing-engine.log"))
        .await
        .expect("log");
    assert!(log.contains("index run started"));
    assert!(log.contains("index completed"));
    assert!(!log.contains("super-secret-key"));
}

async fn table_ids(root: &std::path::Path, table: &str) -> BTreeSet<String> {
    let provider = ParquetTableProvider::new(root.join("output")).expect("provider");
    let dataframe = provider.read_dataframe(table).await.expect("read table");
    dataframe
        .column("id")
        .expect("id column")
        .str()
        .expect("id strings")
        .iter()
        .flatten()
        .map(ToOwned::to_owned)
        .collect()
}

async fn entity_titles(root: &std::path::Path) -> BTreeSet<String> {
    let provider = ParquetTableProvider::new(root.join("output")).expect("provider");
    let dataframe = provider
        .read_dataframe("entities")
        .await
        .expect("read entities");
    dataframe
        .column("title")
        .expect("title column")
        .str()
        .expect("title strings")
        .iter()
        .flatten()
        .map(ToOwned::to_owned)
        .collect()
}

async fn lancedb_count(root: &std::path::Path, embedding_name: &str) -> usize {
    let settings = tokio::fs::read_to_string(root.join("settings.yaml"))
        .await
        .expect("settings");
    let mut config: GraphRagConfig = serde_yaml::from_str(&settings).expect("config");
    config.vector_store.db_uri = root
        .join("output")
        .join("lancedb")
        .to_string_lossy()
        .to_string();
    let store = LanceDbVectorStore::connect(&config.vector_store)
        .await
        .expect("lancedb");
    let schema = config.vector_store.schema_for(embedding_name);
    store.count(&schema).await.expect("count")
}

async fn patch_settings(root: &std::path::Path, server_uri: &str) {
    patch_settings_with_max_gleanings(root, server_uri, 0).await;
}

async fn patch_settings_with_max_gleanings(
    root: &std::path::Path,
    server_uri: &str,
    max_gleanings: usize,
) {
    let path = root.join("settings.yaml");
    let settings = tokio::fs::read_to_string(&path).await.expect("settings");
    let settings = settings
        .replace(
            "api_key: ${GRAPHRAG_API_KEY}",
            &format!("api_key: ${{GRAPHRAG_API_KEY}}\n    api_base: {server_uri}/v1"),
        )
        .replace("vector_size: 3072", "vector_size: 4")
        .replace(
            "max_gleanings: 1",
            &format!("max_gleanings: {max_gleanings}"),
        );
    tokio::fs::write(path, settings)
        .await
        .expect("patch settings");
}

async fn apply_cli_preflight_case(root: &std::path::Path, case: CliPreflightCase) {
    let path = root.join("settings.yaml");
    let settings = tokio::fs::read_to_string(&path).await.expect("settings");
    let mut value: serde_yaml::Value = serde_yaml::from_str(&settings).expect("settings yaml");
    for (yaml_path, replacement) in cli_preflight_mutations(case) {
        set_yaml(&mut value, &yaml_path, replacement);
    }
    tokio::fs::write(
        path,
        serde_yaml::to_string(&value).expect("serialize settings"),
    )
    .await
    .expect("write settings");
}

fn cli_preflight_mutations(case: CliPreflightCase) -> Vec<(Vec<&'static str>, serde_yaml::Value)> {
    let string = |value: &str| serde_yaml::Value::String(value.to_owned());
    match case {
        CliPreflightCase::InvalidRegex => vec![(vec!["input", "file_pattern"], string("["))],
        CliPreflightCase::UnsupportedProvider => vec![(
            vec![
                "completion_models",
                "default_completion_model",
                "model_provider",
            ],
            string("azure"),
        )],
        CliPreflightCase::UnsupportedAuth => vec![(
            vec![
                "completion_models",
                "default_completion_model",
                "auth_method",
            ],
            string("azure_managed_identity"),
        )],
        CliPreflightCase::UnsupportedRetry => vec![(
            vec![
                "completion_models",
                "default_completion_model",
                "retry",
                "type",
            ],
            string("constant"),
        )],
        CliPreflightCase::UnsupportedInput => vec![(vec!["input", "type"], string("csv"))],
        CliPreflightCase::UnsupportedInputStorage => {
            vec![(vec!["input_storage", "type"], string("blob"))]
        }
        CliPreflightCase::UnsupportedOutputStorage => {
            vec![(vec!["output_storage", "type"], string("blob"))]
        }
        CliPreflightCase::UnsupportedCacheStorage => {
            vec![(vec!["cache", "storage", "type"], string("memory"))]
        }
        CliPreflightCase::UnsupportedReportingStorage => {
            vec![(vec!["reporting", "type"], string("memory"))]
        }
        CliPreflightCase::UnsafeOutputRoot => {
            vec![(vec!["output_storage", "base_dir"], string("."))]
        }
        CliPreflightCase::OutputAncestorOfLogs => vec![
            (vec!["output_storage", "base_dir"], string("logs")),
            (vec!["reporting", "base_dir"], string("logs/index")),
        ],
        CliPreflightCase::UnknownWorkflow => vec![(
            vec!["workflows"],
            serde_yaml::Value::Sequence(vec![string("not_a_workflow")]),
        )],
        CliPreflightCase::MissingClaimsModel => vec![
            (
                vec!["extract_claims", "enabled"],
                serde_yaml::Value::Bool(true),
            ),
            (
                vec!["extract_claims", "completion_model_id"],
                string("missing_claim_model"),
            ),
        ],
        CliPreflightCase::InvalidTokenizer => {
            vec![(
                vec!["chunking", "encoding_model"],
                string("definitely-not-an-encoding"),
            )]
        }
    }
}

fn set_yaml(value: &mut serde_yaml::Value, path: &[&str], replacement: serde_yaml::Value) {
    let mut current = value;
    for segment in &path[..path.len().saturating_sub(1)] {
        current = current
            .as_mapping_mut()
            .expect("mapping")
            .entry(serde_yaml::Value::String((*segment).to_owned()))
            .or_insert_with(|| serde_yaml::Value::Mapping(Mapping::default()));
    }
    let leaf = path.last().expect("leaf");
    current
        .as_mapping_mut()
        .expect("mapping")
        .insert(serde_yaml::Value::String((*leaf).to_owned()), replacement);
}

fn init_project(root: &std::path::Path) {
    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "init",
            "--root",
            root.to_str().expect("utf8 root"),
            "--model",
            "gpt-test",
            "--embedding",
            "embed-test",
        ])
        .assert()
        .success();
}

fn run_index(root: &std::path::Path, extra_args: &[&str]) {
    let mut command = Command::cargo_bin("graphloom").expect("binary");
    command.args(["index", "--root", root.to_str().expect("utf8 root")]);
    command.args(extra_args);
    command
        .assert()
        .success()
        .stdout(predicate::str::contains("Index completed successfully"));
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
    } else {
        chat_content_for_non_extraction(last)
    };
    chat_response(&content)
}

fn chat_content_for_non_extraction(last: &str) -> String {
    if last.contains("Return output as a well-formed JSON-formatted string") {
        json!({
            "title": "Alice, Bob, and Acme",
            "summary": "Alice, Bob, and Acme form a connected work community.",
            "rating": 5.0,
            "rating_explanation": "The community is small but coherent.",
            "findings": [
                {
                    "summary": "Collaboration",
                    "explanation": "Alice and Bob collaborate through Acme [Data: Entities (0, 1); Relationships (0)]."
                }
            ]
        })
        .to_string()
    } else {
        "A concise summary of the provided descriptions.".to_owned()
    }
}

fn chat_response(content: &str) -> ResponseTemplate {
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

fn chat_responder_with_gleaning(request: &Request) -> ResponseTemplate {
    let body = request.body_json::<Value>().expect("chat request json");
    let messages = body["messages"].as_array().expect("messages");
    let last = messages
        .last()
        .and_then(|message| message["content"].as_str())
        .unwrap_or_default();
    let content = if last.contains("MANY entities and relationships") {
        "(\"entity\"<|>BOB<|>person<|>Bob manages Acme)##(\"relationship\"<|>BOB<|>ACME<|>Bob \
         manages Acme<|>5)##<|COMPLETE|>"
            .to_owned()
    } else if last.contains("single letter Y or N") {
        "N".to_owned()
    } else if last.contains("Given a text document") {
        "(\"entity\"<|>ALICE<|>person<|>Alice works for \
         Acme)##(\"entity\"<|>ACME<|>organization<|>Acme employs \
         Alice)##(\"relationship\"<|>ALICE<|>ACME<|>Alice works for Acme<|>5)##"
            .to_owned()
    } else {
        chat_content_for_non_extraction(last)
    };
    chat_response(&content)
}

fn request_last_message_contains(needle: &str) -> impl FnMut(&wiremock::Request) -> bool + '_ {
    move |request| {
        request
            .body_json::<Value>()
            .ok()
            .and_then(|body| {
                body["messages"]
                    .as_array()
                    .and_then(|messages| messages.last())
                    .and_then(|message| message["content"].as_str().map(str::to_owned))
            })
            .is_some_and(|content| content.contains(needle))
    }
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
