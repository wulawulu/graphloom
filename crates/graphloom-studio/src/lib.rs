//! Host-side service components for `GraphLoom` Studio.
//!
//! This crate depends on `graphloom` and owns browser-facing protocols. The core `GraphLoom`
//! library remains independent of Axum, HTTP, and Studio lifecycle concerns.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

pub mod explainability;
