//empty file
//! Common infrastructure shared across `GraphLoom` crates.
//!
//! This crate intentionally stays free of `GraphRAG` workflow, table, LLM, and
//! vector-store business logic. It provides the low-level error, configuration,
//! and tracing helpers that the domain crates build on.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use std::{
    env,
    ffi::OsStr,
    path::{Path, PathBuf},
};

use serde::de::DeserializeOwned;
use thiserror::Error;
use tracing_subscriber::{EnvFilter, fmt};

/// Result type used by common infrastructure helpers.
pub type Result<T> = std::result::Result<T, CommonError>;

/// Errors raised by common infrastructure helpers.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CommonError {
    /// A filesystem operation failed.
    #[error("filesystem operation failed for {path}: {source}")]
    Filesystem {
        /// Path involved in the failed operation.
        path: PathBuf,
        /// Original IO error.
        #[source]
        source: std::io::Error,
    },

    /// `.env` parsing failed.
    #[error("failed to load environment file {path}: {source}")]
    Dotenv {
        /// Parsed environment file path.
        path: PathBuf,
        /// `.env` parser error.
        #[source]
        source: dotenvy::Error,
    },

    /// YAML parsing failed.
    #[error("failed to parse YAML configuration {path}: {source}")]
    Yaml {
        /// Parsed file path.
        path: PathBuf,
        /// YAML parser error.
        #[source]
        source: serde_yaml::Error,
    },

    /// Layered configuration loading failed.
    #[error("failed to load layered configuration: {0}")]
    Config(#[from] config::ConfigError),

    /// A referenced environment variable is not present.
    #[error("environment variable {name} referenced by configuration is not set")]
    MissingEnvironment {
        /// Missing variable name.
        name: String,
    },
}

/// Load a strongly typed YAML file.
///
/// Environment placeholders of the form `${NAME}` are expanded before parsing.
///
/// # Errors
///
/// Returns an error when the file cannot be read, an environment placeholder is
/// unresolved, or the expanded YAML cannot be deserialized into `T`.
pub async fn load_yaml_file<T>(path: impl AsRef<Path>) -> Result<T>
where
    T: DeserializeOwned,
{
    let path = path.as_ref();
    let raw = tokio::fs::read_to_string(path)
        .await
        .map_err(|source| CommonError::Filesystem {
            path: path.to_path_buf(),
            source,
        })?;
    let expanded = expand_environment(&raw)?;
    serde_yaml::from_str(&expanded).map_err(|source| CommonError::Yaml {
        path: path.to_path_buf(),
        source,
    })
}

/// Load `settings.yaml` from a `GraphLoom` project root.
///
/// If `.env` exists in the root directory, it is loaded before parsing the YAML
/// file. Process environment variables still take precedence because `dotenvy`
/// does not override existing variables.
///
/// # Errors
///
/// Returns an error when `.env` cannot be parsed, `settings.yaml` cannot be
/// read, or the YAML cannot be deserialized into `T`.
pub async fn load_project_settings<T>(root: impl AsRef<Path>) -> Result<T>
where
    T: DeserializeOwned,
{
    let root = root.as_ref();
    let env_path = root.join(".env");
    if tokio::fs::try_exists(&env_path)
        .await
        .map_err(|source| CommonError::Filesystem {
            path: env_path.clone(),
            source,
        })?
    {
        dotenvy::from_path(&env_path).map_err(|source| CommonError::Dotenv {
            path: env_path,
            source,
        })?;
    }

    load_yaml_file(root.join("settings.yaml")).await
}

/// Load a layered configuration with `config`, preserving compatibility with
/// YAML-oriented `GraphRAG` settings while allowing callers to opt into
/// `config`'s source stack.
///
/// # Errors
///
/// Returns a [`CommonError::Config`] when the source cannot be read or parsed.
pub fn layered_yaml_settings(path: impl AsRef<Path>) -> Result<config::Config> {
    let path = path.as_ref();
    config::Config::builder()
        .add_source(config::File::from(path).format(config::FileFormat::Yaml))
        .build()
        .map_err(CommonError::Config)
}

/// Install a default tracing subscriber for CLI and test harnesses.
///
/// Calling this after a global subscriber has already been installed is a no-op.
pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let subscriber = fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_thread_ids(false)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
}

/// Redact a secret-like value for logs and debug output.
#[must_use]
pub fn redact_secret(value: impl AsRef<str>) -> String {
    if value.as_ref().is_empty() {
        String::new()
    } else {
        "[redacted]".to_owned()
    }
}

fn expand_environment(input: &str) -> Result<String> {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find("${") {
        output.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find('}') else {
            output.push_str(&rest[start..]);
            return Ok(output);
        };

        let name = &after_start[..end];
        validate_environment_name(name)?;
        let value =
            env::var(name).map_err(|_| CommonError::MissingEnvironment { name: name.into() })?;
        output.push_str(&value);
        rest = &after_start[end + 1..];
    }

    output.push_str(rest);
    Ok(output)
}

fn validate_environment_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(CommonError::MissingEnvironment { name: name.into() });
    }

    Ok(())
}

/// Return `true` when a path component is acceptable for logical storage names.
#[must_use]
pub fn is_safe_path_component(component: &OsStr) -> bool {
    let Some(component) = component.to_str() else {
        return false;
    };

    !component.is_empty()
        && component != "."
        && component != ".."
        && !component.contains('\0')
        && !component.contains('/')
        && !component.contains('\\')
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::{expand_environment, redact_secret};

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct Demo {
        value: String,
    }

    #[test]
    fn test_should_expand_environment_placeholders() {
        let expanded = expand_environment("value: ${PATH}").expect("PATH should be set");
        let parsed: Demo = serde_yaml::from_str(&expanded).expect("expanded YAML should parse");

        assert!(!parsed.value.is_empty());
    }

    #[test]
    fn test_should_redact_non_empty_secret() {
        assert_eq!(redact_secret("token"), "[redacted]");
        assert_eq!(redact_secret(""), "");
    }
}
