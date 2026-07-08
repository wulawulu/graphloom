//! Vector store contracts and `LanceDB` provider for `GraphLoom`.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

mod config;
mod error;
mod lancedb;
mod store;

pub use config::{VectorIndexSchema, VectorStoreConfig, VectorStoreType, validate_identifier};
pub use error::{Result, VectorError};
pub use lancedb::LanceDbVectorStore;
pub use store::{VectorDocument, VectorSearchResult, VectorStore, create_vector_store};
