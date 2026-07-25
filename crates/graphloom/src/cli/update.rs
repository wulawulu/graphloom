//! Incremental update command.

use std::sync::Arc;

use crate::{
    IndexWorkflowCallbacks,
    api::{CacheMode, IndexRunResult, IndexingMethod, UpdateIndexOptions, update_validated_index},
    cli::{
        UpdateArgs,
        callbacks::{ConsoleIndexWorkflowCallbacks, ConsoleStageProgress},
        error::Result,
        index::init_logging,
    },
    config::load::{ValidationMode, load_project_config, validate_update_project_with_factory},
    runtime::{DefaultModelFactory, ModelFactory},
};

/// Execute `graphloom update`.
///
/// # Errors
///
/// Returns a config, runtime, provider, workflow, or vector error.
pub async fn run(args: &UpdateArgs) -> Result<IndexRunResult> {
    run_with_model_factory(args, &DefaultModelFactory).await
}

async fn run_with_model_factory(
    args: &UpdateArgs,
    model_factory: &dyn ModelFactory,
) -> Result<IndexRunResult> {
    let progress = ConsoleStageProgress::start("project configuration load", args.verbose);
    let project = load_project_config(&args.root).await?;
    progress.finish();
    let progress = ConsoleStageProgress::start(
        if args.skip_validation {
            "required project validation"
        } else {
            "project and model connectivity validation"
        },
        args.verbose,
    );
    validate_update_project_with_factory(
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

    let _log_guard = init_logging(&project.paths.reporting_dir, args.verbose).await?;
    let callback = Arc::new(ConsoleIndexWorkflowCallbacks::new(args.verbose))
        as Arc<dyn IndexWorkflowCallbacks>;
    let project_root = project.root.clone();
    let result = update_validated_index(
        project,
        UpdateIndexOptions {
            project_root,
            method: IndexingMethod::from(args.method),
            cache_mode: if args.cache_enabled() {
                CacheMode::Configured
            } else {
                CacheMode::Disabled
            },
            callbacks: vec![callback],
        },
    )
    .await?;
    print_success_summary(&result);
    Ok(result)
}

fn print_success_summary(result: &IndexRunResult) {
    println!("Update completed successfully");
    println!("New documents: {}", result.stats.update_document_count);
    println!("Documents: {}", result.stats.document_count);
    println!("Text units: {}", result.stats.text_unit_count);
    println!("Entities: {}", result.stats.entity_count);
    println!("Relationships: {}", result.stats.relationship_count);
    println!("Communities: {}", result.stats.community_count);
    println!("Reports: {}", result.stats.report_count);
    println!("Embeddings: {}", result.stats.embedding_count);
    println!("Elapsed: {:.2?}", result.elapsed);
}
