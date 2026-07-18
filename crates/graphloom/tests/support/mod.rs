use std::{
    io,
    path::{Path, PathBuf},
};

/// Temporary directory guard exposing the canonical filesystem path.
#[derive(Debug)]
pub(crate) struct CanonicalTempDir {
    _guard: tempfile::TempDir,
    path: PathBuf,
}

impl CanonicalTempDir {
    pub(crate) fn new() -> io::Result<Self> {
        let guard = tempfile::TempDir::new()?;
        let path = guard.path().canonicalize()?;
        Ok(Self {
            _guard: guard,
            path,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}
