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
        self.bpe
            .decode(tokens)
            .map_err(|source| ChunkingError::Tokenizer {
                encoding_model: self.encoding_model.clone(),
                message: source.to_string(),
            })
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
