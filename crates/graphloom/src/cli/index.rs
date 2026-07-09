//! Standard index command.

use std::{path::Path, sync::Arc, time::Duration};

use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::{
    PipelineRunStats, WorkflowCallbacks,
    api::{BuildIndexOptions, CacheMode, IndexRunResult, IndexingMethod, build_validated_index},
    cli::{
        args::{IndexArgs, IndexMethodArg},
        callbacks::ConsoleWorkflowCallbacks,
        error::{CliError, Result},
    },
    config::load::{
        ValidationMode, load_project_config, redacted_config_summary, validate_index_project,
    },
};

impl From<IndexMethodArg> for IndexingMethod {
    fn from(value: IndexMethodArg) -> Self {
        match value {
            IndexMethodArg::Standard => Self::Standard,
        }
    }
}

/// Execute `graphloom index`.
///
/// # Errors
///
/// Returns a config, runtime, or pipeline error.
pub async fn run(args: &IndexArgs) -> Result<IndexRunResult> {
    let project = load_project_config(&args.root).await?;
    let method = IndexingMethod::from(args.method);
    validate_index_project(
        &project,
        if args.skip_validation {
            ValidationMode::SkipOptional
        } else {
            ValidationMode::Full
        },
    )
    .await?;
    if args.dry_run {
        let summary = redacted_config_summary(&project.config)?;
        println!("Dry run for {}", project.root.display());
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).map_err(|source| CliError::ConfigParse {
                path: project.config_path.clone(),
                source: Box::new(source),
            })?
        );
        println!("Workflows:");
        for workflow in project.config.workflow_order() {
            println!("- {workflow}");
        }
        return Ok(IndexRunResult {
            workflow_outputs: Vec::new(),
            stats: PipelineRunStats::default(),
            elapsed: Duration::ZERO,
        });
    }

    let _log_guard = init_logging(&project.paths.reporting_dir, args.verbose).await?;
    let callback =
        Arc::new(ConsoleWorkflowCallbacks::new(args.verbose)) as Arc<dyn WorkflowCallbacks>;
    let project_root = project.root.clone();
    let result = build_validated_index(
        project,
        BuildIndexOptions {
            project_root,
            method,
            cache_mode: if args.cache_enabled() {
                CacheMode::Configured
            } else {
                CacheMode::Disabled
            },
            callbacks: vec![callback],
        },
    )
    .await?;
    print_success_summary(&result.stats, result.elapsed);
    Ok(result)
}

async fn init_logging(
    reporting_dir: &Path,
    verbose: bool,
) -> Result<Option<tracing_appender::non_blocking::WorkerGuard>> {
    tokio::fs::create_dir_all(reporting_dir)
        .await
        .map_err(|source| CliError::Io {
            operation: "create log directory",
            path: reporting_dir.to_path_buf(),
            source,
        })?;
    let filter = if verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };
    let file_appender = tracing_appender::rolling::never(reporting_dir, "indexing-engine.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    let console_layer = fmt::layer().with_target(false).with_writer(std::io::stderr);
    let file_layer = fmt::layer()
        .with_target(true)
        .with_ansi(false)
        .with_writer(file_writer);
    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(console_layer)
        .with(file_layer);
    Ok(match tracing::subscriber::set_global_default(subscriber) {
        Ok(()) => Some(guard),
        Err(source) => {
            drop(guard);
            return Err(CliError::RuntimeBuild {
                source: Box::new(source),
            });
        }
    })
}

fn print_success_summary(stats: &PipelineRunStats, elapsed: Duration) {
    println!("Index completed successfully");
    println!("Documents: {}", stats.document_count);
    println!("Text units: {}", stats.text_unit_count);
    println!("Entities: {}", stats.entity_count);
    println!("Relationships: {}", stats.relationship_count);
    println!("Communities: {}", stats.community_count);
    println!("Reports: {}", stats.report_count);
    println!("Embeddings: {}", stats.embedding_count);
    println!("Elapsed: {elapsed:.2?}");
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::cli::{InitArgs, init_project};

    #[tokio::test]
    async fn test_should_reject_unsupported_index_method() {
        let tempdir = TempDir::new().expect("tempdir");
        let error = run(&IndexArgs {
            root: tempdir.path().to_path_buf(),
            method: IndexMethodArg::Standard,
            verbose: false,
            dry_run: false,
            cache: true,
            no_cache: false,
            skip_validation: true,
        })
        .await
        .expect_err("missing settings should fail before indexing");

        assert!(error.to_string().contains("no settings"));
    }

    #[tokio::test]
    async fn test_should_dry_run_without_creating_runtime_outputs() {
        let tempdir = TempDir::new().expect("tempdir");
        init_project(&InitArgs {
            root: tempdir.path().to_path_buf(),
            model: "gpt-test".to_owned(),
            embedding: "embed-test".to_owned(),
            force: false,
        })
        .await
        .expect("init");
        tokio::fs::write(
            tempdir.path().join(".env"),
            "GRAPHRAG_API_KEY=super-secret-key\n",
        )
        .await
        .expect("dotenv");
        tokio::fs::write(
            tempdir.path().join("input").join("doc.txt"),
            "Alice works with Bob.",
        )
        .await
        .expect("input");

        let result = run(&IndexArgs {
            root: tempdir.path().to_path_buf(),
            method: IndexMethodArg::Standard,
            verbose: false,
            dry_run: true,
            cache: true,
            no_cache: false,
            skip_validation: false,
        })
        .await
        .expect("dry run");

        assert_eq!(result.workflow_outputs.len(), 0);
        assert!(!tempdir.path().join("output").exists());
        assert!(!tempdir.path().join("cache").exists());
        assert!(!tempdir.path().join("logs").exists());
    }
}
