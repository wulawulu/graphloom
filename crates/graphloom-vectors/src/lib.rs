//empty file
//! Vector store abstractions will live here in Step 9.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

/// Marker exported so downstream crates can depend on this crate before Step 9
/// fills in vector providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VectorsCrate;
