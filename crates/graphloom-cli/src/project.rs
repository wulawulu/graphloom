//! Project configuration paths.

use std::path::{Component, Path, PathBuf};

use graphloom::GraphRagConfig;

use crate::{
    error::{CliError, Result},
    init::PROMPT_ASSETS,
};

/// Loaded `GraphLoom` project.
#[derive(Debug, Clone)]
pub struct LoadedProject {
    /// Project root, equal to the settings file directory.
    pub root: PathBuf,
    /// Settings file path.
    pub config_path: PathBuf,
    /// Parsed configuration.
    pub config: GraphRagConfig,
    /// Resolved project paths.
    pub paths: ProjectPaths,
}

/// Resolved project paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectPaths {
    /// Project root.
    pub root: PathBuf,
    /// Input directory.
    pub input_dir: PathBuf,
    /// Output directory.
    pub output_dir: PathBuf,
    /// Cache directory.
    pub cache_dir: PathBuf,
    /// Reporting directory.
    pub reporting_dir: PathBuf,
    /// `LanceDB` URI path.
    pub vector_db_uri: PathBuf,
}

impl ProjectPaths {
    /// Resolve project paths from config.
    ///
    /// # Errors
    ///
    /// Returns an error when destructive output path validation fails.
    pub fn resolve(root: &Path, config: &GraphRagConfig) -> Result<Self> {
        let root = normalize_path(root);
        let input_dir = resolve_path(&root, &config.input_storage.base_dir);
        let output_dir = resolve_path(&root, &config.output_storage.base_dir);
        let cache_dir = resolve_path(&root, &config.cache.storage.base_dir);
        let reporting_dir = resolve_path(&root, &config.reporting.base_dir);
        let vector_db_uri = resolve_path(&root, &config.vector_store.db_uri);
        let paths = Self {
            root,
            input_dir,
            output_dir,
            cache_dir,
            reporting_dir,
            vector_db_uri,
        };
        paths.validate_destructive_paths()?;
        Ok(paths)
    }

    /// Validate paths that may be cleared by full indexing.
    ///
    /// # Errors
    ///
    /// Returns an error if output could delete project, input, cache, or logs.
    pub fn validate_destructive_paths(&self) -> Result<()> {
        let output = canonical_or_normalized(&self.output_dir);
        let root = canonical_or_normalized(&self.root);
        let input = canonical_or_normalized(&self.input_dir);
        let cache = canonical_or_normalized(&self.cache_dir);
        let reporting = canonical_or_normalized(&self.reporting_dir);

        if output == root {
            return unsafe_output(
                &self.output_dir,
                "output directory must not be project root",
            );
        }
        if output == input {
            return unsafe_output(&self.output_dir, "output directory must not equal input");
        }
        if output == cache {
            return unsafe_output(&self.output_dir, "output directory must not equal cache");
        }
        if output == reporting {
            return unsafe_output(&self.output_dir, "output directory must not equal logs");
        }
        if root.starts_with(&output) || input.starts_with(&output) || cache.starts_with(&output) {
            return unsafe_output(
                &self.output_dir,
                "output directory must not be an ancestor of project, input, or cache",
            );
        }
        if is_filesystem_root(&output) {
            return unsafe_output(
                &self.output_dir,
                "output directory must not be filesystem root",
            );
        }
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
            && output == canonical_or_normalized(&home)
        {
            return unsafe_output(
                &self.output_dir,
                "output directory must not be home directory",
            );
        }
        Ok(())
    }

    /// Resolve configured prompt paths that are expected to exist.
    #[must_use]
    pub fn prompt_paths(&self, config: &GraphRagConfig) -> Vec<PathBuf> {
        [
            config.extract_graph.prompt.as_deref(),
            config.summarize_descriptions.prompt.as_deref(),
            config.extract_claims.prompt.as_deref(),
            config.community_reports.graph_prompt.as_deref(),
            config.community_reports.text_prompt.as_deref(),
        ]
        .into_iter()
        .flatten()
        .filter(|path| is_prompt_file_path(path))
        .map(|path| resolve_path(&self.root, path))
        .collect()
    }

    /// Return all default prompt paths.
    #[must_use]
    pub fn managed_prompt_paths(&self) -> Vec<PathBuf> {
        PROMPT_ASSETS
            .iter()
            .map(|(name, _)| self.root.join("prompts").join(name))
            .collect()
    }
}

fn is_prompt_file_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
        || path.contains('/')
        || path.contains('\\')
}

fn unsafe_output<T>(path: &Path, message: &str) -> Result<T> {
    Err(CliError::UnsafeOutputPath {
        path: path.to_path_buf(),
        message: message.to_owned(),
    })
}

fn resolve_path(root: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        normalize_path(&path)
    } else {
        normalize_path(&root.join(path))
    }
}

fn canonical_or_normalized(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| normalize_path(path))
}

fn is_filesystem_root(path: &Path) -> bool {
    path.parent().is_none()
}

fn normalize_path(path: &Path) -> PathBuf {
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
