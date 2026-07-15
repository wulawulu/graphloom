//! `IndexWorkflow` callbacks and simple stage progress for the CLI.

use std::{
    sync::atomic::{AtomicU8, Ordering},
    time::Duration,
};

use indicatif::{ProgressBar, ProgressStyle};

use crate::{IndexRunStats, IndexWorkflowCallbacks};

const SPINNER_TICKS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", "✓"];
const SPINNER_INTERVAL: Duration = Duration::from_millis(100);

/// Progress for one CLI lifecycle stage.
#[derive(Debug)]
pub(crate) struct ConsoleStageProgress {
    bar: ProgressBar,
    stage: &'static str,
    finished: bool,
}

impl ConsoleStageProgress {
    /// Start a lifecycle stage.
    pub(crate) fn start(stage: &'static str, verbose: bool) -> Self {
        let bar = progress_bar(verbose);
        start_spinner(&bar, format!("Starting {stage}"));
        Self {
            bar,
            stage,
            finished: false,
        }
    }

    /// Finish a lifecycle stage successfully.
    pub(crate) fn finish(mut self) {
        finish_progress(&self.bar, format!("Completed {}", self.stage));
        self.finished = true;
    }
}

impl Drop for ConsoleStageProgress {
    fn drop(&mut self) {
        if !self.finished && !self.bar.is_hidden() {
            self.bar.disable_steady_tick();
            self.bar.finish_and_clear();
        }
    }
}

/// CLI workflow callbacks.
#[derive(Debug)]
pub struct ConsoleIndexWorkflowCallbacks {
    bar: ProgressBar,
    verbose: bool,
    last_reported_bucket: AtomicU8,
}

impl ConsoleIndexWorkflowCallbacks {
    /// Create callbacks writing user-visible progress to the console.
    #[must_use]
    pub fn new(verbose: bool) -> Self {
        let bar = progress_bar(verbose);
        Self::with_bar(bar, verbose)
    }

    fn with_bar(bar: ProgressBar, verbose: bool) -> Self {
        start_spinner(&bar, "Starting indexing runtime preparation");
        Self {
            bar,
            verbose,
            last_reported_bucket: AtomicU8::new(0),
        }
    }

    fn should_report_non_interactive(&self, completed: usize, total: usize) -> bool {
        let bucket = progress_bucket(completed, total);
        self.last_reported_bucket
            .fetch_max(bucket, Ordering::Relaxed)
            < bucket
    }
}

impl IndexWorkflowCallbacks for ConsoleIndexWorkflowCallbacks {
    fn runtime_prepared(&self) {
        finish_progress(&self.bar, "Completed indexing runtime preparation");
    }

    fn workflow_started(&self, workflow_name: &str) {
        self.last_reported_bucket.store(0, Ordering::Relaxed);
        start_spinner(&self.bar, format!("Starting {workflow_name}"));
        tracing::info!(workflow = workflow_name, "workflow started");
    }

    fn workflow_completed(&self, workflow_name: &str, stats: &IndexRunStats) {
        finish_progress(&self.bar, format!("Completed {workflow_name}"));
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
        if self.bar.is_hidden() {
            if self.verbose {
                match total {
                    Some(total) => println!("{workflow_name}: {completed}/{total}"),
                    None => println!("{workflow_name}: {completed}"),
                }
            } else if let Some(total) = total
                && self.should_report_non_interactive(completed, total)
            {
                println!("{workflow_name}: {completed}/{total}");
            }
        } else if let Some(total) = total {
            let total = u64::try_from(total).unwrap_or(u64::MAX);
            if self.bar.length() != Some(total) {
                self.bar.disable_steady_tick();
                self.bar.set_style(progress_style());
                self.bar.set_length(total);
                self.bar.set_message(workflow_name.to_owned());
            }
            self.bar
                .set_position(u64::try_from(completed).unwrap_or(u64::MAX));
        } else {
            if self.bar.length().is_some() {
                self.bar.unset_length();
                self.bar.set_style(spinner_style());
                self.bar.enable_steady_tick(SPINNER_INTERVAL);
            }
            self.bar
                .set_message(format!("{workflow_name}: {completed}"));
            self.bar.tick();
        }
        tracing::debug!(
            workflow = workflow_name,
            completed,
            total,
            "workflow progress"
        );
    }

    fn warning(&self, workflow_name: &str, message: &str) {
        print_above_progress(&self.bar, format!("Warning in {workflow_name}: {message}"));
        tracing::warn!(workflow = workflow_name, message, "workflow warning");
    }

    fn error(&self, workflow_name: &str, message: &str) {
        if self.bar.is_hidden() {
            eprintln!("Failed {workflow_name}: {message}");
        } else {
            self.bar.disable_steady_tick();
            self.bar.finish_and_clear();
            eprintln!("✗ Failed {workflow_name}: {message}");
        }
        tracing::error!(workflow = workflow_name, message, "workflow error");
    }
}

fn progress_bar(verbose: bool) -> ProgressBar {
    if verbose {
        ProgressBar::hidden()
    } else {
        ProgressBar::new_spinner()
    }
}

fn progress_bucket(completed: usize, total: usize) -> u8 {
    if total == 0 {
        return 100;
    }
    let percent = completed.saturating_mul(100).saturating_div(total).min(100);
    u8::try_from((percent / 10) * 10).unwrap_or(100)
}

fn start_spinner(bar: &ProgressBar, message: impl Into<String>) {
    let message = message.into();
    if bar.is_hidden() {
        println!("{message}");
        return;
    }
    bar.reset();
    bar.unset_length();
    bar.set_style(spinner_style());
    bar.set_message(message);
    bar.enable_steady_tick(SPINNER_INTERVAL);
}

fn finish_progress(bar: &ProgressBar, message: impl Into<String>) {
    let message = message.into();
    if bar.is_hidden() {
        println!("{message}");
        return;
    }
    bar.disable_steady_tick();
    bar.finish_and_clear();
    println!("✓ {message}");
}

fn print_above_progress(bar: &ProgressBar, message: String) {
    if bar.is_hidden() {
        eprintln!("{message}");
    } else {
        bar.println(message);
    }
}

fn spinner_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.cyan} {wide_msg}").map_or_else(
        |source| {
            tracing::debug!(error = %source, "failed to configure spinner style");
            ProgressStyle::default_spinner()
        },
        |style| style.tick_strings(SPINNER_TICKS),
    )
}

fn progress_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "{elapsed_precise} [{bar:30.cyan/blue}] {pos}/{len} {msg} (ETA {eta})",
    )
    .unwrap_or_else(|source| {
        tracing::debug!(error = %source, "failed to configure progress bar style");
        ProgressStyle::default_bar()
    })
}

#[cfg(test)]
mod tests {
    use indicatif::{InMemoryTerm, ProgressDrawTarget};

    use super::*;

    #[test]
    fn test_should_render_spinner_bar_reset_and_error_transitions() {
        let terminal = InMemoryTerm::new(10, 100);
        let bar = ProgressBar::with_draw_target(
            None,
            ProgressDrawTarget::term_like(Box::new(terminal.clone())),
        );
        let callbacks = ConsoleIndexWorkflowCallbacks::with_bar(bar, false);
        callbacks.bar.disable_steady_tick();
        callbacks.bar.force_draw();
        assert!(
            terminal
                .contents()
                .contains("Starting indexing runtime preparation")
        );

        callbacks.runtime_prepared();
        assert!(terminal.contents().is_empty());

        callbacks.workflow_started("first_workflow");
        callbacks.bar.disable_steady_tick();
        callbacks.progress("first_workflow", 5, Some(10));
        callbacks.bar.force_draw();
        let progress = terminal.contents();
        assert!(progress.contains("5/10"));
        assert!(progress.contains("first_workflow"));

        callbacks.workflow_completed("first_workflow", &IndexRunStats::default());
        assert!(terminal.contents().is_empty());

        callbacks.workflow_started("second_workflow");
        callbacks.bar.disable_steady_tick();
        callbacks.bar.force_draw();
        assert!(terminal.contents().contains("Starting second_workflow"));

        callbacks.error("second_workflow", "failure");
        assert!(terminal.contents().is_empty());
    }

    #[test]
    fn test_should_bucket_non_interactive_progress_by_ten_percent() {
        assert_eq!(progress_bucket(0, 100), 0);
        assert_eq!(progress_bucket(9, 100), 0);
        assert_eq!(progress_bucket(10, 100), 10);
        assert_eq!(progress_bucket(99, 100), 90);
        assert_eq!(progress_bucket(100, 100), 100);
        assert_eq!(progress_bucket(200, 100), 100);
        assert_eq!(progress_bucket(0, 0), 100);
    }
}
