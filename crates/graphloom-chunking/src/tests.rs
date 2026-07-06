use std::{num::NonZeroUsize, sync::Arc};

use serde_json::json;

use super::{
    Chunker, ChunkingConfig, TokenDecode, TokenEncode, TokenOverlapChunker, add_metadata,
    prepend_metadata, unicode_scalar_decode, unicode_scalar_encode,
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
    assert_eq!(chunks[0].start_char, 0);
    assert_eq!(chunks[0].end_char, 3);
    assert_eq!(chunks[0].token_count, Some(4));
    assert_eq!(chunks[1].original, "defg");
    assert_eq!(chunks[1].start_char, 4);
    assert_eq!(chunks[2].original, "ghi");
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
