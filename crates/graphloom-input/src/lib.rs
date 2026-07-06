//! Input document readers for `GraphLoom`.
//!
//! The module layout mirrors Microsoft `GraphRAG`'s `graphrag-input` package:
//! text document data, generic reader behavior, text-file input, and hashing
//! helpers are kept separate so later CSV/JSON readers can be added without
//! turning the crate root into a mixed implementation file.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

mod document;
mod error;
mod hashing;
mod property;
mod reader;
mod text;

pub use document::TextDocument;
pub use error::{InputError, Result};
pub use hashing::gen_sha512_hash;
pub use reader::{DocumentStream, InputReader};
pub use text::{FileInputReader, TextFileReader};

#[cfg(test)]
mod tests;
