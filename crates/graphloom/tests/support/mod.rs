use std::{
    io,
    path::{Path, PathBuf},
};

pub(crate) mod capture;

/// Temporary directory guard exposing the canonical filesystem path.
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) struct CanonicalTempDir {
    _guard: tempfile::TempDir,
    path: PathBuf,
}

impl CanonicalTempDir {
    #[allow(dead_code)]
    pub(crate) fn new() -> io::Result<Self> {
        let guard = tempfile::TempDir::new()?;
        let path = guard.path().canonicalize()?;
        Ok(Self {
            _guard: guard,
            path,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}
