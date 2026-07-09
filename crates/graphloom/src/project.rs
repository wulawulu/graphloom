//! Project configuration paths.

use std::{
    collections::BTreeSet,
    ffi::OsString,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

use crate::{
    CREATE_COMMUNITY_REPORTS_WORKFLOW, EXTRACT_COVARIATES_WORKFLOW, EXTRACT_GRAPH_WORKFLOW,
    GraphLoomError, GraphRagConfig, Result,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedPath {
    pub(crate) lexical: PathBuf,
    pub(crate) resolved: PathBuf,
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
        paths.validate_vector_path_safety()?;
        Ok(paths)
    }

    /// Validate paths that may be cleared by full indexing.
    ///
    /// # Errors
    ///
    /// Returns an error if output could delete project, input, cache, or logs.
    pub fn validate_destructive_paths(&self) -> Result<()> {
        self.validate_destructive_paths_with_home(user_home_dir().as_deref())
    }

    fn validate_destructive_paths_with_home(&self, home: Option<&Path>) -> Result<()> {
        let output = resolve_path_rejecting_links(&self.output_dir)?;
        let root = resolve_path_following_links(&self.root)?;
        let input = resolve_path_following_links(&self.input_dir)?;
        let cache = resolve_path_following_links(&self.cache_dir)?;
        let reporting = resolve_path_following_links(&self.reporting_dir)?;

        if output.resolved == root.resolved {
            return unsafe_output(
                &self.output_dir,
                "output directory must not be project root",
            );
        }
        if output.resolved == input.resolved {
            return unsafe_output(&self.output_dir, "output directory must not equal input");
        }
        if output.resolved == cache.resolved {
            return unsafe_output(&self.output_dir, "output directory must not equal cache");
        }
        if output.resolved == reporting.resolved {
            return unsafe_output(&self.output_dir, "output directory must not equal logs");
        }
        if root.resolved.starts_with(&output.resolved)
            || input.resolved.starts_with(&output.resolved)
            || cache.resolved.starts_with(&output.resolved)
            || reporting.resolved.starts_with(&output.resolved)
        {
            return unsafe_output(
                &self.output_dir,
                "output directory must not be an ancestor of project, input, cache, or logs",
            );
        }
        if is_filesystem_root(&output.resolved) {
            return unsafe_output(
                &self.output_dir,
                "output directory must not be filesystem root",
            );
        }
        if let Some(home) = home {
            let home = resolve_path_following_links(home)?;
            if output.resolved == home.resolved || home.resolved.starts_with(&output.resolved) {
                return unsafe_output(
                    &self.output_dir,
                    "output directory must not be home directory or an ancestor of home directory",
                );
            }
        }
        Ok(())
    }

    fn validate_vector_path_safety_with_home(&self, home: Option<&Path>) -> Result<()> {
        let vector = resolve_path_rejecting_links(&self.vector_db_uri)?;
        let root = resolve_path_following_links(&self.root)?;
        let output = resolve_path_rejecting_links(&self.output_dir)?;
        let input = resolve_path_following_links(&self.input_dir)?;
        let cache = resolve_path_following_links(&self.cache_dir)?;
        let reporting = resolve_path_following_links(&self.reporting_dir)?;

        if is_filesystem_root(&vector.resolved) {
            return unsafe_output(
                &self.vector_db_uri,
                "vector DB path must not be filesystem root",
            );
        }
        if vector.resolved == root.resolved || root.resolved.starts_with(&vector.resolved) {
            return unsafe_output(
                &self.vector_db_uri,
                "vector DB path must not be project root or an ancestor of project root",
            );
        }
        if vector.resolved == output.resolved || output.resolved.starts_with(&vector.resolved) {
            return unsafe_output(
                &self.vector_db_uri,
                "vector DB path must not equal output or be an ancestor of output",
            );
        }
        for (path, label) in [
            (&input.resolved, "input"),
            (&cache.resolved, "cache"),
            (&reporting.resolved, "logs"),
        ] {
            if vector.resolved.starts_with(path) || path.starts_with(&vector.resolved) {
                return unsafe_output(
                    &self.vector_db_uri,
                    &format!("vector DB path must not overlap {label} directory"),
                );
            }
        }
        if let Some(home) = home {
            let home = resolve_path_following_links(home)?;
            if vector.resolved == home.resolved || home.resolved.starts_with(&vector.resolved) {
                return unsafe_output(
                    &self.vector_db_uri,
                    "vector DB path must not be home directory or an ancestor of home directory",
                );
            }
        }
        Ok(())
    }

    /// Validate that the vector DB path cannot target destructive project paths.
    ///
    /// # Errors
    ///
    /// Returns an error if the vector DB path is unsafe for reset.
    pub(crate) fn validate_vector_path_safety(&self) -> Result<()> {
        self.validate_vector_path_safety_with_home(user_home_dir().as_deref())
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

/// Resolve a destructive path while rejecting symlink or reparse-point components.
///
/// Use this for paths that may be recursively deleted or reset, such as output
/// and vector database locations.
pub(crate) fn resolve_path_rejecting_links(path: &Path) -> Result<ResolvedPath> {
    resolve_path_with_existing_ancestor(path, LinkPolicy::Reject)
}

/// Resolve a non-destructive comparison path by following existing links.
///
/// Use this for input, cache, reporting, root, and home-directory paths that
/// should participate in safety comparisons by their real filesystem location
/// without being rejected solely because they contain links.
pub(crate) fn resolve_path_following_links(path: &Path) -> Result<ResolvedPath> {
    resolve_path_with_existing_ancestor(path, LinkPolicy::Follow)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkPolicy {
    Reject,
    Follow,
}

fn resolve_path_with_existing_ancestor(
    path: &Path,
    link_policy: LinkPolicy,
) -> Result<ResolvedPath> {
    let lexical = absolute_lexical(path)?;
    let mut current = PathBuf::new();
    let mut existing = None;
    for component in lexical.components() {
        current.push(component.as_os_str());
        match current.symlink_metadata() {
            Ok(metadata)
                if link_policy == LinkPolicy::Reject && is_symlink_or_reparse(&metadata) =>
            {
                return unsafe_output(path, "path must not contain symlink components");
            }
            Ok(_) => {
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

pub(crate) fn user_home_dir() -> Option<PathBuf> {
    user_home_dir_from_env(|name| std::env::var_os(name))
}

pub(crate) fn user_home_dir_from_env(get: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let non_empty = |name| get(name).filter(|value| !value.is_empty());
    if let Some(home) = non_empty("HOME") {
        return Some(normalize_path(&PathBuf::from(home)));
    }
    if let Some(userprofile) = non_empty("USERPROFILE") {
        return Some(normalize_path(&PathBuf::from(userprofile)));
    }
    let homedrive = non_empty("HOMEDRIVE")?;
    let homepath = non_empty("HOMEPATH")?;
    let mut home = PathBuf::from(homedrive);
    home.push(homepath);
    Some(normalize_path(&home))
}

fn absolute_lexical(path: &Path) -> Result<PathBuf> {
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
    Ok(normalize_path(&absolute))
}

#[cfg(windows)]
fn is_symlink_or_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_symlink_or_reparse(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn resolve_path(root: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        normalize_path(&path)
    } else {
        normalize_path(&root.join(path))
    }
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
    Ok(absolute
        .canonicalize()
        .unwrap_or_else(|_| normalize_path(&absolute)))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, ffi::OsString};

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_should_reject_output_ancestor_of_reporting_dir() {
        let tempdir = TempDir::new().expect("tempdir");
        let config = config_with_paths("input", "logs", "cache", "logs/index", "output/lancedb");

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
        let config = config_with_paths(
            "input",
            "output-link/index",
            "cache",
            "logs",
            "output/lancedb",
        );

        let error = ProjectPaths::resolve(tempdir.path(), &config)
            .expect_err("symlink output escape should fail");
        assert!(error.to_string().contains("symlink"));
    }

    #[cfg(unix)]
    #[test]
    fn test_should_allow_input_symlink_when_vector_path_is_separate() {
        let tempdir = TempDir::new().expect("tempdir");
        let external = TempDir::new().expect("external");
        std::os::unix::fs::symlink(external.path(), tempdir.path().join("input"))
            .expect("input symlink");

        let config = config_with_paths("input", "output", "cache", "logs", "output/lancedb");

        ProjectPaths::resolve(tempdir.path(), &config).expect("input symlink should be allowed");
    }

    #[cfg(unix)]
    #[test]
    fn test_should_allow_cache_symlink_when_vector_path_is_separate() {
        let tempdir = TempDir::new().expect("tempdir");
        let external = TempDir::new().expect("external");
        std::os::unix::fs::symlink(external.path(), tempdir.path().join("cache"))
            .expect("cache symlink");

        let config = config_with_paths("input", "output", "cache", "logs", "output/lancedb");

        ProjectPaths::resolve(tempdir.path(), &config).expect("cache symlink should be allowed");
    }

    #[cfg(unix)]
    #[test]
    fn test_should_allow_reporting_symlink_when_vector_path_is_separate() {
        let tempdir = TempDir::new().expect("tempdir");
        let external = TempDir::new().expect("external");
        std::os::unix::fs::symlink(external.path(), tempdir.path().join("logs"))
            .expect("logs symlink");

        let config = config_with_paths("input", "output", "cache", "logs", "output/lancedb");

        ProjectPaths::resolve(tempdir.path(), &config).expect("logs symlink should be allowed");
    }

    #[cfg(unix)]
    #[test]
    fn test_should_reject_vector_overlap_with_resolved_input_symlink() {
        let tempdir = TempDir::new().expect("tempdir");
        let external = TempDir::new().expect("external");
        std::os::unix::fs::symlink(external.path(), tempdir.path().join("input"))
            .expect("input symlink");
        let vector = external.path().join("vector");
        let config = config_with_paths(
            "input",
            "output",
            "cache",
            "logs",
            &vector.to_string_lossy(),
        );

        let error = ProjectPaths::resolve(tempdir.path(), &config)
            .expect_err("vector overlapping resolved input should fail");

        assert!(error.to_string().contains("overlap input"));
    }

    #[cfg(unix)]
    #[test]
    fn test_should_reject_vector_overlap_with_resolved_cache_symlink() {
        let tempdir = TempDir::new().expect("tempdir");
        let external = TempDir::new().expect("external");
        std::os::unix::fs::symlink(external.path(), tempdir.path().join("cache"))
            .expect("cache symlink");
        let vector = external.path().join("vector");
        let config = config_with_paths(
            "input",
            "output",
            "cache",
            "logs",
            &vector.to_string_lossy(),
        );

        let error = ProjectPaths::resolve(tempdir.path(), &config)
            .expect_err("vector overlapping resolved cache should fail");

        assert!(error.to_string().contains("overlap cache"));
    }

    #[cfg(unix)]
    #[test]
    fn test_should_reject_vector_overlap_with_resolved_reporting_symlink() {
        let tempdir = TempDir::new().expect("tempdir");
        let external = TempDir::new().expect("external");
        std::os::unix::fs::symlink(external.path(), tempdir.path().join("logs"))
            .expect("logs symlink");
        let vector = external.path().join("vector");
        let config = config_with_paths(
            "input",
            "output",
            "cache",
            "logs",
            &vector.to_string_lossy(),
        );

        let error = ProjectPaths::resolve(tempdir.path(), &config)
            .expect_err("vector overlapping resolved logs should fail");

        assert!(error.to_string().contains("overlap logs"));
    }

    #[test]
    fn test_should_resolve_home_from_home_env() {
        let home = user_home_dir_from_env(env_getter(&[("HOME", "/home/alice")])).expect("home");

        assert_eq!(home, PathBuf::from("/home/alice"));
    }

    #[test]
    fn test_should_skip_empty_home_and_use_userprofile() {
        let home =
            user_home_dir_from_env(env_getter(&[("HOME", ""), ("USERPROFILE", "/users/alice")]))
                .expect("home");

        assert_eq!(home, PathBuf::from("/users/alice"));
    }

    #[cfg(windows)]
    #[test]
    fn test_should_resolve_windows_home_from_userprofile() {
        let home = user_home_dir_from_env(env_getter(&[("USERPROFILE", r"C:\Users\Alice")]))
            .expect("home");

        assert_eq!(home, PathBuf::from(r"C:\Users\Alice"));
    }

    #[cfg(windows)]
    #[test]
    fn test_should_resolve_windows_home_from_homedrive_and_homepath() {
        let home = user_home_dir_from_env(env_getter(&[
            ("HOMEDRIVE", r"C:"),
            ("HOMEPATH", r"\Users\Alice"),
        ]))
        .expect("home");

        assert_eq!(home, PathBuf::from(r"C:\Users\Alice"));
    }

    #[test]
    fn test_should_prioritize_home_over_userprofile() {
        let home = user_home_dir_from_env(env_getter(&[
            ("HOME", "/home/alice"),
            ("USERPROFILE", "/users/bob"),
        ]))
        .expect("home");

        assert_eq!(home, PathBuf::from("/home/alice"));
    }

    #[tokio::test]
    async fn test_should_reject_output_equal_to_home() {
        let tempdir = TempDir::new().expect("tempdir");
        let home = tempdir.path().join("home");
        let project = tempdir.path().join("project");
        tokio::fs::create_dir(&home).await.expect("home dir");
        tokio::fs::create_dir(&project).await.expect("project dir");
        let config = config_with_paths(
            "input",
            &home.to_string_lossy(),
            "cache",
            "logs",
            "output/lancedb",
        );
        let paths = project_paths(&project, &config);

        let error = paths
            .validate_destructive_paths_with_home(Some(&home))
            .expect_err("output equal home should fail");

        assert!(error.to_string().contains("home directory"));
    }

    #[tokio::test]
    async fn test_should_reject_vector_equal_to_home() {
        let tempdir = TempDir::new().expect("tempdir");
        let home = tempdir.path().join("home");
        let project = tempdir.path().join("project");
        tokio::fs::create_dir(&home).await.expect("home dir");
        tokio::fs::create_dir(&project).await.expect("project dir");
        let config = config_with_paths("input", "output", "cache", "logs", &home.to_string_lossy());
        let paths = project_paths(&project, &config);

        let error = paths
            .validate_vector_path_safety_with_home(Some(&home))
            .expect_err("vector equal home should fail");

        assert!(error.to_string().contains("home directory"));
    }

    #[tokio::test]
    async fn test_should_reject_vector_ancestor_of_home() {
        let tempdir = TempDir::new().expect("tempdir");
        let home_parent = tempdir.path().join("home-parent");
        let home = home_parent.join("home");
        let project = tempdir.path().join("project");
        tokio::fs::create_dir(&home_parent)
            .await
            .expect("home parent dir");
        tokio::fs::create_dir(&home).await.expect("home dir");
        tokio::fs::create_dir(&project).await.expect("project dir");
        let config = config_with_paths(
            "input",
            "output",
            "cache",
            "logs",
            &home_parent.to_string_lossy(),
        );
        let paths = project_paths(&project, &config);

        let error = paths
            .validate_vector_path_safety_with_home(Some(&home))
            .expect_err("vector ancestor of home should fail");

        assert!(error.to_string().contains("home directory"));
    }

    #[tokio::test]
    async fn test_should_allow_vector_inside_home_project() {
        let tempdir = TempDir::new().expect("tempdir");
        let home = tempdir.path().join("home");
        tokio::fs::create_dir(&home).await.expect("home dir");
        let config = config_with_paths(
            "home/project/input",
            "home/project/output",
            "home/project/cache",
            "home/project/logs",
            "home/project/output/lancedb",
        );
        let paths = project_paths(tempdir.path(), &config);

        paths
            .validate_vector_path_safety_with_home(Some(&home))
            .expect("vector under home project should be allowed");
    }

    fn env_getter(values: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        let values = values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), OsString::from(value)))
            .collect::<BTreeMap<_, _>>();
        move |key| values.get(key).cloned()
    }

    fn project_paths(root: &Path, config: &GraphRagConfig) -> ProjectPaths {
        let root = absolute_normalized(root).expect("root");
        ProjectPaths {
            input_dir: resolve_path(&root, &config.input_storage.base_dir),
            output_dir: resolve_path(&root, &config.output_storage.base_dir),
            cache_dir: resolve_path(&root, &config.cache.storage.base_dir),
            reporting_dir: resolve_path(&root, &config.reporting.base_dir),
            vector_db_uri: resolve_path(&root, &config.vector_store.db_uri),
            root,
        }
    }

    fn config_with_paths(
        input: &str,
        output: &str,
        cache: &str,
        reporting: &str,
        vector: &str,
    ) -> GraphRagConfig {
        serde_yaml::from_str(&format!(
            r"
input_storage:
  type: file
  base_dir: {input:?}
output_storage:
  type: file
  base_dir: {output:?}
cache:
  type: json
  storage:
    type: file
    base_dir: {cache:?}
reporting:
  type: file
  base_dir: {reporting:?}
vector_store:
  type: lancedb
  db_uri: {vector:?}
"
        ))
        .expect("config")
    }
}
