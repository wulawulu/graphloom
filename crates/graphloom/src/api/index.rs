//! Public indexing API.

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    GraphLoomError, GraphRagConfig, IndexRunStats, IndexWorkflowCallbacks, IndexWorkflowOutput,
    Result,
    config::load::{ValidationMode, validate_index_project, validate_update_project},
    project::LoadedProject,
    runtime::{IndexRuntime, prepare_index_runtime, prepare_update_runtime},
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
    /// `IndexWorkflow` callbacks.
    pub callbacks: Vec<Arc<dyn IndexWorkflowCallbacks>>,
}

/// Options for [`update_index`].
#[derive(Debug, Clone)]
pub struct UpdateIndexOptions {
    /// Project root used to resolve prompts and project-relative storage.
    pub project_root: PathBuf,
    /// Indexing method. Only [`IndexingMethod::Standard`] is supported.
    pub method: IndexingMethod,
    /// Cache mode.
    pub cache_mode: CacheMode,
    /// `IndexWorkflow` callbacks.
    pub callbacks: Vec<Arc<dyn IndexWorkflowCallbacks>>,
}

impl UpdateIndexOptions {
    /// Create standard update options with configured caching and no callbacks.
    #[must_use]
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
            method: IndexingMethod::Standard,
            cache_mode: CacheMode::Configured,
            callbacks: Vec::new(),
        }
    }
}

/// Successful index run result.
#[derive(Debug, Clone)]
pub struct IndexRunResult {
    /// `IndexWorkflow` outputs.
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

/// Incrementally update an existing standard index.
///
/// # Errors
///
/// Returns a validation, provider, workflow, or vector error. Completed table and
/// vector writes are retained when a later workflow fails.
pub async fn update_index(
    config: GraphRagConfig,
    options: UpdateIndexOptions,
) -> Result<IndexRunResult> {
    let project = LoadedProject::from_config(options.project_root.clone(), config)?;
    tracing::info!(project_root = %project.root.display(), "validating update configuration");
    validate_update_project(
        &project,
        ValidationMode::Full {
            cache_enabled: matches!(options.cache_mode, CacheMode::Configured),
        },
    )
    .await?;
    update_validated_index(project, options).await
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
    let cache_enabled = matches!(options.cache_mode, CacheMode::Configured);
    let active_root = project.root.clone();
    let prepared = prepare_index_runtime(&project, cache_enabled, options.callbacks).await?;
    let runtime = prepared.into_runtime(project.config.clone(), &active_root);
    execute_runtime(runtime, active_root, "index").await
}

/// Update an index for a project that has already passed the desired validation depth.
pub(crate) async fn update_validated_index(
    project: LoadedProject,
    options: UpdateIndexOptions,
) -> Result<IndexRunResult> {
    match options.method {
        IndexingMethod::Standard => {}
    }
    let cache_enabled = matches!(options.cache_mode, CacheMode::Configured);
    let active_root = project.root.clone();
    let prepared = prepare_update_runtime(&project, cache_enabled, options.callbacks).await?;
    let runtime = prepared.into_runtime(project.config.clone(), &active_root);
    execute_runtime(runtime, active_root, "update").await
}

async fn execute_runtime(
    mut runtime: IndexRuntime,
    active_root: PathBuf,
    run_kind: &'static str,
) -> Result<IndexRunResult> {
    let started = Instant::now();
    if run_kind == "update" {
        tracing::info!(project_root = %active_root.display(), "update run started");
        let timestamp = runtime
            .context
            .update_state("update startup")?
            .timestamp
            .clone();
        tracing::info!(update_timestamp = timestamp, "update namespaces prepared");
    } else {
        tracing::info!(project_root = %active_root.display(), "index run started");
    }
    let outputs = runtime
        .pipeline
        .run(&runtime.config, &mut runtime.context)
        .await
        .map_err(|source| {
            tracing::error!(
                project_root = %active_root.display(),
                run_kind,
                error = %source,
                "indexing run failed; completed table and vector writes are retained"
            );
            GraphLoomError::IndexFailed {
                source: Box::new(source),
            }
        })?;
    if run_kind == "update" {
        refresh_final_table_stats(&mut runtime).await?;
    }
    let stats = runtime.context.stats.clone();
    let elapsed = started.elapsed();
    if run_kind == "update" {
        tracing::info!(
            documents = stats.document_count,
            text_units = stats.text_unit_count,
            entities = stats.entity_count,
            relationships = stats.relationship_count,
            communities = stats.community_count,
            reports = stats.report_count,
            embeddings = stats.embedding_count,
            elapsed_ms = elapsed.as_millis(),
            "update run completed"
        );
    } else {
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
    }
    Ok(IndexRunResult {
        workflow_outputs: outputs,
        stats,
        elapsed,
    })
}

async fn refresh_final_table_stats(runtime: &mut IndexRuntime) -> Result<()> {
    let final_provider = Arc::clone(
        &runtime
            .context
            .update_state("update stats")?
            .final_table_provider,
    );
    runtime.context.stats.document_count =
        table_height(final_provider.as_ref(), "documents").await?;
    runtime.context.stats.text_unit_count =
        table_height(final_provider.as_ref(), "text_units").await?;
    runtime.context.stats.entity_count = table_height(final_provider.as_ref(), "entities").await?;
    runtime.context.stats.relationship_count =
        table_height(final_provider.as_ref(), "relationships").await?;
    runtime.context.stats.community_count =
        table_height(final_provider.as_ref(), "communities").await?;
    runtime.context.stats.report_count =
        table_height(final_provider.as_ref(), "community_reports").await?;
    Ok(())
}

async fn table_height(
    provider: &dyn graphloom_storage::TableProvider,
    table: &str,
) -> Result<usize> {
    Ok(provider
        .read_optional_dataframe(table)
        .await?
        .map_or(0, |dataframe| dataframe.height()))
}
