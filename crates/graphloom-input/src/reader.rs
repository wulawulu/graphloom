//! Generic input reader trait.

use std::pin::Pin;

use futures_core::Stream;

use crate::{Result, TextDocument};

/// Stream type returned by input readers.
pub type DocumentStream<'a> = Pin<Box<dyn Stream<Item = Result<TextDocument>> + Send + 'a>>;

/// Asynchronous text document reader.
pub trait InputReader: Send + Sync + std::fmt::Debug {
    /// Stream input documents.
    fn read_documents(&self) -> DocumentStream<'_>;
}
