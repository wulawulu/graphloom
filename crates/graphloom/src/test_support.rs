use std::path::{Path, PathBuf};

use tempfile::TempDir;

/// Temporary directory retained by its original guard but exposed through its
/// canonical filesystem path.
///
/// This avoids platform-owned lexical symlink prefixes such as macOS `/var`
/// while preserving GraphLoom's production rule that user-provided paths must
/// not traverse symlink ancestors.
#[derive(Debug)]
pub(crate) struct CanonicalTempDir {
    _guard: TempDir,
    path: PathBuf,
}

impl CanonicalTempDir {
    pub(crate) fn new() -> Self {
        let guard = TempDir::new().expect("tempdir");
        let path = guard.path().canonicalize().expect("canonical tempdir");
        Self {
            _guard: guard,
            path,
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::CanonicalTempDir;

    #[test]
    fn test_should_expose_canonical_existing_tempdir_path() {
        let tempdir = CanonicalTempDir::new();

        assert_eq!(
            tempdir.path(),
            tempdir.path().canonicalize().expect("canonical path")
        );
        assert!(tempdir.path().is_dir());
    }
}
