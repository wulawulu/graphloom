//! Project configuration paths.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use crate::{
    GraphLoomError, GraphRagConfig, Result,
    path_safety::{
        normalize_path, paths_overlap, resolve_path_following_links, resolve_path_rejecting_links,
    },
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
        paths.validate_output_path_safety()?;
        Ok(paths)
    }

    /// Validate that output writes cannot target protected project paths.
    ///
    /// # Errors
    ///
    /// Returns an error if output overlaps the project, input, cache, or logs.
    pub fn validate_output_path_safety(&self) -> Result<()> {
        self.validate_output_path_safety_with_home(user_home_dir().as_deref())
    }

    fn validate_output_path_safety_with_home(&self, home: Option<&Path>) -> Result<()> {
        let output = resolve_path_rejecting_links(&self.output_dir)?;
        let root = resolve_path_following_links(&self.root)?;
        let input = resolve_path_following_links(&self.input_dir)?;
        let cache = resolve_path_following_links(&self.cache_dir)?;
        let reporting = resolve_path_following_links(&self.reporting_dir)?;

        if output.resolved == root.resolved || root.resolved.starts_with(&output.resolved) {
            return unsafe_output(
                &self.output_dir,
                "output directory must not be project root or an ancestor of project root",
            );
        }
        for (path, label) in [
            (&input.resolved, "input"),
            (&cache.resolved, "cache"),
            (&reporting.resolved, "logs"),
        ] {
            if paths_overlap(&output.resolved, path)? {
                return unsafe_output(
                    &self.output_dir,
                    &format!("output directory must not overlap {label} directory"),
                );
            }
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
            if paths_overlap(&vector.resolved, path)? {
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

    /// Validate that managed vector-index resets cannot target protected project paths.
    ///
    /// # Errors
    ///
    /// Returns an error if the vector DB path is unsafe for an active embedding workflow.
    pub(crate) fn validate_vector_path_safety(&self) -> Result<()> {
        self.validate_vector_path_safety_with_home(user_home_dir().as_deref())
    }
}

impl LoadedProject {
    /// Build a loaded project from an already parsed config and project root.
    ///
    /// # Errors
    ///
    /// Returns an error when path resolution or safety validation fails.
    pub fn from_config(root: impl AsRef<Path>, mut config: GraphRagConfig) -> Result<Self> {
        let root = absolute_normalized(root.as_ref())?;
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

    #[cfg(windows)]
    #[test]
    fn test_should_reject_output_inside_input_with_different_case() {
        assert_output_overlap(
            "Input",
            "input/generated",
            "CacheDir",
            "LogsDir",
            "overlap input",
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_should_reject_output_inside_cache_with_different_case() {
        assert_output_overlap(
            "InputDir",
            "cache/generated",
            "Cache",
            "LogsDir",
            "overlap cache",
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_should_reject_output_inside_logs_with_different_case() {
        assert_output_overlap(
            "InputDir",
            "logs/generated",
            "CacheDir",
            "Logs",
            "overlap logs",
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_should_allow_case_insensitive_non_overlapping_siblings() {
        let tempdir = TempDir::new().expect("tempdir");
        let config = config_with_paths("Input-A", "input-b", "Cache", "Logs", "input-b/lancedb");

        ProjectPaths::resolve(tempdir.path(), &config)
            .expect("case-insensitive sibling directories should be allowed");
    }

    #[test]
    fn test_should_reject_output_inside_input_directory() {
        assert_output_overlap("input", "input/generated", "cache", "logs", "overlap input");
    }

    #[test]
    fn test_should_reject_output_ancestor_of_input_directory() {
        assert_output_overlap("data/input", "data", "cache", "logs", "overlap input");
    }

    #[test]
    fn test_should_reject_output_inside_cache_directory() {
        assert_output_overlap("input", "cache/generated", "cache", "logs", "overlap cache");
    }

    #[test]
    fn test_should_reject_output_inside_reporting_directory() {
        assert_output_overlap("input", "logs/generated", "cache", "logs", "overlap logs");
    }

    #[test]
    fn test_should_allow_separate_sibling_project_directories() {
        let tempdir = TempDir::new().expect("tempdir");
        let config = config_with_paths("input", "output", "cache", "logs", "output/lancedb");

        ProjectPaths::resolve(tempdir.path(), &config)
            .expect("separate sibling project directories should be allowed");
    }

    #[tokio::test]
    async fn test_should_reject_non_directory_existing_path_ancestor() {
        let tempdir = TempDir::new().expect("tempdir");
        let file = tempdir.path().join("file");
        tokio::fs::write(&file, "not a directory")
            .await
            .expect("file");

        let error = resolve_path_rejecting_links(&file.join("child"))
            .expect_err("non-directory ancestor should fail");

        assert!(error.to_string().contains("not a directory"));
    }

    #[cfg(windows)]
    #[test]
    fn test_should_resolve_canonical_verbatim_path_without_querying_prefix_only_path() {
        let tempdir = TempDir::new().expect("tempdir");
        let canonical = tempdir.path().canonicalize().expect("canonical tempdir");
        crate::path_safety::tests::windows::assert_windows_verbatim_path(&canonical);

        resolve_path_rejecting_links(&canonical.join("missing").join("child"))
            .expect("resolve verbatim temp path");
    }

    #[test]
    fn test_should_reject_output_ancestor_of_reporting_dir() {
        let tempdir = TempDir::new().expect("tempdir");
        let config = config_with_paths("input", "logs", "cache", "logs/index", "output/lancedb");

        let error = ProjectPaths::resolve(tempdir.path(), &config)
            .expect_err("output ancestor of logs should fail");
        assert!(error.to_string().contains("overlap logs"));
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
    fn test_should_reject_output_inside_resolved_input_symlink() {
        let tempdir = TempDir::new().expect("tempdir");
        let external = TempDir::new().expect("external");
        std::os::unix::fs::symlink(external.path(), tempdir.path().join("input"))
            .expect("input symlink");
        let output = external.path().join("generated");
        let vector = output.join("lancedb");
        let config = config_with_paths(
            "input",
            &output.to_string_lossy(),
            "cache",
            "logs",
            &vector.to_string_lossy(),
        );

        let error = ProjectPaths::resolve(tempdir.path(), &config)
            .expect_err("output inside resolved input should fail");
        assert!(error.to_string().contains("overlap input"));
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
    fn test_should_reject_output_inside_resolved_cache_symlink() {
        let tempdir = TempDir::new().expect("tempdir");
        let external = TempDir::new().expect("external");
        std::os::unix::fs::symlink(external.path(), tempdir.path().join("cache"))
            .expect("cache symlink");
        let output = external.path().join("generated");
        let vector = output.join("lancedb");
        let config = config_with_paths(
            "input",
            &output.to_string_lossy(),
            "cache",
            "logs",
            &vector.to_string_lossy(),
        );

        let error = ProjectPaths::resolve(tempdir.path(), &config)
            .expect_err("output inside resolved cache should fail");
        assert!(error.to_string().contains("overlap cache"));
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

    #[test]
    fn test_should_reject_unsafe_active_vector_paths() {
        let tempdir = TempDir::new().expect("tempdir");

        for (vector, expected) in [
            ("input", "overlap input"),
            ("output", "equal output"),
            ("cache", "overlap cache"),
            ("logs", "overlap logs"),
        ] {
            let config = config_with_paths("input", "output", "cache", "logs", vector);
            let paths = ProjectPaths::resolve(tempdir.path(), &config).expect("paths resolve");
            let error = paths
                .validate_vector_path_safety()
                .expect_err("unsafe vector path should fail");

            assert!(
                error.to_string().contains(expected),
                "vector path {vector} returned unexpected error: {error}"
            );
        }
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
            .validate_output_path_safety_with_home(Some(&home))
            .expect_err("output equal home should fail");

        assert!(error.to_string().contains("home directory"));
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

    fn assert_output_overlap(
        input: &str,
        output: &str,
        cache: &str,
        reporting: &str,
        expected: &str,
    ) {
        let tempdir = TempDir::new().expect("tempdir");
        let vector = format!("{output}/lancedb");
        let config = config_with_paths(input, output, cache, reporting, &vector);

        let error = ProjectPaths::resolve(tempdir.path(), &config)
            .expect_err("overlapping output should fail");
        assert!(error.to_string().contains(expected));
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
