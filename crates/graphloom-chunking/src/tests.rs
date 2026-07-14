use std::{num::NonZeroUsize, sync::Arc};

use serde_json::json;

use super::{
    Chunker, ChunkerType, ChunkingConfig, ChunkingError, SemanticTextChunker, TokenDecode,
    TokenEncode, TokenOverlapChunker, add_metadata, create_chunker, prepend_metadata,
    split_text_on_tokens, unicode_scalar_decode, unicode_scalar_encode,
};

#[test]
fn test_should_round_trip_unicode_scalar_encoding() {
    let tokens = unicode_scalar_encode("a🦀").expect("encoding should work");

    assert_eq!(
        unicode_scalar_decode(&tokens).expect("decoding should work"),
        "a🦀"
    );
}

#[test]
fn test_should_chunk_with_token_overlap_and_graphrag_chunk_fields() {
    let encode: Arc<TokenEncode> = Arc::new(unicode_scalar_encode);
    let decode: Arc<TokenDecode> = Arc::new(unicode_scalar_decode);
    let chunker = TokenOverlapChunker::new(
        ChunkingConfig::new(NonZeroUsize::new(4).expect("nonzero"), 1, Vec::new())
            .expect("valid config"),
        encode,
        decode,
    )
    .expect("chunker should initialize");

    let chunks = chunker
        .chunk("abcdefghi", None)
        .expect("chunking should work");

    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].original, "abcd");
    assert_eq!(chunks[0].text, "abcd");
    assert_eq!(chunks[0].index, 0);
    assert_eq!(chunks[0].start_char, None);
    assert_eq!(chunks[0].end_char, None);
    assert_eq!(chunks[0].start_token, Some(0));
    assert_eq!(chunks[0].end_token, Some(3));
    assert_eq!(chunks[0].token_count, Some(4));
    assert_eq!(chunks[1].original, "defg");
    assert_eq!(chunks[1].start_char, None);
    assert_eq!(chunks[1].end_char, None);
    assert_eq!(chunks[1].start_token, Some(3));
    assert_eq!(chunks[1].end_token, Some(6));
    assert_eq!(chunks[2].original, "ghi");
    assert_eq!(chunks[2].start_token, Some(6));
    assert_eq!(chunks[2].end_token, Some(8));
}

#[test]
fn test_should_preserve_public_token_split_text_results() {
    let chunks = split_text_on_tokens(
        "abcdefghi",
        4,
        1,
        &unicode_scalar_encode,
        &unicode_scalar_decode,
    )
    .expect("chunking should work");

    assert_eq!(chunks, vec!["abcd", "defg", "ghi"]);
}

#[test]
fn test_should_return_no_token_chunks_for_empty_string() {
    let chunker = TokenOverlapChunker::new(
        ChunkingConfig::new(NonZeroUsize::new(4).expect("nonzero"), 1, Vec::new())
            .expect("valid config"),
        Arc::new(unicode_scalar_encode),
        Arc::new(unicode_scalar_decode),
    )
    .expect("chunker should initialize");

    assert!(
        chunker
            .chunk("", None)
            .expect("chunking should work")
            .is_empty()
    );
}

#[test]
fn test_should_default_chunker_type_to_token_overlap() {
    assert_eq!(ChunkerType::default(), ChunkerType::TokenOverlap);
    assert_eq!(
        ChunkingConfig::default().chunker_type,
        ChunkerType::TokenOverlap
    );
    assert_eq!(
        ChunkingConfig::new(NonZeroUsize::new(64).expect("nonzero"), 8, Vec::new())
            .expect("valid config")
            .chunker_type,
        ChunkerType::TokenOverlap
    );
}

#[test]
fn test_should_deserialize_old_config_as_token_overlap() {
    let config = serde_json::from_value::<ChunkingConfig>(json!({
        "encoding_model": "o200k_base",
        "size": 64,
        "overlap": 8,
        "prepend_metadata": ["title"],
    }))
    .expect("config should deserialize");

    assert_eq!(config.chunker_type, ChunkerType::TokenOverlap);
}

#[test]
fn test_should_deserialize_chunker_type_values() {
    let token_overlap = serde_yaml::from_str::<ChunkingConfig>(
        r"
chunker_type: token_overlap
encoding_model: o200k_base
size: 64
overlap: 8
prepend_metadata: []
",
    )
    .expect("token overlap config should deserialize");
    let semantic_text = serde_yaml::from_str::<ChunkingConfig>(
        r"
chunker_type: semantic_text
encoding_model: o200k_base
size: 64
overlap: 8
prepend_metadata: []
",
    )
    .expect("semantic text config should deserialize");

    assert_eq!(token_overlap.chunker_type, ChunkerType::TokenOverlap);
    assert_eq!(semantic_text.chunker_type, ChunkerType::SemanticText);
}

#[test]
fn test_should_serialize_chunker_type_as_snake_case() {
    assert_eq!(
        serde_json::to_value(ChunkerType::TokenOverlap).expect("serialize should work"),
        json!("token_overlap")
    );
    assert_eq!(
        serde_json::to_value(ChunkerType::SemanticText).expect("serialize should work"),
        json!("semantic_text")
    );
}

#[test]
fn test_should_create_token_overlap_chunker_from_config() {
    let config = ChunkingConfig {
        chunker_type: ChunkerType::TokenOverlap,
        encoding_model: "o200k_base".to_owned(),
        size: NonZeroUsize::new(3).expect("nonzero"),
        overlap: 1,
        prepend_metadata: Vec::new(),
    };
    let chunker = create_chunker(&config).expect("chunker should be created");
    let chunks = chunker
        .chunk("alpha beta gamma delta epsilon", None)
        .expect("chunking should work");

    assert!(chunks.len() > 1);
    assert_eq!(chunks[0].index, 0);
    assert!(chunks.iter().all(|chunk| chunk.token_count.is_some()));
}

#[test]
fn test_should_chunk_invalid_utf8_token_windows_through_production_tokenizer() {
    let text = "GraphLoom 🦀 你好世界 🙂 𠮷 龘";
    let bpe = tiktoken_rs::o200k_base_singleton();
    let tokens = bpe.encode_ordinary(text);
    let config = ChunkingConfig {
        chunker_type: ChunkerType::TokenOverlap,
        encoding_model: "o200k_base".to_owned(),
        size: NonZeroUsize::new(1).expect("nonzero"),
        overlap: 0,
        prepend_metadata: Vec::new(),
    };
    let chunker = create_chunker(&config).expect("chunker should be created");

    let chunks = chunker
        .chunk(text, None)
        .expect("invalid UTF-8 token windows should use replacement characters");

    assert_eq!(chunks.len(), tokens.len());
    assert!(
        chunks
            .iter()
            .any(|chunk| chunk.original.contains('\u{fffd}'))
    );
    for (index, (chunk, token)) in chunks.iter().zip(tokens).enumerate() {
        let bytes = bpe
            .decode_bytes(&[token])
            .expect("encoded token ID should decode to bytes");
        let expected = String::from_utf8_lossy(&bytes);

        assert_eq!(chunk.original, expected);
        assert_eq!(chunk.text, expected);
        assert_eq!(chunk.index, index);
        assert_eq!(chunk.start_char, None);
        assert_eq!(chunk.end_char, None);
        assert_eq!(chunk.start_token, Some(index));
        assert_eq!(chunk.end_token, Some(index));
        assert_eq!(
            chunk.token_count,
            Some(bpe.encode_ordinary(&expected).len())
        );
    }
}

#[test]
fn test_should_create_semantic_text_chunker_from_config() {
    let config = ChunkingConfig {
        chunker_type: ChunkerType::SemanticText,
        encoding_model: "o200k_base".to_owned(),
        size: NonZeroUsize::new(5).expect("nonzero"),
        overlap: 0,
        prepend_metadata: Vec::new(),
    };
    let chunker = create_chunker(&config).expect("chunker should be created");
    let chunks = chunker
        .chunk("First paragraph.\n\nSecond paragraph.", None)
        .expect("chunking should work");

    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].original, "First paragraph.\n\n");
    assert_eq!(chunks[1].original, "Second paragraph.");
}

#[test]
fn test_should_return_error_for_unsupported_encoding_model() {
    let config = ChunkingConfig {
        chunker_type: ChunkerType::SemanticText,
        encoding_model: "not-a-real-model".to_owned(),
        size: NonZeroUsize::new(16).expect("nonzero"),
        overlap: 0,
        prepend_metadata: Vec::new(),
    };
    let error = create_chunker(&config).expect_err("chunker creation should reject tokenizer");

    assert!(matches!(error, ChunkingError::Tokenizer { .. }));
    assert!(error.to_string().contains("not-a-real-model"));
}

#[test]
fn test_should_return_config_error_for_invalid_overlap() {
    let config = ChunkingConfig {
        chunker_type: ChunkerType::SemanticText,
        encoding_model: "o200k_base".to_owned(),
        size: NonZeroUsize::new(8).expect("nonzero"),
        overlap: 8,
        prepend_metadata: Vec::new(),
    };
    let error = create_chunker(&config).expect_err("chunker creation should reject config");

    assert!(matches!(error, ChunkingError::InvalidConfig(_)));
}

#[test]
fn test_should_semantically_split_on_paragraph_boundaries() {
    let chunker = semantic_chunker(5, 0);
    let text = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";
    let chunks = chunker.chunk(text, None).expect("chunking should work");

    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].original, "First paragraph.\n\n");
    assert_eq!(chunks[1].original, "Second paragraph.\n\n");
    assert_eq!(chunks[2].original, "Third paragraph.");
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.original.as_str())
            .collect::<String>(),
        text
    );
}

#[test]
fn test_should_fallback_when_semantic_unit_exceeds_capacity() {
    let chunker = semantic_chunker(2, 0);
    let chunks = chunker
        .chunk("supercalifragilisticexpialidocious", None)
        .expect("chunking should work");

    assert!(chunks.len() > 1);
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.original.as_str())
            .collect::<String>(),
        "supercalifragilisticexpialidocious"
    );
}

#[test]
fn test_should_support_semantic_overlap() {
    let without_overlap = semantic_chunker(5, 0)
        .chunk("one two three four five six seven", None)
        .expect("chunking should work");
    let with_overlap = semantic_chunker(5, 2)
        .chunk("one two three four five six seven", None)
        .expect("chunking should work");

    assert!(with_overlap.len() >= without_overlap.len());
    assert!(
        with_overlap
            .iter()
            .map(|chunk| chunk.original.chars().count())
            .sum::<usize>()
            > "one two three four five six seven".chars().count()
    );
}

#[test]
fn test_should_report_unicode_character_offsets() {
    let text = "你好🙂世界\n\n再见🙂世界";
    let chunker = semantic_chunker(4, 0);
    let chunks = chunker.chunk(text, None).expect("chunking should work");

    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].original, "你好🙂世界\n\n");
    assert_eq!(chunks[0].start_char, Some(0));
    assert_eq!(chunks[0].end_char, Some(6));
    assert_eq!(chunks[0].start_token, None);
    assert_eq!(chunks[0].end_token, None);
    assert_eq!(chunks[1].original, "再见🙂世界");
    assert_eq!(chunks[1].start_char, Some(7));
    assert_eq!(chunks[1].end_char, Some(11));
    assert_eq!(chunks[1].start_token, None);
    assert_eq!(chunks[1].end_token, None);
}

#[test]
fn test_should_transform_text_but_keep_original_and_count_final_tokens() {
    let chunker = semantic_chunker(8, 0);
    let transform = |text: &str| format!("title: Doc\n{text}");
    let chunks = chunker
        .chunk("alpha beta", Some(&transform))
        .expect("chunking should work");
    let transformed = "title: Doc\nalpha beta";

    assert_eq!(chunks[0].original, "alpha beta");
    assert_eq!(chunks[0].text, transformed);
    assert_eq!(
        chunks[0].token_count,
        Some(
            tiktoken_rs::o200k_base_singleton()
                .encode_ordinary(transformed)
                .len()
        )
    );
}

#[test]
fn test_should_return_no_chunks_for_empty_string() {
    let chunks = semantic_chunker(8, 0)
        .chunk("", None)
        .expect("chunking should work");

    assert!(chunks.is_empty());
}

#[test]
fn test_should_allow_token_overlap_and_semantic_text_to_differ() {
    let text = "Alpha sentence.\n\nBeta sentence.";
    let semantic = semantic_chunker(5, 0)
        .chunk(text, None)
        .expect("semantic chunking should work");

    let token_overlap = TokenOverlapChunker::new(
        ChunkingConfig::new(NonZeroUsize::new(5).expect("nonzero"), 0, Vec::new())
            .expect("valid config"),
        Arc::new(unicode_scalar_encode),
        Arc::new(unicode_scalar_decode),
    )
    .expect("token overlap chunker should initialize")
    .chunk(text, None)
    .expect("token overlap chunking should work");

    assert_ne!(semantic, token_overlap);
    assert_eq!(semantic[0].original, "Alpha sentence.\n\n");
    assert_eq!(token_overlap[0].original, "Alpha");
}

#[test]
fn test_should_prepend_metadata_in_input_order_with_single_separator() {
    let metadata = vec![
        ("title".to_owned(), json!("Doc")),
        ("source".to_owned(), json!("a.txt")),
    ];

    assert_eq!(
        prepend_metadata("body", &metadata),
        "title: Doc\nsource: a.txt\nbody"
    );
}

#[test]
fn test_should_count_tokens_after_transform() {
    let encode: Arc<TokenEncode> = Arc::new(unicode_scalar_encode);
    let decode: Arc<TokenDecode> = Arc::new(unicode_scalar_decode);
    let chunker = TokenOverlapChunker::new(
        ChunkingConfig::new(NonZeroUsize::new(4).expect("nonzero"), 1, Vec::new())
            .expect("valid config"),
        encode,
        decode,
    )
    .expect("chunker should initialize");
    let metadata = vec![("title".to_owned(), json!("Doc"))];
    let transform = add_metadata(&metadata, ": ", "\n", false);
    let transform_fn = move |text: &str| transform.transform(text);

    let chunks = chunker
        .chunk("abc", Some(&transform_fn))
        .expect("chunking should work");

    assert_eq!(chunks[0].original, "abc");
    assert_eq!(chunks[0].text, "title: Doc\nabc");
    assert_eq!(
        chunks[0].token_count,
        Some("title: Doc\nabc".chars().count())
    );
}

fn semantic_chunker(size: usize, overlap: usize) -> SemanticTextChunker {
    SemanticTextChunker::new(ChunkingConfig {
        chunker_type: ChunkerType::SemanticText,
        encoding_model: "o200k_base".to_owned(),
        size: NonZeroUsize::new(size).expect("nonzero"),
        overlap,
        prepend_metadata: Vec::new(),
    })
    .expect("semantic chunker should initialize")
}
