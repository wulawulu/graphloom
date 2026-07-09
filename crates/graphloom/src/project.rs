//! Project configuration paths.

use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

use crate::{
    CREATE_COMMUNITY_REPORTS_WORKFLOW, EXTRACT_COVARIATES_WORKFLOW, EXTRACT_GRAPH_WORKFLOW,
    FINALIZE_GRAPH_WORKFLOW, GraphLoomError, GraphRagConfig, Result,
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
        let root = absolute_normalized(root)?;
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
        reject_symlink_components(&self.output_dir)?;
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
        if root.starts_with(&output)
            || input.starts_with(&output)
            || cache.starts_with(&output)
            || reporting.starts_with(&output)
        {
            return unsafe_output(
                &self.output_dir,
                "output directory must not be an ancestor of project, input, cache, or logs",
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

    /// Resolve configured prompt paths used by active workflows.
    #[must_use]
    pub fn active_prompt_paths(
        &self,
        config: &GraphRagConfig,
        active: &BTreeSet<String>,
    ) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if active.contains(EXTRACT_GRAPH_WORKFLOW) {
            if let Some(path) = config.extract_graph.prompt.as_deref() {
                paths.push(resolve_path(&self.root, path));
            }
            if let Some(path) = config.summarize_descriptions.prompt.as_deref() {
                paths.push(resolve_path(&self.root, path));
            }
        }
        if active.contains(FINALIZE_GRAPH_WORKFLOW)
            && let Some(path) = config.summarize_descriptions.prompt.as_deref()
        {
            paths.push(resolve_path(&self.root, path));
        }
        if active.contains(EXTRACT_COVARIATES_WORKFLOW)
            && config.extract_claims.enabled
            && let Some(path) = config.extract_claims.prompt.as_deref()
        {
            paths.push(resolve_path(&self.root, path));
        }
        if active.contains(CREATE_COMMUNITY_REPORTS_WORKFLOW) {
            if let Some(path) = config.community_reports.graph_prompt.as_deref() {
                paths.push(resolve_path(&self.root, path));
            }
            if let Some(path) = config.community_reports.text_prompt.as_deref() {
                paths.push(resolve_path(&self.root, path));
            }
        }
        paths
    }
}

impl LoadedProject {
    /// Build a loaded project from an already parsed config and project root.
    ///
    /// # Errors
    ///
    /// Returns an error when path resolution or safety validation fails.
    pub fn from_config(root: PathBuf, mut config: GraphRagConfig) -> Result<Self> {
        let root = absolute_normalized(&root)?;
        let paths = ProjectPaths::resolve(&root, &config)?;
        config.vector_store.db_uri = paths.vector_db_uri.to_string_lossy().to_string();
        Ok(Self {
            root,
            config_path: paths.root.join("settings.yaml"),
            config,
            paths,
        })
    }
}

fn unsafe_output<T>(path: &Path, message: &str) -> Result<T> {
    Err(GraphLoomError::UnsafeOutputPath {
        path: path.to_path_buf(),
        message: message.to_owned(),
    })
}

fn reject_symlink_components(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        match current.symlink_metadata() {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return unsafe_output(path, "output directory must not contain symlink components");
            }
            Ok(_) => {}
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(GraphLoomError::Io {
                    operation: "inspect output path",
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(())
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

fn absolute_normalized(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|source| GraphLoomError::Io {
                operation: "get current directory",
                path: PathBuf::from("."),
                source,
            })?
            .join(path)
    };
    Ok(canonical_or_normalized(&absolute))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_should_reject_output_ancestor_of_reporting_dir() {
        let tempdir = TempDir::new().expect("tempdir");
        let config: GraphRagConfig = serde_yaml::from_str(
            r"
output_storage:
  type: file
  base_dir: logs
reporting:
  type: file
  base_dir: logs/index
",
        )
        .expect("config");

        let error = ProjectPaths::resolve(tempdir.path(), &config)
            .expect_err("output ancestor of logs should fail");
        assert!(error.to_string().contains("ancestor"));
    }

    #[cfg(unix)]
    #[test]
    fn test_should_reject_destructive_output_symlink_escape() {
        let tempdir = TempDir::new().expect("tempdir");
        let external = TempDir::new().expect("external");
        std::os::unix::fs::symlink(external.path(), tempdir.path().join("output-link"))
            .expect("symlink");
        let config: GraphRagConfig = serde_yaml::from_str(
            r"
output_storage:
  type: file
  base_dir: output-link/index
",
        )
        .expect("config");

        let error = ProjectPaths::resolve(tempdir.path(), &config)
            .expect_err("symlink output escape should fail");
        assert!(error.to_string().contains("symlink"));
    }
}
