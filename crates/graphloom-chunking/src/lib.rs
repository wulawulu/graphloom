//empty file
//! Chunking abstractions will live here in Step 3.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

/// Marker exported so downstream crates can depend on this crate before Step 3
/// fills in chunking providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkingCrate;
