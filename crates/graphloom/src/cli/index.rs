//! Standard index command.

use std::{path::Path, sync::Arc, time::Duration};

use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::{
    IndexRunStats, IndexWorkflowCallbacks,
    api::{BuildIndexOptions, CacheMode, IndexRunResult, IndexingMethod, build_validated_index},
    cli::{
        args::{IndexArgs, IndexMethodArg},
        callbacks::{ConsoleIndexWorkflowCallbacks, ConsoleStageProgress},
        error::{CliError, Result},
    },
    config::load::{
        ValidationMode, load_project_config, redacted_config_summary,
        validate_index_project_with_factory,
    },
    runtime::{DefaultModelFactory, ModelFactory},
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
    run_with_model_factory(args, &DefaultModelFactory).await
}

async fn run_with_model_factory(
    args: &IndexArgs,
    model_factory: &dyn ModelFactory,
) -> Result<IndexRunResult> {
    let progress = ConsoleStageProgress::start("project configuration load", args.verbose);
    let project = load_project_config(&args.root).await?;
    progress.finish();
    let method = IndexingMethod::from(args.method);
    let validation_stage = if args.skip_validation {
        "required project validation"
    } else {
        "project and model connectivity validation"
    };
    let progress = ConsoleStageProgress::start(validation_stage, args.verbose);
    validate_index_project_with_factory(
        &project,
        if args.skip_validation {
            ValidationMode::SkipOptional
        } else {
            ValidationMode::Full {
                cache_enabled: args.cache_enabled(),
            }
        },
        model_factory,
    )
    .await?;
    progress.finish();
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
            stats: IndexRunStats::default(),
            elapsed: Duration::ZERO,
        });
    }

    let _log_guard = init_logging(&project.paths.reporting_dir, args.verbose).await?;
    let callback = Arc::new(ConsoleIndexWorkflowCallbacks::new(args.verbose))
        as Arc<dyn IndexWorkflowCallbacks>;
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
    let file_filter = if verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };
    let console_filter = if verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("off")
    };
    let file_appender = tracing_appender::rolling::never(reporting_dir, "indexing-engine.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    let console_layer = fmt::layer()
        .with_target(false)
        .with_writer(std::io::stderr)
        .with_filter(console_filter);
    let file_layer = fmt::layer()
        .with_target(true)
        .with_ansi(false)
        .with_writer(file_writer)
        .with_filter(file_filter);
    let subscriber = tracing_subscriber::registry()
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

fn print_success_summary(stats: &IndexRunStats, elapsed: Duration) {
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
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use graphloom_llm::{
        CompletionModel, EmbeddingModel, MockCompletionModel, MockEmbeddingModel, ModelConfig,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::cli::{IndexMethodArg, InitArgs, init_project};

    #[tokio::test]
    async fn test_should_reject_missing_project_settings() {
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
        #[cfg(windows)]
        {
            let canonical = tempdir.path().canonicalize().expect("canonical tempdir");
            crate::path_safety::tests::windows::assert_windows_verbatim_path(&canonical);
        }
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

        let factory = TestModelFactory::default();
        let result = run_with_model_factory(
            &IndexArgs {
                root: tempdir.path().to_path_buf(),
                method: IndexMethodArg::Standard,
                verbose: false,
                dry_run: true,
                cache: true,
                no_cache: false,
                skip_validation: false,
            },
            &factory,
        )
        .await
        .expect("dry run");

        assert_eq!(result.workflow_outputs.len(), 0);
        assert!(!tempdir.path().join("output").exists());
        assert!(!tempdir.path().join("output").join("lancedb").exists());
        assert!(!tempdir.path().join("cache").exists());
        assert!(!tempdir.path().join("logs").exists());
        assert_eq!(factory.completion_calls.load(Ordering::SeqCst), 1);
        assert_eq!(factory.embedding_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_should_reject_output_file_during_dry_run_without_side_effects() {
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
        tokio::fs::write(tempdir.path().join("input/doc.txt"), "Alice")
            .await
            .expect("input");
        let output = tempdir.path().join("output");
        tokio::fs::write(&output, "preserve dry-run output")
            .await
            .expect("output file");
        let factory = TestModelFactory::default();

        let error = run_with_model_factory(
            &IndexArgs {
                root: tempdir.path().to_path_buf(),
                method: IndexMethodArg::Standard,
                verbose: false,
                dry_run: true,
                cache: true,
                no_cache: false,
                skip_validation: false,
            },
            &factory,
        )
        .await
        .expect_err("output file must fail dry-run validation");

        assert!(error.to_string().contains("not a directory"));
        assert_eq!(
            tokio::fs::read_to_string(&output)
                .await
                .expect("output contents"),
            "preserve dry-run output"
        );
        assert_eq!(factory.completion_calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.embedding_calls.load(Ordering::SeqCst), 0);
        assert!(!tempdir.path().join("cache").exists());
        assert!(!tempdir.path().join("logs").exists());
        assert!(!tempdir.path().join("output/lancedb").exists());
    }

    #[tokio::test]
    async fn test_should_skip_optional_and_connectivity_validation() {
        let tempdir = TempDir::new().expect("tempdir");
        init_project(&InitArgs {
            root: tempdir.path().to_path_buf(),
            model: "gpt-test".to_owned(),
            embedding: "embed-test".to_owned(),
            force: false,
        })
        .await
        .expect("init");
        let factory = TestModelFactory::default();

        let result = run_with_model_factory(
            &IndexArgs {
                root: tempdir.path().to_path_buf(),
                method: IndexMethodArg::Standard,
                verbose: false,
                dry_run: true,
                cache: true,
                no_cache: false,
                skip_validation: true,
            },
            &factory,
        )
        .await
        .expect("skip-validation dry run");

        assert!(result.workflow_outputs.is_empty());
        assert_eq!(factory.completion_calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.embedding_calls.load(Ordering::SeqCst), 0);
        assert!(!tempdir.path().join("output").exists());
        assert!(!tempdir.path().join("cache").exists());
        assert!(!tempdir.path().join("logs").exists());
    }

    #[tokio::test]
    async fn test_should_report_missing_input_before_model_connectivity() {
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
        let factory = TestModelFactory::default();

        let error = run_with_model_factory(
            &IndexArgs {
                root: tempdir.path().to_path_buf(),
                method: IndexMethodArg::Standard,
                verbose: false,
                dry_run: true,
                cache: true,
                no_cache: false,
                skip_validation: false,
            },
            &factory,
        )
        .await
        .expect_err("missing input should fail before model calls");

        assert!(error.to_string().contains("no matching input files found"));
        assert_eq!(factory.completion_calls.load(Ordering::SeqCst), 0);
        assert_eq!(factory.embedding_calls.load(Ordering::SeqCst), 0);
    }

    #[derive(Debug, Default)]
    struct TestModelFactory {
        completion_calls: AtomicUsize,
        embedding_calls: AtomicUsize,
    }

    impl ModelFactory for TestModelFactory {
        fn create_completion(
            &self,
            id: &str,
            _config: &ModelConfig,
            _concurrent_requests: usize,
        ) -> crate::Result<Arc<dyn CompletionModel>> {
            self.completion_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(MockCompletionModel::new(
                id,
                vec!["Any non-empty response".to_owned()],
            )))
        }

        fn create_embedding(
            &self,
            id: &str,
            _config: &ModelConfig,
            _concurrent_requests: usize,
        ) -> crate::Result<Arc<dyn EmbeddingModel>> {
            self.embedding_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(MockEmbeddingModel::new(id, vec![0.0; 3_072])))
        }
    }
}
