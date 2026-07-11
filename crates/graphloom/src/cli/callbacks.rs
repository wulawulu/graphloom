//! IndexWorkflow callbacks for CLI progress.

use std::fmt;

use crate::{IndexRunStats, IndexWorkflowCallbacks};

/// CLI workflow callbacks.
pub struct ConsoleWorkflowCallbacks {
    verbose: bool,
}

impl fmt::Debug for ConsoleWorkflowCallbacks {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConsoleWorkflowCallbacks")
            .field("verbose", &self.verbose)
            .finish_non_exhaustive()
    }
}

impl ConsoleWorkflowCallbacks {
    /// Create callbacks writing user-visible progress to the console.
    #[must_use]
    pub fn new(verbose: bool) -> Self {
        Self { verbose }
    }
}

impl IndexWorkflowCallbacks for ConsoleWorkflowCallbacks {
    fn workflow_started(&self, workflow_name: &str) {
        println!("Starting {workflow_name}");
        tracing::info!(workflow = workflow_name, "workflow started");
    }

    fn workflow_completed(&self, workflow_name: &str, stats: &IndexRunStats) {
        println!("Completed {workflow_name}");
        tracing::info!(
            workflow = workflow_name,
            documents = stats.document_count,
            text_units = stats.text_unit_count,
            entities = stats.entity_count,
            relationships = stats.relationship_count,
            communities = stats.community_count,
            reports = stats.report_count,
            embeddings = stats.embedding_count,
            "workflow completed"
        );
    }

    fn progress(&self, workflow_name: &str, completed: usize, total: Option<usize>) {
        if self.verbose {
            match total {
                Some(total) => println!("{workflow_name}: {completed}/{total}"),
                None => println!("{workflow_name}: {completed}"),
            }
        }
        tracing::debug!(
            workflow = workflow_name,
            completed,
            total,
            "workflow progress"
        );
    }

    fn warning(&self, workflow_name: &str, message: &str) {
        eprintln!("Warning in {workflow_name}: {message}");
        tracing::warn!(workflow = workflow_name, message, "workflow warning");
    }

    fn error(&self, workflow_name: &str, message: &str) {
        eprintln!("Failed {workflow_name}: {message}");
        tracing::error!(workflow = workflow_name, message, "workflow error");
    }
}
