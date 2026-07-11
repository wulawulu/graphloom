//! Semantic text chunking backed by `text-splitter`.

use std::fmt;

use text_splitter::{ChunkConfig, TextSplitter};

use crate::{
    Chunker, ChunkingConfig, ChunkingError, Result, TextChunk, TextTransform,
    tiktoken::TiktokenCodec,
};

/// Text-structure-aware chunker backed by [`TextSplitter`].
///
/// This chunker does not use embeddings. In this context, "semantic" means
/// text-structure boundaries such as paragraphs, sentences, words, graphemes,
/// and characters. Overlap is delegated to `text-splitter`, so it is selected
/// around semantic boundaries and is not guaranteed to match the exact strict
/// token sliding-window behavior of [`crate::TokenOverlapChunker`].
pub struct SemanticTextChunker {
    config: ChunkingConfig,
    tokenizer: TiktokenCodec,
    splitter: TextSplitter<&'static tiktoken_rs::CoreBPE>,
}

impl SemanticTextChunker {
    /// Create a semantic text chunker from a validated config.
    ///
    /// # Errors
    ///
    /// Returns an error when config validation fails, the tokenizer cannot be
    /// initialized, or the `text-splitter` overlap configuration is invalid.
    pub fn new(config: ChunkingConfig) -> Result<Self> {
        config.validate()?;
        let tokenizer = TiktokenCodec::new(config.encoding_model.clone())?;
        let chunk_config = ChunkConfig::new(config.size.get())
            .with_sizer(tokenizer.bpe())
            .with_trim(false)
            .with_overlap(config.overlap)
            .map_err(|source| ChunkingError::InvalidConfig(source.to_string()))?;
        let splitter = TextSplitter::new(chunk_config);

        Ok(Self {
            config,
            tokenizer,
            splitter,
        })
    }
}

impl fmt::Debug for SemanticTextChunker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SemanticTextChunker")
            .field("config", &self.config)
            .field("tokenizer", &self.tokenizer)
            .finish_non_exhaustive()
    }
}

impl Chunker for SemanticTextChunker {
    fn chunk(&self, text: &str, transform: Option<&TextTransform>) -> Result<Vec<TextChunk>> {
        self.splitter
            .chunk_char_indices(text)
            .enumerate()
            .map(|(index, chunk)| {
                let original = chunk.chunk.to_owned();
                let text =
                    transform.map_or_else(|| original.clone(), |transform| transform(&original));
                let char_len = original.chars().count();
                let end_char = chunk.char_offset.saturating_add(char_len).saturating_sub(1);
                Ok(TextChunk {
                    original,
                    text: text.clone(),
                    index,
                    start_char: Some(chunk.char_offset),
                    end_char: Some(end_char),
                    start_token: None,
                    end_token: None,
                    token_count: Some(self.tokenizer.count(&text)),
                })
            })
            .collect()
    }
}
