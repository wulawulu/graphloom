//! Tokenizer abstraction and tiktoken implementation.

use tiktoken_rs::{CoreBPE, bpe_for_model, cl100k_base_singleton, o200k_base_singleton};

use crate::{LlmError, Result};

/// Tokenizer contract used by workflows.
pub trait Tokenizer: Send + Sync + std::fmt::Debug {
    /// Encode text into token ids.
    ///
    /// # Errors
    ///
    /// Returns an error when the text contains unsupported special-token usage.
    fn encode(&self, text: &str) -> Result<Vec<u32>>;

    /// Decode token ids into text.
    ///
    /// # Errors
    ///
    /// Returns an error when token ids cannot be decoded by the tokenizer.
    fn decode(&self, tokens: &[u32]) -> Result<String>;

    /// Count tokens in `text`.
    ///
    /// # Errors
    ///
    /// Returns an error when encoding fails.
    fn count(&self, text: &str) -> Result<usize> {
        self.encode(text).map(|tokens| tokens.len())
    }
}

/// `OpenAI` tiktoken-backed tokenizer.
#[derive(Clone)]
pub struct TiktokenTokenizer {
    encoding_model: String,
    bpe: &'static CoreBPE,
}

impl TiktokenTokenizer {
    /// Create a tokenizer for an `OpenAI` model or encoding name.
    ///
    /// # Errors
    ///
    /// Returns an error when the encoding model is unsupported.
    pub fn new(encoding_model: impl Into<String>) -> Result<Self> {
        let encoding_model = encoding_model.into();
        let bpe = match encoding_model.as_str() {
            "cl100k_base" => Ok(cl100k_base_singleton()),
            "o200k_base" => Ok(o200k_base_singleton()),
            model => bpe_for_model(model),
        }
        .map_err(|source| LlmError::Tokenizer {
            encoding_model: encoding_model.clone(),
            message: source.to_string(),
        })?;

        Ok(Self {
            encoding_model,
            bpe,
        })
    }
}

impl std::fmt::Debug for TiktokenTokenizer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TiktokenTokenizer")
            .field("encoding_model", &self.encoding_model)
            .finish_non_exhaustive()
    }
}

impl Tokenizer for TiktokenTokenizer {
    fn encode(&self, text: &str) -> Result<Vec<u32>> {
        let allowed = self.bpe.special_tokens();
        let (tokens, _) =
            self.bpe
                .encode(text, &allowed)
                .map_err(|source| LlmError::Tokenizer {
                    encoding_model: self.encoding_model.clone(),
                    message: source.to_string(),
                })?;
        Ok(tokens)
    }

    fn decode(&self, tokens: &[u32]) -> Result<String> {
        self.bpe
            .decode(tokens)
            .map_err(|source| LlmError::Tokenizer {
                encoding_model: self.encoding_model.clone(),
                message: source.to_string(),
            })
    }
}
