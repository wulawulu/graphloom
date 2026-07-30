//! Deterministic GraphRAG prompt-tune Top selection compatibility.

use std::{num::NonZeroUsize, path::PathBuf};

use graphloom::{
    GraphLoomError, GraphRagConfig,
    api::{DocSelectionType, GenerateIndexingPromptsOptions},
    prompt_tune::load_and_chunk_docs,
};
use graphloom_chunking::create_chunker;
use serde_json::Value;
use tempfile::TempDir;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/compat/fixtures/prompt_tune/top")
}

#[tokio::test]
async fn test_should_match_prompt_tune_top_selection_graphrag_fixture() {
    let root = fixture_root();
    let settings = tokio::fs::read_to_string(root.join("settings.yaml"))
        .await
        .expect("read fixture settings");
    let config: GraphRagConfig = serde_yaml::from_str(&settings).expect("parse fixture settings");
    let selected: Value = serde_json::from_slice(
        &tokio::fs::read(root.join("typed/selected_chunks.json"))
            .await
            .expect("read selected chunks"),
    )
    .expect("parse selected chunks");
    let selected = selected.as_array().expect("selected chunks array");
    let options = GenerateIndexingPromptsOptions::new(&root)
        .with_selection_method(DocSelectionType::Top)
        .with_limit(3)
        .with_chunk_size(38)
        .with_overlap(0);

    let actual = load_and_chunk_docs(&config, &root, &options, None)
        .await
        .expect("load Top chunks");

    assert_eq!(actual.len(), selected.len());
    for (actual, expected) in actual.iter().zip(selected) {
        assert_eq!(
            actual.chunk_text.as_bytes(),
            expected["chunk_text"]
                .as_str()
                .expect("chunk text")
                .as_bytes()
        );
        assert_eq!(
            actual.chunk_ordinal,
            usize::try_from(expected["chunk_ordinal"].as_u64().expect("chunk ordinal"))
                .expect("usize chunk ordinal")
        );
        assert_eq!(
            actual.token_count,
            usize::try_from(
                expected["chunk_token_count"]
                    .as_u64()
                    .expect("chunk token count")
            )
            .expect("usize chunk token count")
        );
        let document_path = expected["document_path"].as_str().expect("document path");
        let expected_name = PathBuf::from(document_path)
            .file_name()
            .expect("document name")
            .to_string_lossy()
            .into_owned();
        assert_eq!(actual.document_id.as_ref(), expected_name);
    }
}

#[tokio::test]
async fn test_should_use_embedding_model_tokenizer_for_prompt_tune_chunks() {
    let fixture = fixture_root();
    let settings = tokio::fs::read_to_string(fixture.join("settings.yaml"))
        .await
        .expect("read fixture settings");
    let mut config: GraphRagConfig =
        serde_yaml::from_str(&settings).expect("parse fixture settings");
    config.chunking.encoding_model = "o200k_base".to_owned();
    config.chunking.size = NonZeroUsize::new(40).expect("non-zero chunk size");
    config.chunking.overlap = 0;

    let root = TempDir::new().expect("temporary project");
    let input_dir = root.path().join("input");
    tokio::fs::create_dir(&input_dir)
        .await
        .expect("create input directory");
    let input = "西门庆与潘金莲在清河县相遇。武松回乡后得知兄长武大郎身亡，决意查明真相。\
                 王婆从中牵线，郓哥则向武大郎透露消息。";
    tokio::fs::write(input_dir.join("sample.txt"), input)
        .await
        .expect("write Chinese input");

    let options = GenerateIndexingPromptsOptions::new(root.path())
        .with_selection_method(DocSelectionType::All);
    let actual = load_and_chunk_docs(&config, root.path(), &options, None)
        .await
        .expect("load prompt-tune chunks");

    let embedding_model = config
        .embedding_models
        .get(&config.embed_text.embedding_model_id)
        .expect("configured embedding model");
    assert_eq!(
        embedding_model.effective_tokenizer_encoding(),
        "cl100k_base"
    );
    let mut embedding_tokenizer_config = config.chunking.clone();
    embedding_tokenizer_config.encoding_model =
        embedding_model.effective_tokenizer_encoding().to_owned();
    let expected = create_chunker(&embedding_tokenizer_config)
        .expect("create embedding tokenizer chunker")
        .chunk(input, None)
        .expect("chunk with embedding tokenizer");
    let configured = create_chunker(&config.chunking)
        .expect("create configured chunker")
        .chunk(input, None)
        .expect("chunk with configured tokenizer");

    assert_ne!(
        expected.iter().map(|chunk| &chunk.text).collect::<Vec<_>>(),
        configured
            .iter()
            .map(|chunk| &chunk.text)
            .collect::<Vec<_>>(),
        "fixture must distinguish the embedding tokenizer from chunking.encoding_model"
    );
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!(actual.chunk_text.as_ref(), expected.text);
        assert_eq!(
            actual.token_count,
            expected.token_count.expect("token count")
        );
    }
}

#[tokio::test]
async fn test_should_require_embedding_model_tokenizer_for_top_selection() {
    let root = fixture_root();
    let settings = tokio::fs::read_to_string(root.join("settings.yaml"))
        .await
        .expect("read fixture settings");
    let mut config: GraphRagConfig =
        serde_yaml::from_str(&settings).expect("parse fixture settings");
    config.embedding_models.clear();
    let options = GenerateIndexingPromptsOptions::new(&root)
        .with_selection_method(DocSelectionType::Top)
        .with_limit(3);

    let error = load_and_chunk_docs(&config, &root, &options, None)
        .await
        .expect_err("missing embedding model must fail");

    assert!(matches!(
        error,
        GraphLoomError::InvalidModel { model_id, .. }
            if model_id == config.embed_text.embedding_model_id
    ));
}
