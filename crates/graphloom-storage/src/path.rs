use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
};

use crate::{Result, StorageError};

fn is_safe_path_component(component: &OsStr) -> bool {
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

pub(crate) fn validate_table_name(table_name: &str) -> Result<String> {
    if table_name.is_empty()
        || table_name.len() > 128
        || !table_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(StorageError::InvalidPath {
            path: table_name.to_owned(),
            reason: "table names must match [A-Za-z0-9_-]{1,128}",
        });
    }

    Ok(table_name.to_owned())
}

pub(crate) fn validate_logical_path(path: &str) -> Result<PathBuf> {
    if path.is_empty() {
        return Ok(PathBuf::new());
    }

    let path_ref = Path::new(path);
    if !path_ref.is_relative() {
        return Err(StorageError::InvalidPath {
            path: path.to_owned(),
            reason: "absolute paths are not allowed",
        });
    }

    let mut normalized = PathBuf::new();
    for component in path_ref.components() {
        match component {
            Component::Normal(name) if is_safe_path_component(name) => normalized.push(name),
            _ => {
                return Err(StorageError::InvalidPath {
                    path: path.to_owned(),
                    reason: "path traversal and special components are not allowed",
                });
            }
        }
    }

    Ok(normalized)
}

pub(crate) fn path_to_logical(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(name) => name.to_str().map(ToOwned::to_owned),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

pub(crate) fn strip_namespace(key: &str, namespace: &str) -> String {
    if namespace.is_empty() {
        key.to_owned()
    } else {
        key.strip_prefix(namespace)
            .and_then(|value| value.strip_prefix('/'))
            .unwrap_or(key)
            .to_owned()
    }
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
