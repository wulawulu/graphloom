//empty file
//! Public `GraphLoom` crate.
//!
//! Step 1 establishes the workspace-level API surface and dependency direction.
//! Later implementation steps add configuration, pipeline, workflow, query, and
//! update modules behind this top-level crate.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

pub use graphloom_common as common;
pub use graphloom_storage as storage;
