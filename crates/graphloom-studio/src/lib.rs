//! Host-side service components for `GraphLoom` Studio.
//!
//! This crate depends on `graphloom` and owns browser-facing protocols. The core `GraphLoom`
//! library remains independent of Axum, HTTP, and Studio lifecycle concerns.

// Public Query futures are deeply nested across all four method branches. The higher limit lets
// rustc prove their `Send` bound when the private runner erases the future behind `async_trait`.
#![recursion_limit = "256"]
#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

pub mod api;
pub mod explainability;
pub mod graph;
