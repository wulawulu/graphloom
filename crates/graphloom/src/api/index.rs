//! Public indexing API.

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    GraphLoomError, GraphRagConfig, IndexRunStats, IndexWorkflowCallbacks, IndexWorkflowOutput,
    Result,
    config::load::{ValidationMode, validate_index_project},
    project::LoadedProject,
    runtime::prepare_index_runtime,
};

/// Supported indexing method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexingMethod {
    /// Standard full indexing pipeline.
    Standard,
}

/// Cache mode for indexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    /// Use the cache configuration from settings.
    Configured,
    /// Disable cache for this run.
    Disabled,
}

/// Options for [`build_index`].
#[derive(Debug, Clone)]
pub struct BuildIndexOptions {
    /// Project root used to resolve prompt paths and project-relative storage.
    pub project_root: PathBuf,
    /// Indexing method.
    pub method: IndexingMethod,
    /// Cache mode.
    pub cache_mode: CacheMode,
    /// IndexWorkflow callbacks.
    pub callbacks: Vec<Arc<dyn IndexWorkflowCallbacks>>,
}

/// Successful index run result.
#[derive(Debug, Clone)]
pub struct IndexRunResult {
    /// IndexWorkflow outputs.
    pub workflow_outputs: Vec<IndexWorkflowOutput>,
    /// Final stats.
    pub stats: IndexRunStats,
    /// Elapsed wall time.
    pub elapsed: Duration,
}

/// Build a full standard index.
///
/// # Errors
///
/// Returns a runtime or pipeline error when indexing fails.
pub async fn build_index(
    config: GraphRagConfig,
    options: BuildIndexOptions,
) -> Result<IndexRunResult> {
    let project = LoadedProject::from_config(options.project_root.clone(), config)?;
    tracing::info!(project_root = %project.root.display(), "validating index configuration");
    validate_index_project(
        &project,
        ValidationMode::Full {
            cache_enabled: matches!(options.cache_mode, CacheMode::Configured),
        },
    )
    .await?;
    build_validated_index(project, options).await
}

/// Build an index for a project that has already passed the desired validation depth.
///
/// This is crate-private so callers cannot bypass required/safety validation.
pub(crate) async fn build_validated_index(
    project: LoadedProject,
    options: BuildIndexOptions,
) -> Result<IndexRunResult> {
    match options.method {
        IndexingMethod::Standard => {}
    }
    let started = Instant::now();
    let cache_enabled = matches!(options.cache_mode, CacheMode::Configured);
    let active_root = project.root.clone();
    let prepared = prepare_index_runtime(&project, cache_enabled, options.callbacks).await?;
    let mut runtime = prepared.into_runtime(project.config.clone(), &active_root);
    tracing::info!(project_root = %active_root.display(), "index run started");
    tracing::info!(project_root = %active_root.display(), "running indexing pipeline");
    let outputs = runtime
        .pipeline
        .run(&runtime.config, &mut runtime.context)
        .await
        .map_err(|source| {
            // Completed workflows write directly to the configured output and are intentionally
            // retained. Incremental merge semantics belong to a future update pipeline.
            tracing::error!(
                project_root = %active_root.display(),
                error = %source,
                "index run failed; output may contain results from completed workflows"
            );
            GraphLoomError::IndexFailed {
                source: Box::new(source),
            }
        })?;
    let stats = runtime.context.stats.clone();
    let elapsed = started.elapsed();
    tracing::info!(
        documents = stats.document_count,
        text_units = stats.text_unit_count,
        entities = stats.entity_count,
        relationships = stats.relationship_count,
        communities = stats.community_count,
        reports = stats.report_count,
        embeddings = stats.embedding_count,
        elapsed_ms = elapsed.as_millis(),
        "index run completed"
    );
    Ok(IndexRunResult {
        workflow_outputs: outputs,
        stats,
        elapsed,
    })
}
