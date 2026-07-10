//! Shared filesystem path-safety semantics.

#[cfg(windows)]
use std::{
    ffi::OsStr,
    io,
    os::windows::{ffi::OsStrExt, fs::MetadataExt},
    path::Component,
};
use std::{fs::Metadata, path::Path};

#[cfg(windows)]
use windows_sys::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};

#[cfg(windows)]
use crate::GraphLoomError;
use crate::Result;

/// Returns whether metadata identifies a symlink or Windows reparse point.
#[cfg(not(windows))]
pub(crate) fn is_symlink_or_reparse(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

/// Returns whether metadata identifies a symlink or Windows reparse point.
#[cfg(windows)]
pub(crate) fn is_symlink_or_reparse(metadata: &Metadata) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// Returns whether `path` equals or is contained by `parent`.
#[cfg(not(windows))]
pub(crate) fn path_is_within_or_equal(path: &Path, parent: &Path) -> Result<bool> {
    Ok(path.starts_with(parent))
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
mod tests {
    use std::path::Path;

    use super::paths_overlap;

    #[test]
    fn test_should_detect_overlapping_paths() {
        assert!(paths_overlap(Path::new("/a"), Path::new("/a")).expect("comparison"));
        assert!(paths_overlap(Path::new("/a/b"), Path::new("/a")).expect("comparison"));
        assert!(paths_overlap(Path::new("/a"), Path::new("/a/b")).expect("comparison"));
        assert!(!paths_overlap(Path::new("/a/b"), Path::new("/a/c")).expect("comparison"));
        assert!(!paths_overlap(Path::new("/a/b"), Path::new("/ab")).expect("comparison"));
    }

    #[cfg(windows)]
    mod windows {
        use std::{ffi::OsStr, path::Path};

        use windows_sys::Win32::Globalization::{CSTR_EQUAL, CSTR_GREATER_THAN, CSTR_LESS_THAN};

        use super::super::{
            classify_compare_string_ordinal_result, os_str_eq_ignore_case, paths_overlap,
        };

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
