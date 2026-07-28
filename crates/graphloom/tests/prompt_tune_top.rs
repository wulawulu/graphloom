//! Deterministic GraphRAG prompt-tune Top selection compatibility.

use std::path::PathBuf;

use graphloom::{
    GraphRagConfig,
    api::{DocSelectionType, GenerateIndexingPromptsOptions},
    prompt_tune::load_and_chunk_docs,
};
use serde_json::Value;

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
