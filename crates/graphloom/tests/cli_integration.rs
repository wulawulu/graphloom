use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, SystemTime},
};

use assert_cmd::Command;
use graphloom::{
    ALL_EMBEDDINGS, COMMUNITY_FULL_CONTENT_EMBEDDING, ENTITY_DESCRIPTION_EMBEDDING, GraphRagConfig,
    TEXT_UNIT_TEXT_EMBEDDING,
};
use graphloom_llm::{
    CachedModelResult, ChatMessage, CompletionRequest, CompletionResponse, EmbeddingRequest,
    EmbeddingResponse, completion_request_cache_key, embedding_request_cache_key,
};
use graphloom_storage::{FileStorage, ParquetTableProvider, Storage, TableProvider};
use graphloom_vectors::{LanceDbVectorStore, VectorStore};
use polars_core::prelude::{AnyValue, DataFrame, DataType, NamedFrom, PlSmallStr, Series};
use predicates::prelude::*;
use serde_json::{Value, json};
use serde_yaml::Mapping;
use tempfile::TempDir;
use tokio::io::AsyncReadExt;
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

static REPORT_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[test]
fn test_should_match_complete_query_help_snapshot() {
    let output = Command::cargo_bin("graphloom")
        .expect("binary")
        .args(["query", "--help"])
        .output()
        .expect("query help");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let actual = normalize_help_text(&output.stdout);
    let expected = include_str!("fixtures/cli/query_help.txt").replace("\r\n", "\n");
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn test_should_return_exact_query_cli_exit_codes() {
    let parse_root = TempDir::new().expect("parse root");
    for arguments in [
        vec!["query"],
        vec!["query", "--method", "invalid", "question"],
        vec!["query", "--unknown", "question"],
        vec!["query", "--root"],
    ] {
        let output = Command::cargo_bin("graphloom")
            .expect("binary")
            .args(arguments)
            .current_dir(parse_root.path())
            .output()
            .expect("Clap error");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let stderr = normalize_cli_text(&output.stderr);
        assert!(
            stderr.contains("error:") && stderr.contains("--help"),
            "stderr was {}",
            stderr
        );
    }
    assert!(!parse_root.path().join("logs").exists());

    let missing_settings = Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "query",
            "--root",
            parse_root.path().to_str().expect("UTF-8 root"),
            "question",
        ])
        .output()
        .expect("missing settings");
    assert_eq!(missing_settings.status.code(), Some(1));
    assert!(missing_settings.stdout.is_empty());
    assert!(normalize_cli_text(&missing_settings.stderr).contains("no settings"));
    assert!(!parse_root.path().join("logs").exists());

    let invalid_config_root = TempDir::new().expect("invalid config root");
    init_project(invalid_config_root.path());
    tokio::fs::write(
        invalid_config_root.path().join("settings.yaml"),
        "completion_models: [invalid",
    )
    .await
    .expect("invalid settings");
    let invalid_config = Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "query",
            "--root",
            invalid_config_root.path().to_str().expect("UTF-8 root"),
            "question",
        ])
        .output()
        .expect("invalid configuration");
    assert_eq!(invalid_config.status.code(), Some(1));
    assert!(invalid_config.stdout.is_empty());
    assert!(normalize_cli_text(&invalid_config.stderr).contains("failed to parse"));

    let runtime_root = TempDir::new().expect("runtime root");
    init_project(runtime_root.path());
    let output = Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "query",
            "--root",
            runtime_root.path().to_str().expect("UTF-8 root"),
            "--method",
            "basic",
            "question",
        ])
        .output()
        .expect("runtime error");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = normalize_cli_text(&output.stderr);
    assert!(stderr.contains("requires table text_units"));
    assert!(!stderr.contains("Usage:"));
    let log = tokio::fs::read_to_string(runtime_root.path().join("logs").join("query.log"))
        .await
        .expect("query failure log");
    assert!(log.contains("query run started"));
    assert!(log.contains("query run failed"));
    assert!(log.contains("method=basic"));
    assert!(log.contains("streaming=false"));
    assert!(log.contains("error_category=\"query\""));
}

#[tokio::test]
async fn test_should_flush_first_cli_stream_chunk_before_provider_finishes() {
    let (server_uri, first_chunk_sent, release_completion, server_thread) = delayed_query_server();
    let tempdir = TempDir::new().expect("tempdir");
    init_project(tempdir.path());
    tokio::fs::write(
        tempdir.path().join(".env"),
        "GRAPHRAG_API_KEY=query-secret\n",
    )
    .await
    .expect("env");
    patch_settings(tempdir.path(), &server_uri).await;
    write_minimal_query_index(tempdir.path()).await;

    let mut child = tokio::process::Command::new(assert_cmd::cargo::cargo_bin!("graphloom"))
        .args([
            "query",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--method",
            "basic",
            "--streaming",
            "facts",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn graphloom query");
    let mut stdout = child.stdout.take().expect("child stdout");
    let mut stderr = child.stderr.take().expect("child stderr");
    tokio::task::spawn_blocking(move || first_chunk_sent.recv_timeout(Duration::from_secs(10)))
        .await
        .expect("provider signal waiter")
        .expect("provider should send its first chunk");
    let mut first_stdout = vec![0_u8; "Basic ".len()];
    tokio::time::timeout(Duration::from_secs(5), stdout.read_exact(&mut first_stdout))
        .await
        .expect("CLI should flush before completion ends")
        .expect("read first stdout chunk");
    assert_eq!(first_stdout, b"Basic ");
    release_completion
        .send(())
        .expect("release provider completion");

    let mut stdout_rest = Vec::new();
    let mut stderr_bytes = Vec::new();
    let (stdout_result, stderr_result, status) = tokio::join!(
        stdout.read_to_end(&mut stdout_rest),
        stderr.read_to_end(&mut stderr_bytes),
        child.wait(),
    );
    stdout_result.expect("remaining stdout");
    stderr_result.expect("stderr");
    let status = status.expect("child status");
    server_thread.join().expect("server thread");
    assert!(status.success());
    first_stdout.extend(stdout_rest);
    assert_eq!(first_stdout, b"Basic answer.\n");
    assert!(stderr_bytes.is_empty());
}

#[tokio::test]
async fn test_should_preserve_partial_stream_and_fail_without_terminal_newline() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(partial_failure_stream_response())
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(embedding_responder)
        .mount(&server)
        .await;
    let project = TempDir::new().expect("project");
    init_project(project.path());
    tokio::fs::write(
        project.path().join(".env"),
        "GRAPHRAG_API_KEY=partial-stream-secret\n",
    )
    .await
    .expect("env");
    patch_settings(project.path(), &server.uri()).await;
    write_minimal_query_index(project.path()).await;

    let output = Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "query",
            "--root",
            project.path().to_str().expect("UTF-8 root"),
            "--method",
            "basic",
            "--streaming",
            "PARTIAL_STREAM_QUERY_SENTINEL",
        ])
        .output()
        .expect("partial stream Query");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(normalize_cli_text(&output.stdout), "Basic ");
    let stderr = normalize_cli_text(&output.stderr);
    assert!(stderr.contains("query completion model"));
    assert!(!stderr.contains("Completed"));
    assert!(!stderr.contains("partial-stream-secret"));
    assert!(!stderr.contains("Authorization"));
    let log = tokio::fs::read_to_string(project.path().join("logs").join("query.log"))
        .await
        .expect("failure log");
    assert!(log.contains("query run failed"));
    assert!(!log.contains("query run completed"));
    assert!(!log.contains("partial-stream-secret"));
    assert!(!log.contains("PARTIAL_STREAM_QUERY_SENTINEL"));
}

#[cfg(unix)]
#[tokio::test]
async fn test_should_return_one_for_query_stdout_io_failure() {
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
    let project = TempDir::new().expect("project");
    init_project(project.path());
    tokio::fs::write(project.path().join(".env"), "GRAPHRAG_API_KEY=io-secret\n")
        .await
        .expect("env");
    patch_settings(project.path(), &server.uri()).await;
    write_minimal_query_index(project.path()).await;
    let full = tokio::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .await
        .expect("/dev/full")
        .into_std()
        .await;
    let mut child = tokio::process::Command::new(assert_cmd::cargo::cargo_bin!("graphloom"))
        .args([
            "query",
            "--root",
            project.path().to_str().expect("UTF-8 root"),
            "--method",
            "basic",
            "question",
        ])
        .stdout(Stdio::from(full))
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn stdout failure Query");
    let mut stderr = child.stderr.take().expect("stderr");
    let mut stderr_bytes = Vec::new();
    let (read_result, status) = tokio::join!(stderr.read_to_end(&mut stderr_bytes), child.wait());
    read_result.expect("read stderr");
    let status = status.expect("Query status");

    assert_eq!(status.code(), Some(1));
    let stderr = normalize_cli_text(&stderr_bytes);
    assert!(
        stderr.contains("Query stdout")
            || stderr.contains("Query response")
            || stderr.contains("Query terminal newline"),
        "unexpected stderr: {stderr}",
    );
    assert!(!stderr.contains("io-secret"));
}

fn normalize_cli_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes).replace("\r\n", "\n");
    let mut normalized = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\u{1b}' {
            normalized.push(character);
            continue;
        }
        if characters.next() != Some('[') {
            continue;
        }
        for suffix in characters.by_ref() {
            if suffix.is_ascii_alphabetic() {
                break;
            }
        }
    }
    normalized
}

fn normalize_help_text(bytes: &[u8]) -> String {
    let normalized = normalize_cli_text(bytes);
    let had_terminal_newline = normalized.ends_with('\n');
    let mut normalized = normalized
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    if had_terminal_newline {
        normalized.push('\n');
    }
    normalized
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "the Basic Query CLI scenario keeps stdout, read-only, and data-override assertions \
              together"
)]
async fn test_should_run_basic_query_cli_stream_and_data_override_read_only() {
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
    run_minimal_standard_index(tempdir.path(), &server.uri()).await;
    let parquet_before = parquet_artifact_snapshot(tempdir.path()).await;
    let vector_ids_before = managed_vector_ids(tempdir.path()).await;
    let cache = FileStorage::existing(tempdir.path().join("cache")).expect("cache");
    let cache_before = cache.list("").await.expect("cache before");

    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "query",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--method",
            "basic",
            "What are the main facts?",
        ])
        .assert()
        .success()
        .stdout("Basic answer.\n")
        .stderr(predicate::str::is_empty());
    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "query",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--method",
            "basic",
            "--streaming",
            "What are the main facts?",
        ])
        .assert()
        .success()
        .stdout("Basic answer.\n")
        .stderr(predicate::str::is_empty());
    for streaming in [false, true] {
        let mut arguments = vec![
            "query",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--method",
            "global",
            "--dynamic-community-selection",
        ];
        if streaming {
            arguments.push("--streaming");
        }
        arguments.push("What are the main themes?");
        Command::cargo_bin("graphloom")
            .expect("binary")
            .args(arguments)
            .assert()
            .success()
            .stdout("Global answer.\n")
            .stderr(predicate::str::is_empty());
    }

    assert_eq!(
        parquet_artifact_snapshot(tempdir.path()).await,
        parquet_before
    );
    assert_eq!(managed_vector_ids(tempdir.path()).await, vector_ids_before);
    assert_eq!(cache.list("").await.expect("cache after"), cache_before);
    assert!(tempdir.path().join("logs").join("query.log").is_file());

    let provider = ParquetTableProvider::new(tempdir.path().join("output")).expect("provider");
    let mut overridden = provider
        .read_dataframe("text_units")
        .await
        .expect("text units");
    overridden
        .replace(
            "text",
            Series::new(
                "text".into(),
                vec!["DATA_OVERRIDE_MARKER"; overridden.height()],
            )
            .into(),
        )
        .expect("replace text");
    let override_root = tempdir.path().join("alternate_tables");
    let override_provider = ParquetTableProvider::new(&override_root).expect("override provider");
    override_provider
        .write_dataframe("text_units", overridden)
        .await
        .expect("override text units");
    let override_table = override_root.join("text_units.parquet");
    let override_before = (
        tokio::fs::read(&override_table)
            .await
            .expect("override bytes"),
        tokio::fs::metadata(&override_table)
            .await
            .expect("override metadata")
            .modified()
            .expect("override mtime"),
    );
    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "query",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--method",
            "basic",
            "--data",
            override_root.to_str().expect("utf8 data root"),
            "facts",
        ])
        .assert()
        .success()
        .stdout("Basic answer.\n");
    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "query",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--method",
            "basic",
            "--data",
            "alternate_tables",
            "facts",
        ])
        .assert()
        .success()
        .stdout("Basic answer.\n")
        .stderr(predicate::str::is_empty());
    let missing_override = tempdir.path().join("missing_alternate_tables");
    let missing_output = Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "query",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--method",
            "basic",
            "--data",
            missing_override.to_str().expect("UTF-8 missing data root"),
            "facts",
        ])
        .output()
        .expect("missing data Query");
    assert_eq!(missing_output.status.code(), Some(1));
    assert!(missing_output.stdout.is_empty());
    assert!(normalize_cli_text(&missing_output.stderr).contains("requires table text_units"));
    assert!(!missing_override.exists());
    assert!(!override_root.join("lancedb").exists());
    assert_eq!(
        tokio::fs::read(&override_table)
            .await
            .expect("override bytes after"),
        override_before.0
    );
    assert_eq!(
        tokio::fs::metadata(&override_table)
            .await
            .expect("override metadata after")
            .modified()
            .expect("override mtime after"),
        override_before.1
    );
    let mut override_entries = tokio::fs::read_dir(&override_root)
        .await
        .expect("override directory");
    let mut override_entry_count = 0_usize;
    while override_entries
        .next_entry()
        .await
        .expect("override directory entry")
        .is_some()
    {
        override_entry_count = override_entry_count.saturating_add(1);
    }
    assert_eq!(override_entry_count, 1);
    let requests = server.received_requests().await.expect("requests");
    assert!(requests.iter().any(|request| {
        request
            .body_json::<Value>()
            .ok()
            .and_then(|body| body["messages"].as_array().cloned())
            .is_some_and(|messages| {
                messages.iter().any(|message| {
                    message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("DATA_OVERRIDE_MARKER"))
                })
            })
    }));
    let query_log = tokio::fs::read_to_string(tempdir.path().join("logs").join("query.log"))
        .await
        .expect("query log");
    assert!(!query_log.contains("DATA_OVERRIDE_MARKER"));
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "the CLI dispatch matrix keeps cross-method output, provider, logging, and read-only \
              assertions in one shared-index scenario"
)]
async fn test_should_run_complete_query_cli_dispatch_matrix_and_log_safely() {
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
    let project = TempDir::new().expect("project");
    run_minimal_standard_index(project.path(), &server.uri()).await;
    let original_settings = tokio::fs::read_to_string(project.path().join("settings.yaml"))
        .await
        .expect("settings");
    let vector_fixture = TempDir::new().expect("method vector fixture");
    let basic_vector_db = vector_fixture.path().join("basic");
    let local_vector_db = vector_fixture.path().join("local");
    let drift_vector_db = vector_fixture.path().join("drift");
    let global_vector_db = vector_fixture.path().join("must-not-open");
    let basic_vector_ids = copy_vector_indices(
        project.path(),
        &basic_vector_db,
        &[TEXT_UNIT_TEXT_EMBEDDING],
    )
    .await;
    let local_vector_ids = copy_vector_indices(
        project.path(),
        &local_vector_db,
        &[ENTITY_DESCRIPTION_EMBEDDING],
    )
    .await;
    let drift_vector_ids = copy_vector_indices(
        project.path(),
        &drift_vector_db,
        &[
            ENTITY_DESCRIPTION_EMBEDDING,
            COMMUNITY_FULL_CONTENT_EMBEDDING,
        ],
    )
    .await;
    let parquet_before = parquet_artifact_snapshot(project.path()).await;
    let vector_ids_before = managed_vector_ids(project.path()).await;
    let cache = FileStorage::existing(project.path().join("cache")).expect("cache");
    let cache_before = cache.list("").await.expect("cache before");
    let indexing_log_path = project.path().join("logs").join("indexing-engine.log");
    let indexing_log_before = tokio::fs::read(&indexing_log_path)
        .await
        .expect("indexing log");
    let indexing_log_mtime = tokio::fs::metadata(&indexing_log_path)
        .await
        .expect("indexing log metadata")
        .modified()
        .expect("indexing log mtime");
    assert!(!project.path().join("logs").join("index.log").exists());

    let query_sentinel = "QUERY_SENTINEL_SHOULD_NOT_REACH_LOGS";
    for (method_name, answer, prompt_marker) in [
        ("basic", "Basic answer.\n", "primary context"),
        (
            "local",
            "Local answer.\n",
            "incorporating any relevant general knowledge",
        ),
        (
            "global",
            "Global answer.\n",
            "dataset by synthesizing perspectives",
        ),
        ("drift", "DRIFT answer.\n", "data in the reports provided"),
    ] {
        for streaming in [false, true] {
            let method_vector_db = match method_name {
                "basic" => &basic_vector_db,
                "local" => &local_vector_db,
                "drift" => &drift_vector_db,
                "global" => &global_vector_db,
                other => panic!("unexpected Query method {other}"),
            };
            set_query_vector_db(project.path(), &original_settings, method_vector_db).await;
            let allowed_tables = match method_name {
                "basic" => &["text_units"][..],
                "global" => &["entities", "communities", "community_reports"][..],
                "local" | "drift" => &[
                    "entities",
                    "communities",
                    "community_reports",
                    "text_units",
                    "relationships",
                    "covariates",
                ][..],
                other => panic!("unexpected Query method {other}"),
            };
            let hidden_tables = hide_unneeded_parquet(project.path(), allowed_tables).await;
            let request_offset = server
                .received_requests()
                .await
                .expect("requests before Query")
                .len();
            let mut arguments = vec![
                "query",
                "--root",
                project.path().to_str().expect("UTF-8 root"),
                "--method",
                method_name,
            ];
            if streaming {
                arguments.push("--streaming");
            }
            arguments.push(query_sentinel);
            let output = Command::cargo_bin("graphloom")
                .expect("binary")
                .args(arguments)
                .output()
                .expect("Query command");
            restore_hidden_parquet(&hidden_tables).await;
            assert_eq!(
                output.status.code(),
                Some(0),
                "{method_name} streaming={streaming}: {}",
                normalize_cli_text(&output.stderr)
            );
            assert_eq!(normalize_cli_text(&output.stdout), answer);
            assert!(output.stderr.is_empty());

            let requests = server.received_requests().await.expect("Query requests");
            let query_requests = requests.get(request_offset..).expect("new Query requests");
            assert!(!query_requests.is_empty());
            assert!(
                query_requests
                    .iter()
                    .filter_map(|request| request.body_json::<Value>().ok())
                    .any(|body| {
                        body["messages"]
                            .as_array()
                            .and_then(|messages| messages.first())
                            .and_then(|message| message["content"].as_str())
                            .is_some_and(|content| content.contains(prompt_marker))
                    }),
                "{method_name} did not send its method prompt"
            );
            if method_name == "global" {
                assert!(!query_requests.iter().any(is_dynamic_rating_request));
                assert!(
                    !query_requests
                        .iter()
                        .any(|request| request.url.path().contains("embeddings"))
                );
            } else {
                assert!(
                    query_requests
                        .iter()
                        .any(|request| request.url.path().contains("embeddings"))
                );
            }
        }
    }

    for streaming in [false, true] {
        set_query_vector_db(project.path(), &original_settings, &global_vector_db).await;
        let hidden_tables = hide_unneeded_parquet(
            project.path(),
            &["entities", "communities", "community_reports"],
        )
        .await;
        let dynamic_offset = server
            .received_requests()
            .await
            .expect("requests before Dynamic Global")
            .len();
        let mut arguments = vec![
            "query",
            "--root",
            project.path().to_str().expect("UTF-8 root"),
            "--method",
            "global",
            "--dynamic-community-selection",
        ];
        if streaming {
            arguments.push("--streaming");
        }
        arguments.push(query_sentinel);
        let dynamic_output = Command::cargo_bin("graphloom")
            .expect("binary")
            .args(arguments)
            .output()
            .expect("Dynamic Global Query");
        restore_hidden_parquet(&hidden_tables).await;
        assert_eq!(dynamic_output.status.code(), Some(0));
        assert_eq!(
            normalize_cli_text(&dynamic_output.stdout),
            "Global answer.\n"
        );
        assert!(dynamic_output.stderr.is_empty());
        let requests = server.received_requests().await.expect("Dynamic requests");
        let dynamic_requests = requests
            .get(dynamic_offset..)
            .expect("new Dynamic Global requests");
        assert!(dynamic_requests.iter().any(is_dynamic_rating_request));
        assert!(
            !dynamic_requests
                .iter()
                .any(|request| request.url.path().contains("embeddings"))
        );
    }

    set_query_vector_db(project.path(), &original_settings, &basic_vector_db).await;
    let verbose_output = Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "query",
            "--root",
            project.path().to_str().expect("UTF-8 root"),
            "--method",
            "basic",
            "--verbose",
            query_sentinel,
        ])
        .output()
        .expect("verbose Basic Query");
    assert_eq!(verbose_output.status.code(), Some(0));
    assert_eq!(
        normalize_cli_text(&verbose_output.stdout),
        "Basic answer.\n"
    );
    let verbose_stderr = normalize_cli_text(&verbose_output.stderr);
    assert!(!verbose_stderr.is_empty());
    assert!(!verbose_stderr.contains("Basic answer."));
    assert!(!verbose_stderr.contains("test-key"));
    assert!(!verbose_stderr.contains("Authorization"));
    assert!(!verbose_stderr.contains(query_sentinel));

    tokio::fs::write(project.path().join("settings.yaml"), &original_settings)
        .await
        .expect("restore settings");
    assert_eq!(
        parquet_artifact_snapshot(project.path()).await,
        parquet_before
    );
    assert_eq!(managed_vector_ids(project.path()).await, vector_ids_before);
    assert_vector_indices_unchanged(&original_settings, &basic_vector_db, &basic_vector_ids).await;
    assert_vector_indices_unchanged(&original_settings, &local_vector_db, &local_vector_ids).await;
    assert_vector_indices_unchanged(&original_settings, &drift_vector_db, &drift_vector_ids).await;
    assert!(!global_vector_db.exists());
    assert_eq!(cache.list("").await.expect("cache after"), cache_before);
    assert_eq!(
        tokio::fs::read(&indexing_log_path)
            .await
            .expect("indexing log after Query"),
        indexing_log_before
    );
    assert_eq!(
        tokio::fs::metadata(&indexing_log_path)
            .await
            .expect("indexing log metadata after Query")
            .modified()
            .expect("indexing log mtime after Query"),
        indexing_log_mtime
    );
    assert!(!project.path().join("logs").join("index.log").exists());

    let query_log = tokio::fs::read_to_string(project.path().join("logs").join("query.log"))
        .await
        .expect("query log");
    assert_eq!(query_log.matches("query run started").count(), 11);
    assert_eq!(query_log.matches("query run completed").count(), 11);
    for method_name in ["basic", "local", "global", "drift"] {
        assert!(query_log.contains(&format!("method={method_name} streaming=false")));
        assert!(query_log.contains(&format!("method={method_name} streaming=true")));
    }
    for usage_field in ["elapsed_ms", "llm_calls", "prompt_tokens", "output_tokens"] {
        assert!(query_log.contains(usage_field));
    }
    assert!(!query_log.contains("test-key"));
    assert!(!query_log.contains("Authorization"));
    assert!(!query_log.contains(query_sentinel));
    assert!(!query_log.contains("Alice works for Acme"));
}

fn is_dynamic_rating_request(request: &Request) -> bool {
    request
        .body_json::<Value>()
        .ok()
        .and_then(|body| {
            body["messages"]
                .as_array()
                .and_then(|messages| messages.first())
                .and_then(|message| message["content"].as_str().map(str::to_owned))
        })
        .is_some_and(|content| {
            content.contains("deciding whether the provided information is useful")
        })
}

#[tokio::test]
async fn test_should_fail_cli_query_with_typed_resource_and_method_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(chat_responder_with_drift_parse_failure)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(embedding_responder)
        .mount(&server)
        .await;
    let tempdir = TempDir::new().expect("tempdir");
    run_minimal_standard_index(tempdir.path(), &server.uri()).await;

    let drift_output = Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "query",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--method",
            "drift",
            "facts",
        ])
        .output()
        .expect("DRIFT parse error");
    assert_eq!(drift_output.status.code(), Some(1));
    assert!(drift_output.stdout.is_empty());
    assert!(normalize_cli_text(&drift_output.stderr).contains("query parse failed for drift"));

    let table_path = tempdir.path().join("output").join("text_units.parquet");
    let table_bytes = tokio::fs::read(&table_path).await.expect("table bytes");
    tokio::fs::remove_file(&table_path)
        .await
        .expect("remove text units");
    let table_output = Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "query",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--method",
            "basic",
            "facts",
        ])
        .output()
        .expect("missing table error");
    assert_eq!(table_output.status.code(), Some(1));
    assert!(table_output.stdout.is_empty());
    assert!(normalize_cli_text(&table_output.stderr).contains("requires table text_units"));
    tokio::fs::write(&table_path, table_bytes)
        .await
        .expect("restore text units");

    let settings_path = tempdir.path().join("settings.yaml");
    let original_settings = tokio::fs::read_to_string(&settings_path)
        .await
        .expect("settings");
    let settings = original_settings.replace(
        "  vector_size: 4",
        concat!(
            "  vector_size: 4\n",
            "  index_schema:\n",
            "    text_unit_text:\n",
            "      index_name: missing_text_unit_text\n",
            "      vector_size: 4"
        ),
    );
    tokio::fs::write(&settings_path, settings)
        .await
        .expect("settings with missing index");
    let vector_output = Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "query",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--method",
            "basic",
            "facts",
        ])
        .output()
        .expect("missing vector error");
    assert_eq!(vector_output.status.code(), Some(1));
    assert!(vector_output.stdout.is_empty());
    assert!(
        normalize_cli_text(&vector_output.stderr)
            .contains("requires vector index missing_text_unit_text")
    );

    tokio::fs::write(&settings_path, &original_settings)
        .await
        .expect("restore settings");
    let prompt_path = tempdir
        .path()
        .join("prompts")
        .join("basic_search_system_prompt.txt");
    let prompt = tokio::fs::read(&prompt_path).await.expect("Basic prompt");
    tokio::fs::remove_file(&prompt_path)
        .await
        .expect("remove Basic prompt");
    let prompt_output = Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "query",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--method",
            "basic",
            "facts",
        ])
        .output()
        .expect("missing prompt error");
    assert_eq!(prompt_output.status.code(), Some(1));
    assert!(prompt_output.stdout.is_empty());
    assert!(normalize_cli_text(&prompt_output.stderr).contains("query prompt"));
    tokio::fs::write(&prompt_path, prompt)
        .await
        .expect("restore Basic prompt");

    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"error": {"message": "authentication failed"}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(embedding_responder)
        .mount(&server)
        .await;
    let provider_output = Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "query",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--method",
            "basic",
            "facts",
        ])
        .output()
        .expect("provider authentication error");
    assert_eq!(provider_output.status.code(), Some(1));
    assert!(provider_output.stdout.is_empty());
    let provider_stderr = normalize_cli_text(&provider_output.stderr);
    assert!(provider_stderr.contains("query completion model"));
    assert!(!provider_stderr.contains("test-key"));
    assert!(!provider_stderr.contains("Authorization"));
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "the CLI index end-to-end scenario keeps initialization, dry-run, and rerun \
              assertions together"
)]
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
        .success()
        .stdout(predicate::str::contains(
            "Starting project initialization preflight",
        ))
        .stdout(predicate::str::contains(
            "Completed project file publication",
        ));

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
        .stdout(predicate::str::contains(
            "Starting project configuration load",
        ))
        .stdout(predicate::str::contains(
            "Completed project and model connectivity validation",
        ))
        .stdout(predicate::str::contains("Workflows:"))
        .stdout(predicate::str::contains("<redacted>"))
        .stdout(predicate::str::contains(format!("{}/v1", server.uri())))
        .stdout(predicate::str::contains("super-secret-key").not())
        .stderr(predicate::str::contains("super-secret-key").not());
    assert!(!tempdir.path().join("output").exists());
    assert!(!tempdir.path().join("cache").exists());
    assert!(!tempdir.path().join("logs").exists());
    let validation_requests = server.received_requests().await.expect("requests");
    assert_eq!(validation_requests.len(), 2);
    assert!(
        validation_requests
            .iter()
            .any(request_last_message_contains(
                "This is an LLM connectivity test. Say Hello World",
            ))
    );
    assert!(validation_requests.iter().any(|request| {
        request
            .body_json::<Value>()
            .ok()
            .and_then(|body| body["input"].as_array().cloned())
            .is_some_and(|inputs| {
                inputs
                    .iter()
                    .any(|input| input.as_str() == Some("This is an LLM Embedding Test String"))
            })
    }));

    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "index",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Starting indexing runtime preparation",
        ))
        .stdout(predicate::str::contains(
            "Completed indexing runtime preparation",
        ))
        .stdout(predicate::str::contains("create_base_text_units:"))
        .stderr(predicate::str::contains("index run started").not())
        .stdout(predicate::str::contains("Index completed successfully"));

    assert_standard_outputs(tempdir.path()).await;
    assert_log_redaction_and_success(tempdir.path()).await;
    let requests_after_index = server.received_requests().await.expect("requests");
    assert_eq!(
        requests_after_index
            .iter()
            .filter(|request| is_completion_connectivity_request(request))
            .count(),
        2,
        "dry-run and real index should each validate completion exactly once",
    );
    assert_eq!(
        requests_after_index
            .iter()
            .filter(|request| is_embedding_connectivity_request(request))
            .count(),
        2,
        "dry-run and real index should each validate embeddings exactly once",
    );
    let first_document_ids = table_ids(tempdir.path(), "documents").await;
    let first_vector_ids = managed_vector_ids(tempdir.path()).await;
    assert!(tempdir.path().join("cache").exists());

    assert_full_rerun_resets_vector_ids(tempdir.path(), &first_document_ids, &first_vector_ids)
        .await;
    assert!(tempdir.path().join("cache").exists());
}

#[tokio::test]
async fn test_should_bypass_matching_cache_during_dry_run_connectivity() {
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
        tempdir.path().join(".env"),
        "GRAPHRAG_API_KEY=super-secret-key\n",
    )
    .await
    .expect("env");
    tokio::fs::write(tempdir.path().join("input").join("document.txt"), "Alice")
        .await
        .expect("input");
    patch_settings(tempdir.path(), &server.uri()).await;
    write_matching_validation_cache(tempdir.path()).await;
    let cache = FileStorage::existing(tempdir.path().join("cache")).expect("cache storage");
    let before = cache.list("").await.expect("cache files before dry run");

    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "index",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--dry-run",
        ])
        .assert()
        .success();

    assert_eq!(server.received_requests().await.expect("requests").len(), 2);
    assert_eq!(
        cache.list("").await.expect("cache files after dry run"),
        before
    );
    assert!(!tempdir.path().join("output").exists());
    assert!(!tempdir.path().join("logs").exists());
}

#[tokio::test]
async fn test_should_write_text_units_in_graphrag_3_1_schema() {
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
    run_minimal_standard_index(tempdir.path(), &server.uri()).await;

    let provider = ParquetTableProvider::new(tempdir.path().join("output")).expect("provider");
    let text_units = provider
        .read_dataframe("text_units")
        .await
        .expect("text_units");
    let documents = provider
        .read_dataframe("documents")
        .await
        .expect("documents");

    assert_columns(
        &text_units,
        &[
            "id",
            "human_readable_id",
            "text",
            "n_tokens",
            "document_id",
            "entity_ids",
            "relationship_ids",
            "covariate_ids",
        ],
    );
    assert_dtype(&text_units, "document_id", &DataType::String);
    assert!(
        text_units.column("document_ids").is_err(),
        "GraphRAG 3.1 text_units must not contain document_ids"
    );
    let document_ids = string_set(&documents, "id");
    let text_unit_ids = string_set(&text_units, "id");
    assert_subset(&string_set(&text_units, "document_id"), &document_ids);
    for document_id in &document_ids {
        let mut reverse = BTreeSet::new();
        let document_id_column = documents.column("id").expect("id").str().expect("id str");
        for row_index in 0..documents.height() {
            if document_id_column.get(row_index) == Some(document_id.as_str()) {
                let row = documents.get_row(row_index).expect("document row");
                let text_unit_ids_index = documents
                    .get_column_names()
                    .iter()
                    .position(|name| name.as_str() == "text_unit_ids")
                    .expect("text_unit_ids");
                if let Some(value) = row.0.get(text_unit_ids_index) {
                    reverse.extend(any_value_to_strings(value));
                }
            }
        }
        let forward = (0..text_units.height())
            .filter_map(|row_index| {
                let ids = text_units.column("id").ok()?.str().ok()?;
                let document_id_values = text_units.column("document_id").ok()?.str().ok()?;
                (document_id_values.get(row_index) == Some(document_id.as_str()))
                    .then(|| ids.get(row_index).map(str::to_owned))
                    .flatten()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(reverse, forward);
    }
    assert_subset(
        &list_string_set(&documents, "text_unit_ids"),
        &text_unit_ids,
    );
}

async fn run_minimal_standard_index(root: &std::path::Path, server_uri: &str) {
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
    tokio::fs::write(
        root.join("input").join("document.txt"),
        "Alice works for Acme.",
    )
    .await
    .expect("write input");
    tokio::fs::write(root.join(".env"), "GRAPHRAG_API_KEY=test-key\n")
        .await
        .expect("write env");
    patch_settings(root, server_uri).await;

    Command::cargo_bin("graphloom")
        .expect("binary")
        .args(["index", "--root", root.to_str().expect("utf8 root")])
        .assert()
        .success();
}

async fn assert_full_rerun_resets_vector_ids(
    root: &std::path::Path,
    first_document_ids: &BTreeSet<String>,
    first_vector_ids: &std::collections::BTreeMap<String, BTreeSet<String>>,
) {
    tokio::fs::write(
        root.join("input").join("document.txt"),
        "Carol founded Beta.",
    )
    .await
    .expect("replace input");
    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "index",
            "--root",
            root.to_str().expect("utf8 root"),
            "--no-cache",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Index completed successfully"));

    let second_document_ids = table_ids(root, "documents").await;
    let second_vector_ids = managed_vector_ids(root).await;
    assert!(
        first_document_ids.is_disjoint(&second_document_ids),
        "full rerun should replace document output instead of appending"
    );
    for embedding_name in ALL_EMBEDDINGS {
        let old_ids = first_vector_ids.get(*embedding_name).expect("old ids");
        let new_ids = second_vector_ids.get(*embedding_name).expect("new ids");
        assert!(
            old_ids.is_disjoint(new_ids),
            "{embedding_name} full rerun should remove old vector ids"
        );
        assert!(
            !new_ids.is_empty(),
            "{embedding_name} should contain second-run vector ids"
        );
    }
    assert_eq!(
        second_vector_ids
            .get("entity_description")
            .expect("entity ids"),
        &table_ids(root, "entities").await
    );
    assert_eq!(
        second_vector_ids
            .get("community_full_content")
            .expect("community ids"),
        &table_ids(root, "community_reports").await
    );
    assert_eq!(
        second_vector_ids
            .get("text_unit_text")
            .expect("text unit ids"),
        &table_ids(root, "text_units").await
    );
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
async fn test_should_fail_dry_run_on_real_model_authentication_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": {"message": "invalid API key super-secret-key"}
        })))
        .mount(&server)
        .await;
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
        .failure()
        .stderr(predicate::str::contains("default_completion_model"))
        .stderr(predicate::str::contains("completion connectivity check"))
        .stderr(predicate::str::contains("super-secret-key").not());
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
async fn test_should_skip_optional_validation_for_real_index_but_keep_dry_run_side_effect_free() {
    let tempdir = TempDir::new().expect("tempdir");
    init_project(tempdir.path());
    tokio::fs::write(
        tempdir.path().join(".env"),
        "GRAPHRAG_API_KEY=super-secret-key\n",
    )
    .await
    .expect("env");
    tokio::fs::write(
        tempdir.path().join("input").join("document.txt"),
        "Alice works for Acme.",
    )
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
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to load ExtractGraph"))
        .stderr(predicate::str::contains("super-secret-key").not());

    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "index",
            "--root",
            tempdir.path().to_str().expect("utf8 root"),
            "--skip-validation",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "index workflow `extract_graph` failed",
        ))
        .stderr(predicate::str::contains("super-secret-key").not());

    let log = tokio::fs::read_to_string(tempdir.path().join("logs").join("indexing-engine.log"))
        .await
        .expect("log");
    assert!(log.contains("index run started"));
    assert!(log.contains("workflow_name=extract_graph") || log.contains("extract_graph"));
    assert!(!log.contains("super-secret-key"));

    let dry_run_root = TempDir::new().expect("dry run tempdir");
    init_project(dry_run_root.path());
    tokio::fs::write(
        dry_run_root.path().join("input").join("document.txt"),
        "Alice works for Acme.",
    )
    .await
    .expect("dry input");

    Command::cargo_bin("graphloom")
        .expect("binary")
        .args([
            "index",
            "--root",
            dry_run_root.path().to_str().expect("utf8 root"),
            "--dry-run",
            "--skip-validation",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Workflows:"))
        .stdout(predicate::str::contains("<API_KEY>").not());
    assert!(!dry_run_root.path().join("output").exists());
    assert!(!dry_run_root.path().join("cache").exists());
    assert!(!dry_run_root.path().join("logs").exists());
    assert!(!dry_run_root.path().join("output").join("lancedb").exists());
}

#[tokio::test]
async fn test_should_fail_dry_run_when_no_input_matches_pattern() {
    let tempdir = TempDir::new().expect("tempdir");
    init_project(tempdir.path());

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
            "output directory must not overlap logs directory",
        ),
        (
            CliPreflightCase::OutputParentIsFile,
            "path ancestor is not a directory",
        ),
        (CliPreflightCase::UnknownIndexWorkflow, "not_a_workflow"),
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
            "--skip-validation",
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

#[tokio::test]
async fn test_should_fail_embedding_cardinality_mismatch_without_secret() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(chat_responder)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(embedding_cardinality_mismatch_responder)
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
        .stderr(predicate::str::contains("embedding connectivity check"))
        .stderr(predicate::str::contains("embedding returned no vectors"))
        .stderr(predicate::str::contains("super-secret-key").not());

    assert!(!tempdir.path().join("logs").exists());
    assert!(!tempdir.path().join("output").exists());
}

#[tokio::test]
async fn test_should_skip_malformed_community_report_without_secret() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(chat_responder_malformed_report)
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
        .success()
        .stdout(predicate::str::contains("Reports: 0"))
        .stderr(predicate::str::contains("returned invalid JSON"))
        .stderr(predicate::str::contains("super-secret-key").not());

    assert!(
        tempdir
            .path()
            .join("output")
            .join("community_reports.parquet")
            .exists(),
        "malformed reports should be skipped while preserving the workflow output",
    );
    let log = tokio::fs::read_to_string(tempdir.path().join("logs").join("indexing-engine.log"))
        .await
        .expect("log");
    assert!(log.contains("create_community_reports"));
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
    OutputParentIsFile,
    UnknownIndexWorkflow,
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
    assert_parquet_schema_and_integrity(&provider).await;

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
    assert_lancedb_content(root, &config, store.as_ref()).await;
    let reopened = LanceDbVectorStore::connect(&config.vector_store)
        .await
        .expect("reopen lancedb");
    assert_lancedb_content(root, &config, &reopened).await;
}

async fn assert_lancedb_content(
    root: &std::path::Path,
    config: &GraphRagConfig,
    store: &dyn graphloom_vectors::VectorStore,
) {
    let expected_sources = [
        ("entity_description", table_ids(root, "entities").await),
        (
            "community_full_content",
            table_ids(root, "community_reports").await,
        ),
        ("text_unit_text", table_ids(root, "text_units").await),
    ];
    for embedding in ALL_EMBEDDINGS {
        let schema = config.vector_store.schema_for(embedding);
        let ids = store
            .ids(&schema)
            .await
            .expect("vector ids")
            .into_iter()
            .collect::<BTreeSet<_>>();
        let expected = expected_sources
            .iter()
            .find_map(|(name, ids)| (*name == *embedding).then_some(ids))
            .expect("expected ids");
        assert_eq!(&ids, expected, "{embedding} ids should match source table");
        assert_eq!(store.count(&schema).await.expect("count"), ids.len());
        for id in ids {
            let document = store
                .get_by_id(&schema, &id)
                .await
                .expect("get vector")
                .expect("vector document");
            assert_eq!(document.vector.len(), schema.vector_size);
            assert!(
                document.vector.iter().all(|value| value.is_finite()),
                "{embedding} vector {id} should contain only finite values",
            );
        }
    }
}

struct OutputFrames {
    documents: DataFrame,
    text_units: DataFrame,
    entities: DataFrame,
    relationships: DataFrame,
    communities: DataFrame,
    community_reports: DataFrame,
}

async fn assert_parquet_schema_and_integrity(provider: &ParquetTableProvider) {
    let frames = OutputFrames {
        documents: provider
            .read_dataframe("documents")
            .await
            .expect("documents"),
        text_units: provider
            .read_dataframe("text_units")
            .await
            .expect("text_units"),
        entities: provider.read_dataframe("entities").await.expect("entities"),
        relationships: provider
            .read_dataframe("relationships")
            .await
            .expect("relationships"),
        communities: provider
            .read_dataframe("communities")
            .await
            .expect("communities"),
        community_reports: provider
            .read_dataframe("community_reports")
            .await
            .expect("community_reports"),
    };

    assert_parquet_column_order(&frames);
    assert_parquet_dtypes(&frames);
    assert_reference_integrity(&frames);
}

fn assert_parquet_column_order(frames: &OutputFrames) {
    assert_columns(
        &frames.documents,
        &[
            "id",
            "human_readable_id",
            "title",
            "text",
            "text_unit_ids",
            "creation_date",
            "raw_data",
        ],
    );
    assert_columns(
        &frames.text_units,
        &[
            "id",
            "human_readable_id",
            "text",
            "n_tokens",
            "document_id",
            "entity_ids",
            "relationship_ids",
            "covariate_ids",
        ],
    );
    assert_columns(
        &frames.entities,
        &[
            "id",
            "human_readable_id",
            "title",
            "type",
            "description",
            "text_unit_ids",
            "frequency",
            "degree",
        ],
    );
    assert_columns(
        &frames.relationships,
        &[
            "id",
            "human_readable_id",
            "source",
            "target",
            "description",
            "weight",
            "combined_degree",
            "text_unit_ids",
        ],
    );
    assert_columns(
        &frames.communities,
        &[
            "id",
            "human_readable_id",
            "community",
            "level",
            "parent",
            "children",
            "title",
            "entity_ids",
            "relationship_ids",
            "text_unit_ids",
            "period",
            "size",
        ],
    );
    assert_columns(
        &frames.community_reports,
        &[
            "id",
            "human_readable_id",
            "community",
            "level",
            "parent",
            "children",
            "title",
            "summary",
            "full_content",
            "rank",
            "rating_explanation",
            "findings",
            "full_content_json",
            "period",
            "size",
        ],
    );
}

fn assert_parquet_dtypes(frames: &OutputFrames) {
    assert_common_dtypes(&frames.documents, &["title", "text"]);
    assert_common_dtypes(&frames.text_units, &["text", "document_id"]);
    assert_common_dtypes(&frames.entities, &["title", "type", "description"]);
    assert_common_dtypes(&frames.relationships, &["source", "target", "description"]);
    assert_common_dtypes(&frames.communities, &["title", "period"]);
    assert_common_dtypes(
        &frames.community_reports,
        &[
            "title",
            "summary",
            "full_content",
            "rating_explanation",
            "full_content_json",
            "period",
        ],
    );
    for (dataframe, integer_columns) in [
        (&frames.documents, &["human_readable_id"][..]),
        (&frames.text_units, &["human_readable_id", "n_tokens"][..]),
        (
            &frames.entities,
            &["human_readable_id", "frequency", "degree"][..],
        ),
        (
            &frames.relationships,
            &["human_readable_id", "combined_degree"][..],
        ),
        (
            &frames.communities,
            &["human_readable_id", "community", "level", "parent", "size"][..],
        ),
        (
            &frames.community_reports,
            &["human_readable_id", "community", "level", "parent", "size"][..],
        ),
    ] {
        for column in integer_columns {
            assert_dtype(dataframe, column, &DataType::Int64);
        }
    }
    assert_dtype(&frames.relationships, "weight", &DataType::Float64);
    assert_dtype(&frames.community_reports, "rank", &DataType::Float64);
    for (dataframe, list_columns) in [
        (&frames.documents, &["text_unit_ids"][..]),
        (
            &frames.text_units,
            &["entity_ids", "relationship_ids", "covariate_ids"][..],
        ),
        (&frames.entities, &["text_unit_ids"][..]),
        (&frames.relationships, &["text_unit_ids"][..]),
        (
            &frames.communities,
            &["entity_ids", "relationship_ids", "text_unit_ids"][..],
        ),
    ] {
        for column in list_columns {
            assert_dtype(
                dataframe,
                column,
                &DataType::List(Box::new(DataType::String)),
            );
        }
    }
    assert_dtype(
        &frames.communities,
        "children",
        &DataType::List(Box::new(DataType::Int64)),
    );
    assert_dtype(
        &frames.community_reports,
        "children",
        &DataType::List(Box::new(DataType::Int64)),
    );
}

fn assert_reference_integrity(frames: &OutputFrames) {
    let document_ids = string_set(&frames.documents, "id");
    let text_unit_ids = string_set(&frames.text_units, "id");
    let entity_ids = string_set(&frames.entities, "id");
    let entity_titles = string_set(&frames.entities, "title");
    let relationship_ids = string_set(&frames.relationships, "id");
    let community_keys = i64_set(&frames.communities, "community");

    assert_subset(
        &list_string_set(&frames.documents, "text_unit_ids"),
        &text_unit_ids,
    );
    assert_subset(
        &string_set(&frames.text_units, "document_id"),
        &document_ids,
    );
    assert_subset(
        &list_string_set(&frames.text_units, "entity_ids"),
        &entity_ids,
    );
    assert_subset(
        &list_string_set(&frames.text_units, "relationship_ids"),
        &relationship_ids,
    );
    assert_subset(
        &list_string_set(&frames.entities, "text_unit_ids"),
        &text_unit_ids,
    );
    assert_subset(&string_set(&frames.relationships, "source"), &entity_titles);
    assert_subset(&string_set(&frames.relationships, "target"), &entity_titles);
    assert_subset(
        &list_string_set(&frames.relationships, "text_unit_ids"),
        &text_unit_ids,
    );
    assert_subset(
        &list_string_set(&frames.communities, "entity_ids"),
        &entity_ids,
    );
    assert_subset(
        &list_string_set(&frames.communities, "relationship_ids"),
        &relationship_ids,
    );
    assert_subset(
        &list_string_set(&frames.communities, "text_unit_ids"),
        &text_unit_ids,
    );
    assert_subset_i64(
        &i64_set(&frames.community_reports, "community"),
        &community_keys,
    );
    validate_document_text_unit_mapping(&frames.documents, &frames.text_units)
        .expect("documents.text_unit_ids should exactly mirror text_units.document_id");
}

#[test]
fn test_should_reject_incomplete_document_text_unit_reverse_mapping() {
    let documents = DataFrame::new(
        1,
        vec![
            Series::new("id".into(), ["doc-1"]).into(),
            string_list_column("text_unit_ids", &[&["tu-1"]]),
        ],
    )
    .expect("documents");
    let text_units = DataFrame::new(
        2,
        vec![
            Series::new("id".into(), ["tu-1", "tu-2"]).into(),
            Series::new("document_id".into(), ["doc-1", "doc-1"]).into(),
        ],
    )
    .expect("text_units");

    let error = validate_document_text_unit_mapping(&documents, &text_units)
        .expect_err("missing reverse mapping should fail");

    assert!(error.contains("doc-1"));
    assert!(error.contains("tu-2"));
}

fn validate_document_text_unit_mapping(
    documents: &DataFrame,
    text_units: &DataFrame,
) -> Result<(), String> {
    let document_ids = string_set(documents, "id");
    let text_unit_ids = string_set(text_units, "id");
    let mut expected = std::collections::BTreeMap::<String, BTreeSet<String>>::new();
    let text_unit_id_column = text_units
        .column("id")
        .map_err(|source| source.to_string())?
        .str()
        .map_err(|source| source.to_string())?;
    let document_id_column = text_units
        .column("document_id")
        .map_err(|source| source.to_string())?
        .str()
        .map_err(|source| source.to_string())?;
    let mut seen_text_unit_ids = BTreeSet::new();
    for row_index in 0..text_units.height() {
        let text_unit_id = text_unit_id_column
            .get(row_index)
            .ok_or_else(|| format!("text_units row {row_index} has null id"))?;
        if !seen_text_unit_ids.insert(text_unit_id.to_owned()) {
            return Err(format!("duplicate text unit id {text_unit_id}"));
        }
        let document_id = document_id_column
            .get(row_index)
            .ok_or_else(|| format!("text_units row {row_index} has null document_id"))?;
        if !document_ids.contains(document_id) {
            return Err(format!(
                "text unit {text_unit_id} references missing document {document_id}"
            ));
        }
        expected
            .entry(document_id.to_owned())
            .or_default()
            .insert(text_unit_id.to_owned());
    }

    let mut actual = std::collections::BTreeMap::<String, BTreeSet<String>>::new();
    let document_id_column = documents
        .column("id")
        .map_err(|source| source.to_string())?
        .str()
        .map_err(|source| source.to_string())?;
    let text_unit_ids_index = documents
        .get_column_names()
        .iter()
        .position(|name| name.as_str() == "text_unit_ids")
        .ok_or_else(|| "documents.text_unit_ids column is missing".to_owned())?;
    let mut seen_document_ids = BTreeSet::new();
    for row_index in 0..documents.height() {
        let document_id = document_id_column
            .get(row_index)
            .ok_or_else(|| format!("documents row {row_index} has null id"))?;
        if !seen_document_ids.insert(document_id.to_owned()) {
            return Err(format!("duplicate document id {document_id}"));
        }
        let row = documents
            .get_row(row_index)
            .map_err(|source| source.to_string())?;
        let values = row
            .0
            .get(text_unit_ids_index)
            .ok_or_else(|| format!("documents row {row_index} has no text_unit_ids value"))?;
        let values = any_value_to_strings(values)
            .into_iter()
            .map(|text_unit_id| {
                if text_unit_ids.contains(&text_unit_id) {
                    Ok(text_unit_id)
                } else {
                    Err(format!(
                        "document {document_id} references missing text unit {text_unit_id}"
                    ))
                }
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        actual.insert(document_id.to_owned(), values);
    }

    for document_id in &document_ids {
        let empty = BTreeSet::new();
        let expected_ids = expected.get(document_id).unwrap_or(&empty);
        let actual_ids = actual.get(document_id).unwrap_or(&empty);
        if actual_ids != expected_ids {
            return Err(format!(
                "document {document_id} text_unit_ids mismatch: expected {expected_ids:?}, got \
                 {actual_ids:?}",
            ));
        }
    }
    Ok(())
}

fn assert_columns(dataframe: &DataFrame, expected: &[&str]) {
    assert_eq!(column_names(dataframe), expected);
}

fn column_names(dataframe: &DataFrame) -> Vec<&str> {
    dataframe
        .get_column_names()
        .into_iter()
        .map(PlSmallStr::as_str)
        .collect()
}

fn assert_common_dtypes(dataframe: &DataFrame, string_columns: &[&str]) {
    assert_dtype(dataframe, "id", &DataType::String);
    for column in string_columns {
        assert_dtype(dataframe, column, &DataType::String);
    }
}

fn assert_dtype(dataframe: &DataFrame, column: &str, expected: &DataType) {
    assert_eq!(
        dataframe.column(column).expect("column").dtype(),
        expected,
        "{column} dtype mismatch",
    );
}

fn string_set(dataframe: &DataFrame, column: &str) -> BTreeSet<String> {
    dataframe
        .column(column)
        .expect("column")
        .str()
        .expect("string column")
        .iter()
        .flatten()
        .map(ToOwned::to_owned)
        .collect()
}

fn i64_set(dataframe: &DataFrame, column: &str) -> BTreeSet<i64> {
    dataframe
        .column(column)
        .expect("column")
        .i64()
        .expect("i64 column")
        .iter()
        .flatten()
        .collect()
}

fn list_string_set(dataframe: &DataFrame, column: &str) -> BTreeSet<String> {
    let column_index = dataframe
        .get_column_names()
        .iter()
        .position(|name| name.as_str() == column)
        .expect("list column");
    let mut values = BTreeSet::new();
    for row_index in 0..dataframe.height() {
        let row = dataframe.get_row(row_index).expect("row");
        if let Some(value) = row.0.get(column_index) {
            values.extend(any_value_to_strings(value));
        }
    }
    values
}

fn string_list_column(name: &str, rows: &[&[&str]]) -> polars_core::prelude::Column {
    let series = rows
        .iter()
        .map(|values| Series::new("item".into(), *values))
        .collect::<Vec<_>>();
    Series::new(name.into(), series).into()
}

fn any_value_to_strings(value: &AnyValue<'_>) -> Vec<String> {
    match value {
        AnyValue::List(series) => series
            .str()
            .expect("string list")
            .iter()
            .flatten()
            .map(ToOwned::to_owned)
            .collect(),
        AnyValue::Null => Vec::new(),
        AnyValue::String(value) => vec![(*value).to_owned()],
        AnyValue::StringOwned(value) => vec![value.to_string()],
        other => panic!("expected string list, got {other:?}"),
    }
}

fn assert_subset(actual: &BTreeSet<String>, expected: &BTreeSet<String>) {
    assert!(
        actual.is_subset(expected),
        "unexpected references: {:?}",
        actual.difference(expected).collect::<Vec<_>>(),
    );
}

fn assert_subset_i64(actual: &BTreeSet<i64>, expected: &BTreeSet<i64>) {
    assert!(
        actual.is_subset(expected),
        "unexpected integer references: {:?}",
        actual.difference(expected).collect::<Vec<_>>(),
    );
}

async fn assert_log_redaction_and_success(root: &std::path::Path) {
    let log = tokio::fs::read_to_string(root.join("logs").join("indexing-engine.log"))
        .await
        .expect("log");
    assert!(log.contains("index run started"));
    assert!(log.contains("index run completed"));
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

async fn parquet_artifact_snapshot(
    root: &std::path::Path,
) -> BTreeMap<std::path::PathBuf, (Vec<u8>, SystemTime)> {
    let mut entries = tokio::fs::read_dir(root.join("output"))
        .await
        .expect("output directory");
    let mut snapshot = BTreeMap::new();
    while let Some(entry) = entries.next_entry().await.expect("output entry") {
        let path = entry.path();
        if path
            .extension()
            .is_none_or(|extension| extension != "parquet")
        {
            continue;
        }
        let bytes = tokio::fs::read(&path).await.expect("Parquet artifact");
        let modified = entry
            .metadata()
            .await
            .expect("Parquet metadata")
            .modified()
            .expect("Parquet modified time");
        snapshot.insert(path, (bytes, modified));
    }
    assert!(!snapshot.is_empty(), "index must produce Parquet artifacts");
    snapshot
}

async fn hide_unneeded_parquet(
    root: &std::path::Path,
    allowed_tables: &[&str],
) -> Vec<(std::path::PathBuf, std::path::PathBuf)> {
    let mut entries = tokio::fs::read_dir(root.join("output"))
        .await
        .expect("output directory");
    let mut hidden = Vec::new();
    while let Some(entry) = entries.next_entry().await.expect("output entry") {
        let path = entry.path();
        if path
            .extension()
            .is_none_or(|extension| extension != "parquet")
        {
            continue;
        }
        let table = path
            .file_stem()
            .and_then(std::ffi::OsStr::to_str)
            .expect("UTF-8 table name");
        if allowed_tables.contains(&table) {
            continue;
        }
        let hidden_path = path.with_extension("parquet.query-test-hidden");
        tokio::fs::rename(&path, &hidden_path)
            .await
            .expect("hide unrelated Query table");
        hidden.push((hidden_path, path));
    }
    hidden
}

async fn restore_hidden_parquet(hidden: &[(std::path::PathBuf, std::path::PathBuf)]) {
    for (hidden_path, original_path) in hidden {
        tokio::fs::rename(hidden_path, original_path)
            .await
            .expect("restore unrelated Query table");
    }
}

async fn managed_vector_ids(
    root: &std::path::Path,
) -> std::collections::BTreeMap<String, BTreeSet<String>> {
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
    let mut ids = std::collections::BTreeMap::new();
    for embedding_name in ALL_EMBEDDINGS {
        let schema = config.vector_store.schema_for(embedding_name);
        ids.insert(
            (*embedding_name).to_owned(),
            store.ids(&schema).await.expect("ids").into_iter().collect(),
        );
    }
    ids
}

async fn copy_vector_indices(
    root: &std::path::Path,
    target_uri: &std::path::Path,
    embedding_names: &[&str],
) -> std::collections::BTreeMap<String, BTreeSet<String>> {
    let settings = tokio::fs::read_to_string(root.join("settings.yaml"))
        .await
        .expect("settings");
    let mut source_config: GraphRagConfig = serde_yaml::from_str(&settings).expect("config");
    source_config.vector_store.db_uri = root
        .join("output")
        .join("lancedb")
        .to_string_lossy()
        .to_string();
    let source = LanceDbVectorStore::connect(&source_config.vector_store)
        .await
        .expect("source LanceDB");
    let mut target_config = source_config.clone();
    target_config.vector_store.db_uri = target_uri.to_string_lossy().to_string();
    let target = LanceDbVectorStore::connect(&target_config.vector_store)
        .await
        .expect("target LanceDB");
    let mut expected_ids = std::collections::BTreeMap::new();
    for embedding_name in embedding_names {
        let schema = source_config.vector_store.schema_for(embedding_name);
        let ids = source.ids(&schema).await.expect("source vector ids");
        let mut documents = Vec::with_capacity(ids.len());
        for id in &ids {
            documents.push(
                source
                    .get_by_id(&schema, id)
                    .await
                    .expect("source vector")
                    .expect("source vector document"),
            );
        }
        target
            .ensure_index(&schema)
            .await
            .expect("target vector index");
        target
            .upsert_documents(&schema, &documents)
            .await
            .expect("target vectors");
        expected_ids.insert((*embedding_name).to_owned(), ids.into_iter().collect());
    }
    expected_ids
}

async fn assert_vector_indices_unchanged(
    settings: &str,
    db_uri: &std::path::Path,
    expected: &std::collections::BTreeMap<String, BTreeSet<String>>,
) {
    let mut config: GraphRagConfig = serde_yaml::from_str(settings).expect("config");
    config.vector_store.db_uri = db_uri.to_string_lossy().to_string();
    let store = LanceDbVectorStore::connect(&config.vector_store)
        .await
        .expect("dedicated LanceDB");
    for (embedding_name, expected_ids) in expected {
        let schema = config.vector_store.schema_for(embedding_name);
        let actual = store
            .ids(&schema)
            .await
            .expect("dedicated vector ids")
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert_eq!(&actual, expected_ids, "{embedding_name} vector ids changed");
    }
}

async fn set_query_vector_db(root: &std::path::Path, settings: &str, db_uri: &std::path::Path) {
    let mut config: GraphRagConfig = serde_yaml::from_str(settings).expect("config");
    config.vector_store.db_uri = db_uri.to_string_lossy().to_string();
    tokio::fs::write(
        root.join("settings.yaml"),
        serde_yaml::to_string(&config).expect("serialize config"),
    )
    .await
    .expect("write Query vector configuration");
}

async fn patch_settings(root: &std::path::Path, server_uri: &str) {
    patch_settings_with_max_gleanings(root, server_uri, 0).await;
}

async fn write_matching_validation_cache(root: &std::path::Path) {
    let completion_request = CompletionRequest::new(vec![ChatMessage::user(
        "This is an LLM connectivity test. Say Hello World".to_owned(),
    )]);
    let completion_key =
        completion_request_cache_key(&completion_request).expect("completion cache key");
    let completion = CachedModelResult {
        response: CompletionResponse::text_for_test("cached", "cached hello"),
        metrics: std::collections::BTreeMap::default(),
    };
    for namespace in [
        "default_completion_model",
        "extract_graph",
        "summarize_descriptions",
        "community_reporting",
    ] {
        let directory = root.join("cache").join(namespace);
        tokio::fs::create_dir_all(&directory)
            .await
            .expect("completion cache directory");
        tokio::fs::write(
            directory.join(&completion_key),
            serde_json::to_vec(&json!({"result": &completion})).expect("completion cache payload"),
        )
        .await
        .expect("completion cache entry");
    }

    let embedding_request =
        EmbeddingRequest::new(vec!["This is an LLM Embedding Test String".to_owned()]);
    let embedding_key =
        embedding_request_cache_key(&embedding_request).expect("embedding cache key");
    let embedding = CachedModelResult {
        response: EmbeddingResponse::vectors_for_test("cached", vec![vec![9.0; 4]]),
        metrics: std::collections::BTreeMap::default(),
    };
    for namespace in ["default_embedding_model", "text_embedding"] {
        let directory = root.join("cache").join(namespace);
        tokio::fs::create_dir_all(&directory)
            .await
            .expect("embedding cache directory");
        tokio::fs::write(
            directory.join(&embedding_key),
            serde_json::to_vec(&json!({"result": &embedding})).expect("embedding cache payload"),
        )
        .await
        .expect("embedding cache entry");
    }
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
    if matches!(case, CliPreflightCase::OutputParentIsFile) {
        tokio::fs::write(root.join("output-parent-file"), "not a directory")
            .await
            .expect("output parent file");
    }
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
        CliPreflightCase::OutputParentIsFile => vec![(
            vec!["output_storage", "base_dir"],
            string("output-parent-file/output"),
        )],
        CliPreflightCase::UnknownIndexWorkflow => vec![(
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
    let first = messages
        .first()
        .and_then(|message| message["content"].as_str())
        .unwrap_or_default();
    if body["stream"].as_bool() == Some(true) {
        if body.get("response_format").is_some() {
            return query_stream_response(
                r#"{"response":"DRIFT action.","score":90,"follow_up_queries":[]}"#,
            );
        }
        let answer = if first.contains("dataset by synthesizing perspectives") {
            "Global answer."
        } else if first.contains("data in the reports provided") {
            "DRIFT answer."
        } else if first.contains("incorporating any relevant general knowledge") {
            "Local answer."
        } else {
            "Basic answer."
        };
        return query_stream_response(answer);
    }
    let last = messages
        .last()
        .and_then(|message| message["content"].as_str())
        .unwrap_or_default();
    let content = if first.contains("deciding whether the provided information is useful") {
        r#"{"reason":"relevant","rating":5}"#.to_owned()
    } else if first.contains("list of key points") && body.get("response_format").is_some() {
        r#"{"points":[{"description":"Mapped fact [Data: Reports (0)]","score":8}]}"#.to_owned()
    } else if first.starts_with("Create a hypothetical answer") {
        "Expanded query answer.".to_owned()
    } else if first.starts_with("You are a helpful agent designed to reason") {
        r#"{"intermediate_answer":"Primer answer.","score":90,"follow_up_queries":["Who?"]}"#
            .to_owned()
    } else if first.contains("data in the reports provided") {
        "DRIFT answer.".to_owned()
    } else if last.contains("Given a text document") && last.contains("Carol founded Beta") {
        "(\"entity\"<|>CAROL<|>person<|>Carol founded \
         Beta)##(\"entity\"<|>BETA<|>organization<|>Beta was founded by \
         Carol)##(\"relationship\"<|>CAROL<|>BETA<|>Carol founded Beta<|>5)##<|COMPLETE|>"
            .to_owned()
    } else if last.contains("Given a text document") {
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

fn chat_responder_with_drift_parse_failure(request: &Request) -> ResponseTemplate {
    let body = request.body_json::<Value>().expect("chat request json");
    let first = body["messages"]
        .as_array()
        .and_then(|messages| messages.first())
        .and_then(|message| message["content"].as_str())
        .unwrap_or_default();
    if first.starts_with("You are a helpful agent designed to reason") {
        return chat_response("invalid DRIFT primer");
    }
    chat_responder(request)
}

fn query_stream_response(answer: &str) -> ResponseTemplate {
    let split = answer
        .find(' ')
        .map_or(answer.len(), |index| index.saturating_add(1));
    let (first, second) = answer.split_at(split);
    let body = format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({
            "id": "query-1",
            "model": "gpt-test",
            "choices": [{
                "index": 0,
                "delta": {"content": first},
                "finish_reason": null
            }]
        }),
        json!({
            "id": "query-2",
            "model": "gpt-test",
            "choices": [{
                "index": 0,
                "delta": {"content": second},
                "finish_reason": "stop"
            }]
        }),
    );
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

fn partial_failure_stream_response() -> ResponseTemplate {
    let body = concat!(
        "data: {\"id\":\"query-1\",\"model\":\"gpt-test\",\"choices\":[{\"index\":0,\"delta\":{\"\
         content\":\"Basic \"},\"finish_reason\":null}]}\n\n",
        "data: {not valid JSON}\n\n"
    );
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

fn chat_content_for_non_extraction(last: &str) -> String {
    if last.contains("Return output as a well-formed JSON-formatted string") {
        let sequence = REPORT_COUNTER.fetch_add(1, Ordering::SeqCst);
        if mentions_carol_beta(last) {
            json!({
                "title": format!("Carol and Beta {sequence}"),
                "summary": format!("Carol and Beta form a founder-company community {sequence}."),
                "rating": 6.0,
                "rating_explanation": "The community captures a founding relationship.",
                "findings": [
                    {
                        "summary": "Founding",
                        "explanation": "Carol founded Beta [Data: Entities (0, 1); Relationships (0)]."
                    }
                ]
            })
            .to_string()
        } else {
            json!({
                "title": format!("Alice, Bob, and Acme {sequence}"),
                "summary": format!("Alice, Bob, and Acme form a connected work community {sequence}."),
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
        }
    } else if mentions_carol_beta(last) {
        "Carol founded Beta summary.".to_owned()
    } else {
        "A concise summary of the provided descriptions.".to_owned()
    }
}

fn mentions_carol_beta(value: &str) -> bool {
    value.contains("CAROL")
        || value.contains("BETA")
        || value.contains("Carol")
        || value.contains("Beta")
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

fn chat_responder_malformed_report(request: &Request) -> ResponseTemplate {
    let body = request.body_json::<Value>().expect("chat request json");
    let messages = body["messages"].as_array().expect("messages");
    let last = messages
        .last()
        .and_then(|message| message["content"].as_str())
        .unwrap_or_default();
    let content = if last.contains("Return output as a well-formed JSON-formatted string") {
        "not valid json".to_owned()
    } else if last.contains("Given a text document") {
        "(\"entity\"<|>ALICE<|>person<|>Alice works for \
         Acme)##(\"entity\"<|>ACME<|>organization<|>Acme employs \
         Alice)##(\"relationship\"<|>ALICE<|>ACME<|>Alice works for Acme<|>5)##<|COMPLETE|>"
            .to_owned()
    } else {
        "A concise summary of the provided descriptions.".to_owned()
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

fn is_completion_connectivity_request(request: &Request) -> bool {
    request
        .body_json::<Value>()
        .ok()
        .and_then(|body| {
            body["messages"]
                .as_array()
                .and_then(|messages| messages.last())
                .and_then(|message| message["content"].as_str().map(str::to_owned))
        })
        .is_some_and(|content| content == "This is an LLM connectivity test. Say Hello World")
}

fn is_embedding_connectivity_request(request: &Request) -> bool {
    request
        .body_json::<Value>()
        .ok()
        .and_then(|body| body["input"].as_array().cloned())
        .is_some_and(|inputs| {
            inputs.len() == 1
                && inputs.first().and_then(Value::as_str)
                    == Some("This is an LLM Embedding Test String")
        })
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

fn embedding_cardinality_mismatch_responder(request: &Request) -> ResponseTemplate {
    let body = request
        .body_json::<Value>()
        .expect("embedding request json");
    let inputs = body["input"].as_array().expect("input");
    let response_count = inputs.len().saturating_sub(1);
    let data = (0..response_count)
        .map(|index| {
            json!({
                "object": "embedding",
                "index": index,
                "embedding": [1.0, 0.0, 0.0, 0.0]
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

type DelayedServer = (
    String,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
    std::thread::JoinHandle<()>,
);

fn delayed_query_server() -> DelayedServer {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind delayed query server");
    let address = listener.local_addr().expect("server address");
    let (first_chunk_tx, first_chunk_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept query request");
            let path = read_http_request_path(&mut stream).expect("read query request");
            if path == "/v1/embeddings" {
                let body = json!({
                    "object": "list",
                    "data": [{"object": "embedding", "index": 0, "embedding": [1.0, 0.0, 0.0, 0.0]}],
                    "model": "embed-test",
                    "usage": {"prompt_tokens": 1, "total_tokens": 1}
                })
                .to_string();
                write_http_headers(&mut stream, "application/json", body.len())
                    .expect("embedding headers");
                stream
                    .write_all(body.as_bytes())
                    .expect("embedding response");
                stream.flush().expect("embedding flush");
                continue;
            }
            assert_eq!(path, "/v1/chat/completions");
            let first = concat!(
                "data: {\"id\":\"delayed-1\",\"model\":\"gpt-test\",\"choices\":[{\"index\":0,",
                "\"delta\":{\"content\":\"Basic \"},\"finish_reason\":null}]}\n\n"
            );
            let rest = concat!(
                "data: {\"id\":\"delayed-2\",\"model\":\"gpt-test\",\"choices\":[{\"index\":0,\"\
                 delta\":{\"content\":\"answer.\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n"
            );
            write_http_headers(
                &mut stream,
                "text/event-stream",
                first.len().saturating_add(rest.len()),
            )
            .expect("completion headers");
            stream
                .write_all(first.as_bytes())
                .expect("first completion chunk");
            stream.flush().expect("first completion flush");
            first_chunk_tx.send(()).expect("first chunk signal");
            release_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("completion release");
            stream
                .write_all(rest.as_bytes())
                .expect("remaining completion chunks");
            stream.flush().expect("completion flush");
        }
    });
    (
        format!("http://{address}"),
        first_chunk_rx,
        release_tx,
        thread,
    )
}

fn read_http_request_path(stream: &mut TcpStream) -> std::io::Result<String> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4_096];
    let header_end = loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "HTTP request ended before headers",
            ));
        }
        request.extend_from_slice(&buffer[..read]);
        if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break index.saturating_add(4);
        }
    };
    let headers = std::str::from_utf8(&request[..header_end])
        .map_err(|source| std::io::Error::new(std::io::ErrorKind::InvalidData, source))?;
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or_default();
    let path = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .map(str::to_owned)
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid HTTP request line")
        })?;
    let target_length = header_end.saturating_add(content_length);
    while request.len() < target_length {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "HTTP request ended before body",
            ));
        }
        request.extend_from_slice(&buffer[..read]);
    }
    Ok(path)
}

fn write_http_headers(
    stream: &mut TcpStream,
    content_type: &str,
    content_length: usize,
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: \
         {content_length}\r\nConnection: close\r\n\r\n"
    )
}

async fn write_minimal_query_index(root: &std::path::Path) {
    let output = root.join("output");
    let provider = ParquetTableProvider::new(&output).expect("query table provider");
    let dataframe = DataFrame::new(
        1,
        vec![
            Series::new("id".into(), ["tu-1"]).into(),
            Series::new("text".into(), ["A source"]).into(),
            Series::new("n_tokens".into(), [2_u32]).into(),
            Series::new("document_id".into(), ["doc-1"]).into(),
            string_list_column("entity_ids", &[&[]]),
            string_list_column("relationship_ids", &[&[]]),
            string_list_column("covariate_ids", &[&[]]),
        ],
    )
    .expect("query text units");
    provider
        .write_dataframe("text_units", dataframe)
        .await
        .expect("write query text units");
    let settings = tokio::fs::read_to_string(root.join("settings.yaml"))
        .await
        .expect("settings");
    let mut config: GraphRagConfig = serde_yaml::from_str(&settings).expect("config");
    config.vector_store.db_uri = output.join("lancedb").display().to_string();
    let store = LanceDbVectorStore::connect(&config.vector_store)
        .await
        .expect("query LanceDB");
    let schema = config.vector_store.schema_for("text_unit_text");
    store
        .ensure_index(&schema)
        .await
        .expect("query vector index");
    store
        .upsert_documents(
            &schema,
            &[graphloom_vectors::VectorDocument {
                id: "tu-1".to_owned(),
                vector: vec![1.0, 0.0, 0.0, 0.0],
            }],
        )
        .await
        .expect("query vector");
}
