//! Storage and table-provider contracts for `GraphLoom`.
//!
//! The traits in this crate are object-safe because the pipeline context must
//! hold provider instances behind `dyn` dispatch. Per AGENTS.md § Async &
//! Concurrency, that is the reason this crate uses `async-trait` instead of
//! native `async fn` in traits.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

mod error;
mod path;
pub mod storage;
pub mod table;

pub use error::{Result, StorageError};
pub use storage::{FileStorage, MemoryStorage, Storage};
pub use table::{MemoryTableProvider, ParquetTableProvider, Table, TableProvider};

#[cfg(test)]
mod tests;
