//! Shared filesystem path-safety semantics.

#[cfg(windows)]
use std::{
    ffi::OsStr,
    io,
    os::windows::{ffi::OsStrExt, fs::MetadataExt},
};
use std::{
    fs::Metadata,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

#[cfg(windows)]
use windows_sys::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};

use crate::{GraphLoomError, Result};

/// A path represented both lexically and through its nearest existing ancestor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedPath {
    pub(crate) lexical: PathBuf,
    pub(crate) resolved: PathBuf,
}

/// Policy applied while resolving existing path components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkPolicy {
    Reject,
    Follow,
}

/// Advances absolute-path traversal state and reports whether metadata may be queried.
pub(crate) fn component_reaches_queryable_path(
    component: Component<'_>,
    reached_root: &mut bool,
) -> bool {
    match component {
        Component::Prefix(_) => false,
        Component::RootDir => {
            *reached_root = true;
            true
        }
        _ => *reached_root,
    }
}

/// Returns whether metadata identifies a symlink or Windows reparse point.
#[cfg(not(windows))]
pub(crate) fn is_symlink_or_reparse(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

/// Returns whether metadata identifies a symlink or Windows reparse point.
#[cfg(windows)]
pub(crate) fn is_symlink_or_reparse(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink() || file_attributes_are_reparse(metadata.file_attributes())
}

#[cfg(windows)]
fn file_attributes_are_reparse(attributes: u32) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// Returns whether `path` equals or is contained by `parent`.
#[cfg(not(windows))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "the Windows implementation performs a fallible ordinal comparison"
)]
pub(crate) fn path_is_within_or_equal(path: &Path, parent: &Path) -> Result<bool> {
    Ok(path.starts_with(parent))
}

/// Return the original-casing suffix when `path` is a strict lexical descendant of `parent`.
///
/// Path components use the platform's containment semantics. In particular, Windows compares
/// components with ordinal case-insensitive comparison while retaining the casing from `path` in
/// the returned relative path. This function does not access the filesystem or follow links.
pub(crate) fn relative_descendant(path: &Path, parent: &Path) -> Result<Option<PathBuf>> {
    if !path_is_within_or_equal(path, parent)? {
        return Ok(None);
    }

    let parent_component_count = parent.components().count();
    let relative = path
        .components()
        .skip(parent_component_count)
        .map(|component| component.as_os_str())
        .collect::<PathBuf>();
    if relative.as_os_str().is_empty() {
        return Ok(None);
    }
    if relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        Ok(Some(relative))
    } else {
        Err(GraphLoomError::UnsafeOutputPath {
            path: path.to_path_buf(),
            message: format!(
                "relative descendant of {} contains a non-normal path component",
                parent.display()
            ),
        })
    }
}

/// Returns whether `path` equals or is contained by `parent`.
#[cfg(windows)]
pub(crate) fn path_is_within_or_equal(path: &Path, parent: &Path) -> Result<bool> {
    path_is_prefix_case_insensitive(path, parent).map_err(|source| GraphLoomError::Io {
        operation: "compare Windows path components",
        path: path.to_path_buf(),
        source,
    })
}

/// Returns whether either resolved path contains the other.
pub(crate) fn paths_overlap(left: &Path, right: &Path) -> Result<bool> {
    Ok(path_is_within_or_equal(left, right)? || path_is_within_or_equal(right, left)?)
}

/// Return an absolute, lexically normalized path without following links.
pub(crate) fn absolute_lexical(path: &Path) -> Result<PathBuf> {
    Ok(normalize_path(&absolute_unresolved(path)?))
}

/// Return an absolute path while preserving `.` and `..` components.
pub(crate) fn absolute_unresolved(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(|source| GraphLoomError::Io {
                operation: "get current directory",
                path: PathBuf::from("."),
                source,
            })?
            .join(path))
    }
}

/// Normalize `.` and `..` components without consulting the filesystem.
#[must_use]
pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// Resolve a destructive path while rejecting symlink or reparse-point components.
pub(crate) fn resolve_path_rejecting_links(path: &Path) -> Result<ResolvedPath> {
    resolve_path_with_existing_ancestor(path, LinkPolicy::Reject)
}

/// Resolve a comparison path by following existing symlink components.
pub(crate) fn resolve_path_following_links(path: &Path) -> Result<ResolvedPath> {
    resolve_path_with_existing_ancestor(path, LinkPolicy::Follow)
}

/// Reject an existing symlink/reparse point or non-directory ancestor.
pub(crate) async fn reject_symlink_ancestors(path: &Path) -> Result<()> {
    let path = absolute_unresolved(path)?;
    let mut current = PathBuf::new();
    let mut reached_root = false;
    for component in path.components() {
        current.push(component.as_os_str());
        if !component_reaches_queryable_path(component, &mut reached_root) {
            continue;
        }
        match tokio::fs::symlink_metadata(&current).await {
            Ok(metadata) if is_symlink_or_reparse(&metadata) => {
                return Err(GraphLoomError::InvalidRoot {
                    path: current,
                    message: "refusing to write through symlink parent".to_owned(),
                });
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(GraphLoomError::InvalidRoot {
                    path: current,
                    message: "path ancestor is not a directory".to_owned(),
                });
            }
            Ok(_) => {}
            Err(source) if source.kind() == ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(GraphLoomError::Io {
                    operation: "check parent symlink",
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(())
}

/// Reject existing symlink or reparse-point components below a controlled root.
///
/// Missing components are accepted so callers can validate destinations before creating their
/// parent directories. Callers must separately enforce that `relative` is a non-empty path made
/// only of normal components.
pub(crate) async fn reject_descendant_link_components(
    root: &Path,
    relative: &Path,
    operation: &'static str,
) -> Result<()> {
    let mut current = root.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        current.push(component.as_os_str());
        match tokio::fs::symlink_metadata(&current).await {
            Ok(metadata) if is_symlink_or_reparse(&metadata) => {
                return Err(GraphLoomError::UnsafePreservedDescendantPath {
                    operation,
                    root: root.to_path_buf(),
                    descendant: relative.to_path_buf(),
                    path: current,
                });
            }
            Ok(metadata) if components.peek().is_some() && !metadata.is_dir() => {
                return Err(GraphLoomError::Io {
                    operation: "inspect preserved descendant component",
                    path: current,
                    source: std::io::Error::new(
                        ErrorKind::NotADirectory,
                        "preserved descendant ancestor is not a directory",
                    ),
                });
            }
            Ok(_) => {}
            Err(source) if source.kind() == ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(GraphLoomError::Io {
                    operation: "inspect preserved descendant component",
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(())
}

fn resolve_path_with_existing_ancestor(
    path: &Path,
    link_policy: LinkPolicy,
) -> Result<ResolvedPath> {
    let lexical = absolute_lexical(path)?;
    let mut current = PathBuf::new();
    let mut existing = None;
    let mut reached_root = false;
    let mut components = lexical.components().peekable();
    while let Some(component) = components.next() {
        current.push(component.as_os_str());
        if !component_reaches_queryable_path(component, &mut reached_root) {
            continue;
        }
        let has_remaining = components.peek().is_some();
        match current.symlink_metadata() {
            Ok(metadata)
                if link_policy == LinkPolicy::Reject && is_symlink_or_reparse(&metadata) =>
            {
                return Err(GraphLoomError::UnsafeOutputPath {
                    path: path.to_path_buf(),
                    message: "path must not contain symlink components".to_owned(),
                });
            }
            Ok(metadata) => {
                if has_remaining && !path_component_is_directory(&current, &metadata, link_policy)?
                {
                    return Err(GraphLoomError::Io {
                        operation: "inspect path ancestor",
                        path: current,
                        source: std::io::Error::new(
                            ErrorKind::NotADirectory,
                            "path ancestor is not a directory",
                        ),
                    });
                }
                existing = Some(current.clone());
            }
            Err(source) if source.kind() == ErrorKind::NotFound => break,
            Err(source) => {
                return Err(GraphLoomError::Io {
                    operation: "inspect path",
                    path: current,
                    source,
                });
            }
        }
    }
    let existing = existing.unwrap_or_else(|| lexical.clone());
    let suffix = lexical
        .strip_prefix(&existing)
        .map_or_else(|_| PathBuf::new(), Path::to_path_buf);
    let resolved_ancestor = existing
        .canonicalize()
        .map_err(|source| GraphLoomError::Io {
            operation: "canonicalize path ancestor",
            path: existing.clone(),
            source,
        })?;
    Ok(ResolvedPath {
        lexical,
        resolved: normalize_path(&resolved_ancestor.join(suffix)),
    })
}

fn path_component_is_directory(
    path: &Path,
    metadata: &Metadata,
    link_policy: LinkPolicy,
) -> Result<bool> {
    if metadata.is_dir() {
        return Ok(true);
    }
    if link_policy == LinkPolicy::Follow && is_symlink_or_reparse(metadata) {
        return path
            .metadata()
            .map(|target| target.is_dir())
            .map_err(|source| GraphLoomError::Io {
                operation: "inspect resolved path ancestor",
                path: path.to_path_buf(),
                source,
            });
    }
    Ok(false)
}

#[cfg(windows)]
fn path_is_prefix_case_insensitive(path: &Path, prefix: &Path) -> io::Result<bool> {
    let mut path_components = path.components();
    for prefix_component in prefix.components() {
        let Some(path_component) = path_components.next() else {
            return Ok(false);
        };
        if !component_eq_ignore_case(path_component, prefix_component)? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(windows)]
fn component_eq_ignore_case(left: Component<'_>, right: Component<'_>) -> io::Result<bool> {
    os_str_eq_ignore_case(left.as_os_str(), right.as_os_str())
}

#[cfg(windows)]
fn os_str_eq_ignore_case(left: &OsStr, right: &OsStr) -> io::Result<bool> {
    let left = left.encode_wide().collect::<Vec<_>>();
    let right = right.encode_wide().collect::<Vec<_>>();
    let length_error = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows path component exceeds CompareStringOrdinal length limit",
        )
    };
    let left_len = i32::try_from(left.len()).map_err(|_| length_error())?;
    let right_len = i32::try_from(right.len()).map_err(|_| length_error())?;

    // SAFETY: The vectors own both buffers for the duration of the call, their
    // validated lengths describe the accessible UTF-16 elements, and the API
    // neither mutates nor retains either pointer.
    let result =
        unsafe { CompareStringOrdinal(left.as_ptr(), left_len, right.as_ptr(), right_len, 1) };
    classify_compare_string_ordinal_result(result).ok_or_else(compare_string_ordinal_error)
}

#[cfg(windows)]
fn compare_string_ordinal_error() -> io::Error {
    let source = io::Error::last_os_error();
    if source.raw_os_error().is_some_and(|code| code != 0) {
        source
    } else {
        io::Error::other("CompareStringOrdinal failed")
    }
}

#[cfg(windows)]
fn classify_compare_string_ordinal_result(result: i32) -> Option<bool> {
    match result {
        0 => None,
        CSTR_EQUAL => Some(true),
        _ => Some(false),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::Path;

    use super::{
        component_reaches_queryable_path, paths_overlap, reject_descendant_link_components,
        relative_descendant,
    };

    #[tokio::test]
    async fn test_should_accept_normal_descendant_components() {
        let tempdir = tempfile::TempDir::new().expect("tempdir");
        let root = tempdir.path().join("root");
        tokio::fs::create_dir_all(root.join("vectors").join("lancedb"))
            .await
            .expect("directories");

        reject_descendant_link_components(
            &root,
            Path::new("vectors/lancedb"),
            "test preserved path",
        )
        .await
        .expect("normal components");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_should_reject_symlink_descendant_component() {
        use std::os::unix::fs::symlink;

        let tempdir = tempfile::TempDir::new().expect("tempdir");
        let root = tempdir.path().join("root");
        let external = tempdir.path().join("external");
        tokio::fs::create_dir_all(&root).await.expect("root");
        tokio::fs::create_dir_all(external.join("lancedb"))
            .await
            .expect("external");
        symlink(&external, root.join("vectors")).expect("symlink");

        let error = reject_descendant_link_components(
            &root,
            Path::new("vectors/lancedb"),
            "test preserved path",
        )
        .await
        .expect_err("symlink must fail");

        assert!(error.to_string().contains("symlink or reparse point"));
        assert!(error.to_string().contains("vectors/lancedb"));
    }

    #[cfg(not(windows))]
    #[test]
    fn test_should_return_relative_descendant() {
        assert_eq!(
            relative_descendant(
                Path::new("/project/output/lancedb/data"),
                Path::new("/project/output"),
            )
            .expect("comparison"),
            Some(Path::new("lancedb/data").to_path_buf())
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn test_should_not_treat_equal_or_similar_path_as_descendant() {
        assert_eq!(
            relative_descendant(Path::new("/project/output"), Path::new("/project/output"))
                .expect("comparison"),
            None
        );
        assert_eq!(
            relative_descendant(
                Path::new("/project/output-old/lancedb"),
                Path::new("/project/output"),
            )
            .expect("comparison"),
            None
        );
    }

    #[test]
    fn test_should_detect_overlapping_paths() {
        assert!(paths_overlap(Path::new("/a"), Path::new("/a")).expect("comparison"));
        assert!(paths_overlap(Path::new("/a/b"), Path::new("/a")).expect("comparison"));
        assert!(paths_overlap(Path::new("/a"), Path::new("/a/b")).expect("comparison"));
        assert!(!paths_overlap(Path::new("/a/b"), Path::new("/a/c")).expect("comparison"));
        assert!(!paths_overlap(Path::new("/a/b"), Path::new("/ab")).expect("comparison"));
    }

    #[test]
    fn test_should_query_metadata_from_root_component_on_absolute_path() {
        assert_queryable_states(Path::new("/var/tmp"), &[true, true, true]);
    }

    #[test]
    fn test_should_not_query_metadata_for_relative_path_components() {
        assert_queryable_states(Path::new("foo/bar"), &[false, false]);
    }

    fn assert_queryable_states(path: &Path, expected: &[bool]) {
        let mut reached_root = false;
        let states = path
            .components()
            .map(|component| component_reaches_queryable_path(component, &mut reached_root))
            .collect::<Vec<_>>();

        assert_eq!(states, expected);
    }

    #[cfg(windows)]
    pub(crate) mod windows {
        use std::{
            ffi::OsStr,
            path::{Component, Path, Prefix},
        };

        use windows_sys::Win32::Globalization::{CSTR_EQUAL, CSTR_GREATER_THAN, CSTR_LESS_THAN};

        use super::super::{
            classify_compare_string_ordinal_result, component_reaches_queryable_path,
            file_attributes_are_reparse, os_str_eq_ignore_case, paths_overlap, relative_descendant,
        };

        pub(crate) fn assert_windows_verbatim_path(path: &Path) {
            let Some(Component::Prefix(prefix)) = path.components().next() else {
                panic!("expected Windows path prefix: {}", path.display());
            };
            assert!(
                matches!(
                    prefix.kind(),
                    Prefix::Verbatim(_) | Prefix::VerbatimUNC(_, _) | Prefix::VerbatimDisk(_)
                ),
                "test must use a verbatim path: {}",
                path.display(),
            );
        }

        #[test]
        fn test_should_skip_verbatim_disk_prefix_until_root_component() {
            assert_queryable_states(Path::new(r"\\?\C:\Users"));
        }

        #[test]
        fn test_should_skip_normal_disk_prefix_until_root_component() {
            assert_queryable_states(Path::new(r"C:\Users"));
        }

        fn assert_queryable_states(path: &Path) {
            let mut reached_root = false;
            let states = path
                .components()
                .map(|component| component_reaches_queryable_path(component, &mut reached_root))
                .collect::<Vec<_>>();

            assert_eq!(states, vec![false, true, true]);
        }

        #[test]
        fn test_should_compare_windows_paths_by_component_case_insensitively() {
            assert!(
                paths_overlap(
                    Path::new(r"C:\Project\Input"),
                    Path::new(r"c:\project\input\Generated"),
                )
                .expect("comparison")
            );
            assert!(
                !paths_overlap(
                    Path::new(r"C:\Project\Input"),
                    Path::new(r"c:\project\input-Other"),
                )
                .expect("comparison")
            );
        }

        #[test]
        fn test_should_return_relative_descendant_case_insensitively() {
            assert_eq!(
                relative_descendant(
                    Path::new(r"c:\project\output\LanceDB\data"),
                    Path::new(r"C:\Project\Output"),
                )
                .expect("comparison"),
                Some(Path::new(r"LanceDB\data").to_path_buf())
            );
        }

        #[test]
        fn test_should_not_match_similar_windows_prefix() {
            assert_eq!(
                relative_descendant(
                    Path::new(r"C:\Project\OutputOld\lancedb"),
                    Path::new(r"C:\Project\Output"),
                )
                .expect("comparison"),
                None
            );
        }

        #[test]
        fn test_should_compare_windows_strings_using_ordinal_ignore_case() {
            assert!(
                os_str_eq_ignore_case(OsStr::new("Input"), OsStr::new("input"))
                    .expect("comparison")
            );
            assert!(
                os_str_eq_ignore_case(OsStr::new("ABC"), OsStr::new("abc")).expect("comparison")
            );
            assert!(os_str_eq_ignore_case(OsStr::new("Ä"), OsStr::new("ä")).expect("comparison"));
            assert!(
                !os_str_eq_ignore_case(OsStr::new("Input-A"), OsStr::new("input-b"))
                    .expect("comparison")
            );
        }

        #[test]
        fn test_should_classify_windows_reparse_file_attributes() {
            assert!(file_attributes_are_reparse(0x400));
            assert!(file_attributes_are_reparse(0x400 | 0x10));
            assert!(!file_attributes_are_reparse(0x10));
        }

        #[test]
        fn test_should_classify_compare_string_ordinal_failure_as_failure() {
            assert_eq!(classify_compare_string_ordinal_result(0), None);
            assert_eq!(
                classify_compare_string_ordinal_result(CSTR_EQUAL),
                Some(true)
            );
            assert_eq!(
                classify_compare_string_ordinal_result(CSTR_LESS_THAN),
                Some(false)
            );
            assert_eq!(
                classify_compare_string_ordinal_result(CSTR_GREATER_THAN),
                Some(false)
            );
        }
    }
}
