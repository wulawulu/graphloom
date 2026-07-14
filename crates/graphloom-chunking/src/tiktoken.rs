//! Local tiktoken adapter for chunking implementations.

use std::fmt;

use tiktoken_rs::{
    CoreBPE, bpe_for_model, cl100k_base_singleton, o200k_base_singleton, o200k_harmony_singleton,
    p50k_base_singleton, p50k_edit_singleton, r50k_base_singleton,
};

use crate::{ChunkingError, Result};

#[derive(Clone)]
pub(crate) struct TiktokenCodec {
    encoding_model: String,
    bpe: &'static CoreBPE,
}

impl TiktokenCodec {
    pub(crate) fn new(encoding_model: impl Into<String>) -> Result<Self> {
        let encoding_model = encoding_model.into();
        let bpe = bpe_for_encoding_or_model(&encoding_model).map_err(|source| {
            ChunkingError::Tokenizer {
                encoding_model: encoding_model.clone(),
                message: source.to_string(),
            }
        })?;

        Ok(Self {
            encoding_model,
            bpe,
        })
    }

    pub(crate) fn bpe(&self) -> &'static CoreBPE {
        self.bpe
    }

    pub(crate) fn encode(&self, text: &str) -> Vec<u32> {
        self.bpe.encode_ordinary_as(text)
    }

    pub(crate) fn decode(&self, tokens: &[u32]) -> Result<String> {
        let bytes = self
            .bpe
            .decode_bytes(tokens)
            .map_err(|source| ChunkingError::Tokenizer {
                encoding_model: self.encoding_model.clone(),
                message: source.to_string(),
            })?;

        // Arbitrary token slices need not be valid UTF-8. Python tiktoken defaults to
        // errors="replace", so use lossy decoding for GraphRAG compatibility.
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub(crate) fn count(&self, text: &str) -> usize {
        self.bpe.encode_ordinary(text).len()
    }
}

impl fmt::Debug for TiktokenCodec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TiktokenCodec")
            .field("encoding_model", &self.encoding_model)
            .finish_non_exhaustive()
    }
}

fn bpe_for_encoding_or_model(
    encoding_model: &str,
) -> std::result::Result<&'static CoreBPE, Box<dyn std::error::Error + Send + Sync>> {
    match encoding_model {
        "cl100k_base" => Ok(cl100k_base_singleton()),
        "o200k_base" => Ok(o200k_base_singleton()),
        "o200k_harmony" => Ok(o200k_harmony_singleton()),
        "p50k_base" => Ok(p50k_base_singleton()),
        "p50k_edit" => Ok(p50k_edit_singleton()),
        "r50k_base" => Ok(r50k_base_singleton()),
        model => bpe_for_model(model).map_err(Into::into),
    }
}

#[cfg(test)]
mod tests {
    use super::TiktokenCodec;

    const UNICODE_CANDIDATES: &[&str] = &[
        "🦀",
        "🙂",
        "𠮷",
        "龘",
        "你好世界",
        "中文与 emoji 🙂 混合文本",
        "GraphLoom 🦀 你好世界 🙂 𠮷 龘",
    ];

    #[test]
    fn test_should_round_trip_valid_utf8_token_sequences() {
        let codec = TiktokenCodec::new("o200k_base").expect("tokenizer should initialize");

        for text in ["Hello, GraphLoom", "你好，世界", "GraphLoom 🦀"] {
            let tokens = codec.encode(text);

            assert_eq!(
                codec.decode(&tokens).expect("valid UTF-8 should decode"),
                text
            );
        }
    }

    #[test]
    fn test_should_replace_invalid_utf8_in_token_subslice() {
        let codec = TiktokenCodec::new("o200k_base").expect("tokenizer should initialize");
        let (tokens, bytes) = find_invalid_utf8_token_slice(&codec);
        let expected = String::from_utf8_lossy(&bytes);

        assert!(expected.contains('\u{fffd}'));
        assert_eq!(
            codec
                .decode(&tokens)
                .expect("invalid UTF-8 should be replaced"),
            expected
        );
    }

    fn find_invalid_utf8_token_slice(codec: &TiktokenCodec) -> (Vec<u32>, Vec<u8>) {
        for candidate in UNICODE_CANDIDATES {
            let tokens = codec.encode(candidate);
            for start in 0..tokens.len() {
                for end in (start + 1)..=tokens.len() {
                    let slice = &tokens[start..end];
                    let bytes = codec
                        .bpe()
                        .decode_bytes(slice)
                        .expect("encoded token IDs should decode to bytes");
                    if std::str::from_utf8(&bytes).is_err() {
                        return (slice.to_vec(), bytes);
                    }
                }
            }
        }

        panic!("the deterministic Unicode corpus should contain an invalid UTF-8 token slice");
    }
}
