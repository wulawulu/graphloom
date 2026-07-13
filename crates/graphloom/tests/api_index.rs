use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use graphloom::{
    ALL_EMBEDDINGS, GraphRagConfig, IndexRunStats, IndexWorkflowCallbacks,
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
async fn test_should_preserve_inactive_vector_store_inside_output() {
    let tempdir = TempDir::new().expect("tempdir");
    let input = tempdir.path().join("input");
    let output = tempdir.path().join("output");
    tokio::fs::create_dir_all(&input).await.expect("input dir");
    tokio::fs::create_dir_all(output.join("lancedb"))
        .await
        .expect("vector dir");
    tokio::fs::write(input.join("doc.txt"), "alpha beta gamma")
        .await
        .expect("input document");
    tokio::fs::write(output.join("old-sentinel"), "replace me")
        .await
        .expect("output sentinel");
    tokio::fs::write(output.join("lancedb").join("vector-marker"), "preserve me")
        .await
        .expect("vector marker");
    let mut config = GraphRagConfig::default();
    config.workflows = vec![
        "load_input_documents".to_owned(),
        "create_base_text_units".to_owned(),
    ];

    build_index(
        config,
        BuildIndexOptions {
            project_root: tempdir.path().to_path_buf(),
            method: IndexingMethod::Standard,
            cache_mode: CacheMode::Disabled,
            callbacks: Vec::new(),
        },
    )
    .await
    .expect("partial index should succeed");

    let provider = ParquetTableProvider::new(&output).expect("output provider");
    assert!(provider.has("documents").await.expect("documents table"));
    assert!(provider.has("text_units").await.expect("text units table"));
    assert!(
        !tokio::fs::try_exists(output.join("old-sentinel"))
            .await
            .expect("sentinel lookup")
    );
    assert_eq!(
        tokio::fs::read_to_string(output.join("lancedb").join("vector-marker"))
            .await
            .expect("vector marker"),
        "preserve me"
    );
    assert_no_generation_residue(tempdir.path()).await;
}

#[tokio::test]
async fn test_should_leave_inactive_external_vector_store_untouched() {
    let tempdir = TempDir::new().expect("tempdir");
    let input = tempdir.path().join("input");
    let external_vector = tempdir.path().join("vector-db");
    tokio::fs::create_dir_all(&input).await.expect("input dir");
    tokio::fs::create_dir_all(&external_vector)
        .await
        .expect("vector dir");
    tokio::fs::write(input.join("doc.txt"), "alpha beta")
        .await
        .expect("input document");
    tokio::fs::write(external_vector.join("vector-marker"), "unchanged")
        .await
        .expect("vector marker");
    let mut config = GraphRagConfig::default();
    config.workflows = vec![
        "load_input_documents".to_owned(),
        "create_base_text_units".to_owned(),
    ];
    config.vector_store.db_uri = "vector-db".to_owned();

    build_index(
        config,
        BuildIndexOptions {
            project_root: tempdir.path().to_path_buf(),
            method: IndexingMethod::Standard,
            cache_mode: CacheMode::Disabled,
            callbacks: Vec::new(),
        },
    )
    .await
    .expect("partial index should succeed");

    assert_eq!(
        tokio::fs::read_to_string(external_vector.join("vector-marker"))
            .await
            .expect("vector marker"),
        "unchanged"
    );
    assert_no_generation_residue(tempdir.path()).await;
}

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
    tokio::fs::create_dir_all(tempdir.path().join("output").join("lancedb"))
        .await
        .expect("old vector dir");
    tokio::fs::write(
        tempdir
            .path()
            .join("output")
            .join("lancedb")
            .join("old-vector-marker"),
        "old",
    )
    .await
    .expect("old vector marker");
    let mut config = test_config(&server.uri());
    let mut custom_schema = graphloom_vectors::VectorIndexSchema::default();
    custom_schema.index_name = "custom_entity_descriptions".to_owned();
    custom_schema.vector_size = 4;
    config
        .vector_store
        .index_schema
        .insert("entity_description".to_owned(), custom_schema);
    let callbacks = Arc::new(RecordingCallbacks::default());

    let result = build_index(
        config.clone(),
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
    let requests = server.received_requests().await.expect("requests");
    assert_eq!(
        requests
            .iter()
            .filter(|request| is_completion_connectivity_request(request))
            .count(),
        1,
        "public API should run completion connectivity once",
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| is_embedding_connectivity_request(request))
            .count(),
        1,
        "public API should run embedding connectivity once",
    );
    assert_standard_outputs(tempdir.path(), &config).await;
    assert!(
        !tokio::fs::try_exists(
            tempdir
                .path()
                .join("output")
                .join("lancedb")
                .join("old-vector-marker")
        )
        .await
        .expect("old vector marker lookup")
    );
    let mut vector_config = config.vector_store;
    vector_config.db_uri = tempdir
        .path()
        .join("output")
        .join("lancedb")
        .to_string_lossy()
        .to_string();
    let store = LanceDbVectorStore::connect(&vector_config)
        .await
        .expect("lancedb");
    let custom_schema = vector_config.schema_for("entity_description");
    assert_eq!(custom_schema.index_name, "custom_entity_descriptions");
    assert!(store.count(&custom_schema).await.expect("custom count") > 0);
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
async fn test_should_preserve_active_index_when_connectivity_preflight_fails() {
    let server = MockServer::start().await;
    let tempdir = TempDir::new().expect("tempdir");
    let mut config = test_config(&server.uri());
    prepare_old_output_and_vector(tempdir.path(), &mut config).await;

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
    .expect_err("unconfigured mock server should fail during preflight");

    assert!(error.to_string().contains("completion connectivity check"));
    assert!(tempdir.path().join("output").join("sentinel.txt").is_file());
    assert_old_vector_still_exists(tempdir.path(), &config).await;
    assert_no_generation_residue(tempdir.path()).await;
}

#[tokio::test]
async fn test_should_reject_existing_output_file_before_model_connectivity_or_generation() {
    let server = MockServer::start().await;
    let tempdir = TempDir::new().expect("tempdir");
    tokio::fs::create_dir(tempdir.path().join("input"))
        .await
        .expect("input directory");
    tokio::fs::write(tempdir.path().join("input/document.txt"), "Alice")
        .await
        .expect("input document");
    let output = tempdir.path().join("output");
    tokio::fs::write(&output, "preserve output file")
        .await
        .expect("output file");

    let error = build_index(
        test_config(&server.uri()),
        BuildIndexOptions {
            project_root: tempdir.path().to_path_buf(),
            method: IndexingMethod::Standard,
            cache_mode: CacheMode::Configured,
            callbacks: Vec::new(),
        },
    )
    .await
    .expect_err("output file must fail validation");

    assert!(error.to_string().contains("output publication"));
    assert!(error.to_string().contains("not a directory"));
    assert!(output.is_file());
    assert_eq!(
        tokio::fs::read_to_string(&output)
            .await
            .expect("output file contents"),
        "preserve output file"
    );
    assert!(
        server
            .received_requests()
            .await
            .expect("requests")
            .is_empty()
    );
    assert!(!tempdir.path().join("cache").exists());
    assert!(!tempdir.path().join("logs").exists());
    assert_no_validation_probes(tempdir.path()).await;
    assert_no_generation_residue(tempdir.path()).await;
}

#[tokio::test]
async fn test_should_reject_existing_external_vector_file_before_connectivity_or_generation() {
    let server = MockServer::start().await;
    let tempdir = TempDir::new().expect("tempdir");
    tokio::fs::create_dir(tempdir.path().join("input"))
        .await
        .expect("input directory");
    tokio::fs::write(tempdir.path().join("input/document.txt"), "Alice")
        .await
        .expect("input document");
    let output = tempdir.path().join("output");
    tokio::fs::create_dir(&output)
        .await
        .expect("output directory");
    let output_sentinel = output.join("sentinel.txt");
    tokio::fs::write(&output_sentinel, "preserve output")
        .await
        .expect("output sentinel");
    let vector = tempdir.path().join("vector-db");
    tokio::fs::write(&vector, "preserve vector file")
        .await
        .expect("vector file");
    let mut config = test_config(&server.uri());
    config.vector_store.db_uri = vector.to_string_lossy().into_owned();

    let error = build_index(
        config,
        BuildIndexOptions {
            project_root: tempdir.path().to_path_buf(),
            method: IndexingMethod::Standard,
            cache_mode: CacheMode::Configured,
            callbacks: Vec::new(),
        },
    )
    .await
    .expect_err("external vector file must fail validation");

    assert!(error.to_string().contains("vector DB publication"));
    assert!(error.to_string().contains("not a directory"));
    assert_eq!(
        tokio::fs::read_to_string(&vector)
            .await
            .expect("vector file contents"),
        "preserve vector file"
    );
    assert_eq!(
        tokio::fs::read_to_string(&output_sentinel)
            .await
            .expect("output sentinel contents"),
        "preserve output"
    );
    assert!(
        server
            .received_requests()
            .await
            .expect("requests")
            .is_empty()
    );
    assert!(!tempdir.path().join("cache").exists());
    assert!(!tempdir.path().join("logs").exists());
    assert_no_validation_probes(tempdir.path()).await;
    assert_no_generation_residue(tempdir.path()).await;
}

#[tokio::test]
async fn test_should_publish_output_and_external_vector_generation_together() {
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
    let mut config = test_config(&server.uri());
    let external_vector = tempdir.path().join("vector-db");
    prepare_old_output_and_vector_at(tempdir.path(), &mut config, &external_vector).await;

    build_index(
        config.clone(),
        BuildIndexOptions {
            project_root: tempdir.path().to_path_buf(),
            method: IndexingMethod::Standard,
            cache_mode: CacheMode::Configured,
            callbacks: Vec::new(),
        },
    )
    .await
    .expect("external-vector index should publish");

    assert!(!tempdir.path().join("output").join("sentinel.txt").exists());
    let provider = ParquetTableProvider::new(tempdir.path().join("output")).expect("provider");
    assert!(provider.has("documents").await.expect("documents"));
    let store = LanceDbVectorStore::connect(&config.vector_store)
        .await
        .expect("external vector store");
    for embedding in ALL_EMBEDDINGS {
        let schema = config.vector_store.schema_for(embedding);
        assert!(store.count(&schema).await.expect("vector count") > 0);
    }
    let entity_schema = config.vector_store.schema_for("entity_description");
    assert!(
        store
            .get_by_id(&entity_schema, "old-id")
            .await
            .expect("old vector lookup")
            .is_none(),
    );
    assert_no_generation_residue(tempdir.path()).await;
}

#[tokio::test]
async fn test_should_reject_output_overlapping_input_before_reset() {
    let server = MockServer::start().await;
    let tempdir = TempDir::new().expect("tempdir");
    let mut config = test_config(&server.uri());
    let old_vector = tempdir.path().join("old-vector");
    prepare_old_output_and_vector_at(tempdir.path(), &mut config, &old_vector).await;
    let generated = tempdir.path().join("input").join("generated");
    tokio::fs::create_dir_all(&generated)
        .await
        .expect("generated input dir");
    tokio::fs::write(generated.join("sentinel.txt"), "keep")
        .await
        .expect("sentinel");
    "input/generated".clone_into(&mut config.output_storage.base_dir);
    "input/generated/lancedb".clone_into(&mut config.vector_store.db_uri);

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
    .expect_err("output overlapping input should fail");

    assert!(error.to_string().contains("overlap input"));
    assert!(generated.join("sentinel.txt").is_file());
    assert!(tempdir.path().join("input/document.txt").is_file());
    let mut old_config = config;
    old_config.vector_store.db_uri = old_vector.to_string_lossy().to_string();
    assert_old_vector_exists(&old_config).await;
    assert!(
        server
            .received_requests()
            .await
            .expect("requests")
            .is_empty()
    );
    assert_no_validation_probes(tempdir.path()).await;
}

#[cfg(unix)]
#[tokio::test]
async fn test_should_index_from_symlinked_input_directory() {
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
    let external_input = TempDir::new().expect("external input");
    let document = external_input.path().join("document.txt");
    tokio::fs::write(&document, "Alice works for Acme.")
        .await
        .expect("input");
    std::os::unix::fs::symlink(external_input.path(), tempdir.path().join("input"))
        .expect("input symlink");
    let config = test_config(&server.uri());

    let result = build_index(
        config.clone(),
        BuildIndexOptions {
            project_root: tempdir.path().to_path_buf(),
            method: IndexingMethod::Standard,
            cache_mode: CacheMode::Configured,
            callbacks: Vec::new(),
        },
    )
    .await
    .expect("index symlinked input");

    assert!(result.stats.document_count > 0);
    assert_standard_outputs(tempdir.path(), &config).await;
    assert!(document.is_file());
    assert!(!external_input.path().join("output").exists());
    assert!(!external_input.path().join("lancedb").exists());
    assert_no_validation_probes(tempdir.path()).await;
    assert_no_validation_probes(external_input.path()).await;
}

#[tokio::test]
async fn test_should_preflight_runtime_failures_before_destructive_reset() {
    for case in [
        RuntimePreflightCase::InvalidVectorSize,
        RuntimePreflightCase::InvalidVectorIndexName,
        RuntimePreflightCase::UnwritableVectorParent,
        RuntimePreflightCase::OutputParentIsFile,
        RuntimePreflightCase::CachePathIsFile,
        RuntimePreflightCase::InputPathIsFile,
        RuntimePreflightCase::InvalidCompletionEncoding,
        RuntimePreflightCase::InvalidEmbeddingEncoding,
    ] {
        let tempdir = TempDir::new().expect("tempdir");
        let mut config = test_config("http://127.0.0.1:1");
        prepare_old_output_and_vector(tempdir.path(), &mut config).await;
        let old_config = config.clone();
        apply_runtime_preflight_case(tempdir.path(), &mut config, case).await;

        let error = build_index(
            config,
            BuildIndexOptions {
                project_root: tempdir.path().to_path_buf(),
                method: IndexingMethod::Standard,
                cache_mode: CacheMode::Configured,
                callbacks: Vec::new(),
            },
        )
        .await
        .expect_err("runtime preflight case should fail");

        assert!(
            !error.to_string().is_empty(),
            "case {case:?} should return a useful error"
        );
        assert!(
            tempdir.path().join("output").join("sentinel.txt").is_file(),
            "case {case:?} must not clear old output"
        );
        assert_old_vector_still_exists(tempdir.path(), &old_config).await;
        assert_no_validation_probes(tempdir.path()).await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn test_should_probe_symlink_cache_and_reporting_targets_without_residue() {
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
    let external_cache = TempDir::new().expect("external cache");
    let external_logs = TempDir::new().expect("external logs");
    tokio::fs::create_dir(tempdir.path().join("input"))
        .await
        .expect("input dir");
    tokio::fs::write(
        tempdir.path().join("input").join("document.txt"),
        "Alice works for Acme.",
    )
    .await
    .expect("input");
    std::os::unix::fs::symlink(external_cache.path(), tempdir.path().join("cache"))
        .expect("cache symlink");
    std::os::unix::fs::symlink(external_logs.path(), tempdir.path().join("logs"))
        .expect("logs symlink");

    build_index(
        test_config(&server.uri()),
        BuildIndexOptions {
            project_root: tempdir.path().to_path_buf(),
            method: IndexingMethod::Standard,
            cache_mode: CacheMode::Configured,
            callbacks: Vec::new(),
        },
    )
    .await
    .expect("index with symlink cache and logs");

    assert_no_validation_probes(tempdir.path()).await;
    assert_no_validation_probes(external_cache.path()).await;
    assert_no_validation_probes(external_logs.path()).await;
}

#[tokio::test]
async fn test_should_drop_inside_output_lancedb_before_clearing_output() {
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
    let mut config = test_config(&server.uri());
    prepare_old_output_and_vector(tempdir.path(), &mut config).await;

    build_index(
        config.clone(),
        BuildIndexOptions {
            project_root: tempdir.path().to_path_buf(),
            method: IndexingMethod::Standard,
            cache_mode: CacheMode::Configured,
            callbacks: Vec::new(),
        },
    )
    .await
    .expect("inside-output index should succeed");

    assert!(!tempdir.path().join("output").join("sentinel.txt").exists());
    let store = LanceDbVectorStore::connect(&config.vector_store)
        .await
        .expect("reopen lancedb");
    for embedding in ALL_EMBEDDINGS {
        let schema = config.vector_store.schema_for(embedding);
        assert!(
            store.count(&schema).await.expect("managed table count") > 0,
            "{embedding} should be recreated and populated"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn test_should_reject_vector_child_symlink_before_destructive_reset() {
    let tempdir = TempDir::new().expect("tempdir");
    let external = TempDir::new().expect("external");
    let mut config = test_config("http://127.0.0.1:1");
    let external_db = external.path().join("lancedb");
    prepare_old_output_and_vector_at(tempdir.path(), &mut config, external_db.clone()).await;
    let old_config = config.clone();
    let vector_link = tempdir.path().join("output").join("lancedb");
    std::os::unix::fs::symlink(&external_db, &vector_link).expect("vector symlink");
    config.vector_store.db_uri = vector_link.to_string_lossy().to_string();

    let error = build_index(
        config,
        BuildIndexOptions {
            project_root: tempdir.path().to_path_buf(),
            method: IndexingMethod::Standard,
            cache_mode: CacheMode::Configured,
            callbacks: Vec::new(),
        },
    )
    .await
    .expect_err("vector symlink should fail before reset");

    assert!(error.to_string().contains("symlink"));
    assert!(tempdir.path().join("output").join("sentinel.txt").is_file());
    assert_old_vector_exists(&old_config).await;
}

#[cfg(unix)]
#[tokio::test]
async fn test_should_reject_vector_ancestor_symlink_before_destructive_reset() {
    let tempdir = TempDir::new().expect("tempdir");
    let external = TempDir::new().expect("external");
    let mut config = test_config("http://127.0.0.1:1");
    let external_db = external.path().join("db");
    prepare_old_output_and_vector_at(tempdir.path(), &mut config, external_db).await;
    let old_config = config.clone();
    let vector_link = tempdir.path().join("vector-link");
    std::os::unix::fs::symlink(external.path(), &vector_link).expect("vector ancestor symlink");
    config.vector_store.db_uri = vector_link.join("db").to_string_lossy().to_string();

    let error = build_index(
        config,
        BuildIndexOptions {
            project_root: tempdir.path().to_path_buf(),
            method: IndexingMethod::Standard,
            cache_mode: CacheMode::Configured,
            callbacks: Vec::new(),
        },
    )
    .await
    .expect_err("vector ancestor symlink should fail before reset");

    assert!(error.to_string().contains("symlink"));
    assert!(tempdir.path().join("output").join("sentinel.txt").is_file());
    assert_old_vector_exists(&old_config).await;
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

#[tokio::test]
async fn test_should_fail_embedding_dimension_mismatch_during_preflight() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(chat_responder)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(embedding_dimension_mismatch_responder)
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

    let error = build_index(
        test_config(&server.uri()),
        BuildIndexOptions {
            project_root: tempdir.path().to_path_buf(),
            method: IndexingMethod::Standard,
            cache_mode: CacheMode::Disabled,
            callbacks: Vec::new(),
        },
    )
    .await
    .expect_err("dimension mismatch should fail");
    let error_text = format!("{error:#}");

    assert!(error_text.contains("default_embedding_model"));
    assert!(error_text.contains("returned 3 dimensions"));
    assert!(error_text.contains("configured as 4"));
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

#[derive(Debug, Clone, Copy)]
enum RuntimePreflightCase {
    InvalidVectorSize,
    InvalidVectorIndexName,
    UnwritableVectorParent,
    OutputParentIsFile,
    CachePathIsFile,
    InputPathIsFile,
    InvalidCompletionEncoding,
    InvalidEmbeddingEncoding,
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

impl IndexWorkflowCallbacks for RecordingCallbacks {
    fn workflow_started(&self, workflow_name: &str) {
        self.started
            .lock()
            .expect("started lock")
            .push(workflow_name.to_owned());
    }

    fn workflow_completed(&self, workflow_name: &str, _stats: &IndexRunStats) {
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

async fn assert_standard_outputs(root: &std::path::Path, config: &GraphRagConfig) {
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

    let mut config = config.clone();
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
    let vector_db_uri = root.join("output").join("lancedb");
    prepare_old_output_and_vector_at(root, config, vector_db_uri).await;
}

async fn prepare_old_output_and_vector_at(
    root: &std::path::Path,
    config: &mut GraphRagConfig,
    vector_db_uri: impl Into<std::path::PathBuf>,
) {
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
    config.vector_store.db_uri = vector_db_uri.into().to_string_lossy().to_string();
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
    assert_old_vector_exists(&config).await;
}

async fn assert_old_vector_exists(config: &GraphRagConfig) {
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

async fn assert_no_validation_probes(root: &std::path::Path) {
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let Ok(metadata) = tokio::fs::symlink_metadata(&path).await else {
            continue;
        };
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.starts_with(".graphloom-write-probe-")
                    || name.starts_with(".graphloom-publication-probe-")
            })
        {
            panic!(
                "validation probe should not be left behind: {}",
                path.display()
            );
        }
        if metadata.is_dir() {
            let mut entries = tokio::fs::read_dir(&path).await.expect("read dir");
            while let Some(entry) = entries.next_entry().await.expect("dir entry") {
                pending.push(entry.path());
            }
        }
    }
}

async fn assert_no_generation_residue(root: &Path) {
    let mut entries = tokio::fs::read_dir(root).await.expect("project entries");
    while let Some(entry) = entries.next_entry().await.expect("project entry") {
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(
            !name.ends_with(".staging") && !name.ends_with(".backup"),
            "index transaction residue should be removed: {name}",
        );
    }
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

async fn apply_runtime_preflight_case(
    root: &Path,
    config: &mut GraphRagConfig,
    case: RuntimePreflightCase,
) {
    match case {
        RuntimePreflightCase::InvalidVectorSize => {
            config.vector_store.vector_size = 0;
        }
        RuntimePreflightCase::InvalidVectorIndexName => {
            let mut schema = graphloom_vectors::VectorIndexSchema::default();
            "bad-name".clone_into(&mut schema.index_name);
            schema.vector_size = 4;
            config
                .vector_store
                .index_schema
                .insert("entity_description".to_owned(), schema);
        }
        RuntimePreflightCase::UnwritableVectorParent => {
            let file = root.join("vector-parent-file");
            tokio::fs::write(&file, "not a directory")
                .await
                .expect("vector parent file");
            config.vector_store.db_uri = file.join("db").to_string_lossy().to_string();
        }
        RuntimePreflightCase::OutputParentIsFile => {
            tokio::fs::write(root.join("output-parent-file"), "not a directory")
                .await
                .expect("output parent file");
            "output-parent-file/output".clone_into(&mut config.output_storage.base_dir);
        }
        RuntimePreflightCase::CachePathIsFile => {
            tokio::fs::write(root.join("cache"), "not a directory")
                .await
                .expect("cache file");
        }
        RuntimePreflightCase::InputPathIsFile => {
            tokio::fs::remove_dir_all(root.join("input"))
                .await
                .expect("remove input dir");
            tokio::fs::write(root.join("input"), "not a directory")
                .await
                .expect("input file");
        }
        RuntimePreflightCase::InvalidCompletionEncoding => {
            config
                .completion_models
                .get_mut("default_completion_model")
                .expect("completion model")
                .encoding_model = Some("definitely-not-an-encoding".to_owned());
        }
        RuntimePreflightCase::InvalidEmbeddingEncoding => {
            config
                .embedding_models
                .get_mut("default_embedding_model")
                .expect("embedding model")
                .encoding_model = Some("definitely-not-an-encoding".to_owned());
        }
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

fn embedding_dimension_mismatch_responder(request: &Request) -> ResponseTemplate {
    let body = request
        .body_json::<Value>()
        .expect("embedding request json");
    let inputs = body["input"].as_array().expect("input");
    let data = inputs
        .iter()
        .enumerate()
        .map(|(index, _)| {
            json!({
                "object": "embedding",
                "index": index,
                "embedding": [1.0, 0.0, 0.0]
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
