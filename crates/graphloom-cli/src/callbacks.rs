//! Workflow callbacks for CLI progress and logging.

use std::{fmt, path::Path, sync::Arc};

use graphloom::{PipelineRunStats, WorkflowCallbacks};
use tokio::{
    fs::{File, OpenOptions},
    io::AsyncWriteExt,
    sync::Mutex,
};

use crate::error::{CliError, Result};

/// CLI workflow callbacks.
pub struct ConsoleWorkflowCallbacks {
    verbose: bool,
    log: Arc<Mutex<File>>,
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
    /// Create callbacks writing to a stable log file.
    ///
    /// # Errors
    ///
    /// Returns an error when the log file cannot be opened.
    pub async fn new(log_dir: &Path, verbose: bool) -> Result<Self> {
        tokio::fs::create_dir_all(log_dir)
            .await
            .map_err(|source| CliError::Io {
                operation: "create log directory",
                path: log_dir.to_path_buf(),
                source,
            })?;
        let path = log_dir.join("indexing-engine.log");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|source| CliError::Io {
                operation: "open log file",
                path,
                source,
            })?;
        Ok(Self {
            verbose,
            log: Arc::new(Mutex::new(file)),
        })
    }

    fn write_log(&self, message: &str) {
        let log = Arc::clone(&self.log);
        let message = format!("{message}\n");
        tokio::spawn(async move {
            let mut file = log.lock().await;
            let _ = file.write_all(message.as_bytes()).await;
            let _ = file.flush().await;
        });
    }
}

impl WorkflowCallbacks for ConsoleWorkflowCallbacks {
    fn workflow_started(&self, workflow_name: &str) {
        println!("Starting {workflow_name}");
        self.write_log(&format!("workflow started: {workflow_name}"));
    }

    fn workflow_completed(&self, workflow_name: &str, stats: &PipelineRunStats) {
        println!("Completed {workflow_name}");
        self.write_log(&format!(
            "workflow completed: {workflow_name}; documents={}; text_units={}; entities={}; \
             relationships={}; communities={}; reports={}; embeddings={}",
            stats.document_count,
            stats.text_unit_count,
            stats.entity_count,
            stats.relationship_count,
            stats.community_count,
            stats.report_count,
            stats.embedding_count
        ));
    }

    fn progress(&self, workflow_name: &str, completed: usize, total: Option<usize>) {
        if self.verbose {
            match total {
                Some(total) => println!("{workflow_name}: {completed}/{total}"),
                None => println!("{workflow_name}: {completed}"),
            }
        }
    }

    fn warning(&self, workflow_name: &str, message: &str) {
        eprintln!("Warning in {workflow_name}: {message}");
        self.write_log(&format!("warning in {workflow_name}: {message}"));
    }

    fn error(&self, workflow_name: &str, message: &str) {
        eprintln!("Failed {workflow_name}: {message}");
        self.write_log(&format!("error in {workflow_name}: {message}"));
    }
}
