//! Small primitives shared across `GraphLoom` crates.
//!
//! Domain-specific configuration, tracing, and project loading belong to the
//! crate that owns those policies. This crate intentionally contains only
//! dependency-free helpers used by more than one lower-level crate.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::ffi::OsStr;

/// Return `true` when a path component is acceptable for logical storage names.
#[must_use]
pub fn is_safe_path_component(component: &OsStr) -> bool {
    let Some(component) = component.to_str() else {
        return false;
    };

    !component.is_empty()
        && component != "."
        && component != ".."
        && !component.contains('\0')
        && !component.contains('/')
        && !component.contains('\\')
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::is_safe_path_component;

    #[test]
    fn test_should_accept_simple_storage_component() {
        assert!(is_safe_path_component(OsStr::new("documents_2026")));
    }

    #[test]
    fn test_should_reject_traversal_and_separators() {
        for component in ["", ".", "..", "a/b", "a\\b", "nul\0suffix"] {
            assert!(!is_safe_path_component(OsStr::new(component)));
        }
    }
}
