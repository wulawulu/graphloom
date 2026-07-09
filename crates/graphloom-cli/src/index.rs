//! Standard index command.

use std::time::{Duration, Instant};

use graphloom::{PipelineRunStats, WorkflowFunctionOutput};

use crate::{
    IndexArgs,
    config::{load_project_config, redacted_config_summary, validate_project},
    error::{CliError, Result},
    runtime::{build_runtime, prepare_full_index},
};

/// Successful index run result.
#[derive(Debug, Clone)]
pub struct IndexRunResult {
    /// Workflow outputs.
    pub workflow_outputs: Vec<WorkflowFunctionOutput>,
    /// Final stats.
    pub stats: PipelineRunStats,
    /// Elapsed wall time.
    pub elapsed: Duration,
}

/// Execute `graphloom index`.
///
/// # Errors
///
/// Returns a config, runtime, or pipeline error.
pub async fn run_index(args: &IndexArgs) -> Result<IndexRunResult> {
    if args.method != "standard" {
        return Err(CliError::UnsupportedMethod {
            method: args.method.clone(),
        });
    }
    let project = load_project_config(&args.root).await?;
    validate_project(&project, args.skip_validation || args.dry_run).await?;
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

    let started = Instant::now();
    prepare_full_index(&project).await?;
    let mut runtime = build_runtime(&project, args.cache_enabled(), args.verbose).await?;
    let outputs = runtime
        .pipeline
        .run(&runtime.config, &mut runtime.context)
        .await
        .map_err(|source| CliError::IndexFailed {
            source: Box::new(source),
        })?;
    let elapsed = started.elapsed();
    let stats = runtime.context.stats.clone();
    print_success_summary(&stats, elapsed);
    Ok(IndexRunResult {
        workflow_outputs: outputs,
        stats,
        elapsed,
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
    use crate::{InitArgs, init_project};

    #[tokio::test]
    async fn test_should_reject_unsupported_index_method() {
        let tempdir = TempDir::new().expect("tempdir");
        let error = run_index(&IndexArgs {
            root: tempdir.path().to_path_buf(),
            method: "fast".to_owned(),
            verbose: false,
            dry_run: false,
            cache: true,
            no_cache: false,
            skip_validation: true,
        })
        .await
        .expect_err("fast is unsupported");

        assert!(
            error
                .to_string()
                .contains("unsupported indexing method fast")
        );
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

        let result = run_index(&IndexArgs {
            root: tempdir.path().to_path_buf(),
            method: "standard".to_owned(),
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
