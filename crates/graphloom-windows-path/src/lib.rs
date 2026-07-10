//! Safe Windows ordinal comparison for filesystem path components.

#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

#[cfg(windows)]
use std::{ffi::OsStr, os::windows::ffi::OsStrExt};

#[cfg(windows)]
use windows_sys::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};

/// Compares Windows strings with ordinal, case-insensitive filesystem semantics.
#[cfg(windows)]
#[must_use]
pub fn os_str_eq_ignore_case(left: &OsStr, right: &OsStr) -> bool {
    let left = left.encode_wide().collect::<Vec<_>>();
    let right = right.encode_wide().collect::<Vec<_>>();
    let (Ok(left_len), Ok(right_len)) = (i32::try_from(left.len()), i32::try_from(right.len()))
    else {
        return false;
    };

    // SAFETY: Both pointers remain valid for the explicitly supplied slice lengths
    // throughout the call. CompareStringOrdinal does not retain or mutate them.
    unsafe {
        CompareStringOrdinal(left.as_ptr(), left_len, right.as_ptr(), right_len, 1) == CSTR_EQUAL
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::ffi::OsStr;

    use super::os_str_eq_ignore_case;

    #[test]
    fn test_should_compare_windows_strings_using_ordinal_ignore_case() {
        assert!(os_str_eq_ignore_case(
            OsStr::new("Input"),
            OsStr::new("input")
        ));
        assert!(os_str_eq_ignore_case(OsStr::new("ABC"), OsStr::new("abc")));
        assert!(os_str_eq_ignore_case(OsStr::new("Ä"), OsStr::new("ä")));
        assert!(!os_str_eq_ignore_case(
            OsStr::new("Input-A"),
            OsStr::new("input-b")
        ));
    }
}
