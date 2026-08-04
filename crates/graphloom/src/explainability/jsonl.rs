//! Bounded, single-writer JSONL persistence for Explainability records.

use std::{
    collections::{HashMap, HashSet},
    io,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

use async_trait::async_trait;
use dashmap::DashMap;
use thiserror::Error;
use tokio::{
    fs::OpenOptions,
    io::{AsyncWrite, AsyncWriteExt, BufWriter},
    sync::{Semaphore, mpsc, oneshot, watch},
    task::{JoinError, JoinHandle},
};

use super::{
    ExplainabilityContractError, ExplainabilityEnvelope, ExplainabilityRecord, ExplainabilityRunId,
    ExplainabilitySink, ExplainabilitySinkError,
};

const DEFAULT_QUEUE_CAPACITY: usize = 256;
const WRITER_RUNNING: u8 = 0;
const WRITER_CLOSING: u8 = 1;
const WRITER_CLOSED: u8 = 2;
const WRITER_FAILED: u8 = 3;
const RUN_ACTIVE: u8 = 0;
const RUN_FINISHING: u8 = 1;
const RUN_FINISHED: u8 = 2;
const RUN_FAILED_WRITER: u8 = 3;
const RUN_FAILED_CLOSED: u8 = 4;
const RUN_FAILED_FINALIZATION: u8 = 5;

/// Configuration for a JSONL Explainability Recorder.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct JsonlExplainabilityOptions {
    path: PathBuf,
    queue_capacity: NonZeroUsize,
}

impl JsonlExplainabilityOptions {
    /// Create options with a bounded queue capacity of 256 records.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            queue_capacity: NonZeroUsize::new(DEFAULT_QUEUE_CAPACITY).unwrap_or(NonZeroUsize::MIN),
        }
    }

    /// Override the bounded input queue capacity.
    #[must_use]
    pub const fn with_queue_capacity(mut self, queue_capacity: NonZeroUsize) -> Self {
        self.queue_capacity = queue_capacity;
        self
    }

    /// Borrow the configured output path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the bounded input queue capacity.
    #[must_use]
    pub const fn queue_capacity(&self) -> NonZeroUsize {
        self.queue_capacity
    }
}

/// Detailed JSONL Recorder creation, writer, and shutdown failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum JsonlExplainabilityError {
    /// The process current directory could not be resolved for a relative output path.
    #[error("resolve Explainability output current directory failed: {source}")]
    CurrentDirectory {
        /// Original I/O failure.
        #[source]
        source: io::Error,
    },
    /// The configured path did not name an output file.
    #[error("Explainability output path must name a file: {path}")]
    InvalidPath {
        /// Rejected output path.
        path: PathBuf,
    },
    /// A required parent directory could not be created.
    #[error("create Explainability output directory {path} failed: {source}")]
    CreateDirectory {
        /// Parent directory path.
        path: PathBuf,
        /// Original I/O failure.
        #[source]
        source: io::Error,
    },
    /// The created parent directory could not be canonicalized.
    #[error("resolve Explainability output directory {path} failed: {source}")]
    ResolveDirectory {
        /// Parent directory path.
        path: PathBuf,
        /// Original I/O failure.
        #[source]
        source: io::Error,
    },
    /// The output file could not be opened with create-new semantics.
    #[error("create new Explainability JSONL output {path} failed: {source}")]
    OpenOutput {
        /// Output file path.
        path: PathBuf,
        /// Original I/O failure.
        #[source]
        source: io::Error,
    },
    /// An Envelope could not be constructed from the existing transport contract.
    #[error("construct Explainability Envelope for {path} failed: {source}")]
    Envelope {
        /// Output file path.
        path: PathBuf,
        /// Contract failure.
        #[source]
        source: ExplainabilityContractError,
    },
    /// An Envelope could not be serialized.
    #[error("serialize Explainability Envelope for {path} failed: {source}")]
    Serialize {
        /// Output file path.
        path: PathBuf,
        /// Serialization failure.
        #[source]
        source: serde_json::Error,
    },
    /// A complete JSONL line could not be written.
    #[error("write Explainability JSONL output {path} failed: {source}")]
    Write {
        /// Output file path.
        path: PathBuf,
        /// Original I/O failure.
        #[source]
        source: io::Error,
    },
    /// The JSONL writer could not flush accepted data.
    #[error("flush Explainability JSONL output {path} failed: {source}")]
    Flush {
        /// Output file path.
        path: PathBuf,
        /// Original I/O failure.
        #[source]
        source: io::Error,
    },
    /// A Run exhausted the non-zero sequence range.
    #[error("Explainability sequence overflow while writing {path}")]
    SequenceOverflow {
        /// Output file path.
        path: PathBuf,
    },
    /// A record reached the writer after that Run was finalized.
    #[error("Explainability writer received a record after Run finalization for {path}")]
    RecordAfterFinish {
        /// Output file path.
        path: PathBuf,
    },
    /// The shutdown command could not be accepted by the writer queue.
    #[error("send Explainability writer shutdown for {path} failed")]
    ShutdownCommand {
        /// Output file path.
        path: PathBuf,
    },
    /// The writer ended before acknowledging shutdown.
    #[error("Explainability writer ended before shutdown confirmation for {path}")]
    WriterEnded {
        /// Output file path.
        path: PathBuf,
    },
    /// The writer task panicked or was cancelled.
    #[error("join Explainability writer for {path} failed: {source}")]
    WriterJoin {
        /// Output file path.
        path: PathBuf,
        /// Tokio task join failure.
        #[source]
        source: JoinError,
    },
}

/// Owner of one bounded JSONL Sink and its single asynchronous writer task.
///
/// The Recorder must be explicitly consumed with [`Self::shutdown`] so writer and flush failures
/// can be returned to the owner. Dropping it does not synthesize a Run terminal event.
#[derive(Debug)]
#[must_use = "the Recorder must be explicitly shut down to observe writer failures"]
pub struct JsonlExplainabilityRecorder {
    path: PathBuf,
    sink: Arc<JsonlExplainabilitySink>,
    writer: JoinHandle<Result<(), JsonlExplainabilityError>>,
}

impl JsonlExplainabilityRecorder {
    /// Create parent directories and a new JSONL output file, then start the single writer.
    ///
    /// Relative paths are resolved against the process current working directory. The output file
    /// is opened with create-new semantics and is never truncated or appended.
    ///
    /// # Errors
    ///
    /// Returns a detailed path or I/O error when the parent directory or new file cannot be
    /// created.
    pub async fn create(
        options: JsonlExplainabilityOptions,
    ) -> Result<Self, JsonlExplainabilityError> {
        let path = prepare_output_path(options.path()).await?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
            .map_err(|source| JsonlExplainabilityError::OpenOutput {
                path: path.clone(),
                source,
            })?;
        let (sender, receiver) = mpsc::channel(options.queue_capacity().get());
        let writer_status = Arc::new(AtomicU8::new(WRITER_RUNNING));
        let runs = Arc::new(DashMap::new());
        let sink = Arc::new(JsonlExplainabilitySink::new(
            sender,
            Arc::clone(&writer_status),
            Arc::clone(&runs),
        ));
        let writer = spawn_writer(
            BufWriter::new(file),
            receiver,
            path.clone(),
            writer_status,
            runs,
            JsonlWriterState::default(),
        );
        Ok(Self { path, sink, writer })
    }

    /// Return the shared reliable Sink owned by this Recorder.
    #[must_use]
    pub fn sink(&self) -> Arc<dyn ExplainabilitySink> {
        Arc::clone(&self.sink) as Arc<dyn ExplainabilitySink>
    }

    /// Borrow the stable absolute JSONL output path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Stop accepting commands, flush all previously accepted records, and await the writer.
    ///
    /// Runs that were not finalized remain truthful unfinished prefixes; shutdown never creates a
    /// `RunCompleted` or `RunFailed` event.
    ///
    /// # Errors
    ///
    /// Returns the detailed writer, flush, shutdown-delivery, or task-join failure.
    pub async fn shutdown(self) -> Result<(), JsonlExplainabilityError> {
        let admission = Arc::clone(&self.sink.admission)
            .acquire_owned()
            .await
            .map_err(|_| JsonlExplainabilityError::ShutdownCommand {
                path: self.path.clone(),
            })?;
        let mut shutdown_send_failed = false;
        let response = if self.sink.writer_status.load(Ordering::Acquire) == WRITER_RUNNING {
            let (response, receiver) = oneshot::channel();
            if let Ok(command_slot) = self.sink.sender.reserve().await {
                if self
                    .sink
                    .writer_status
                    .compare_exchange(
                        WRITER_RUNNING,
                        WRITER_CLOSING,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
                {
                    command_slot.send(JsonlWriterCommand::Shutdown { response });
                    Some(receiver)
                } else {
                    None
                }
            } else {
                shutdown_send_failed = true;
                None
            }
        } else {
            None
        };
        drop(admission);

        let acknowledgement = match response {
            Some(response) => Some(response.await),
            None => None,
        };
        match self.writer.await {
            Ok(Ok(())) if shutdown_send_failed => {
                Err(JsonlExplainabilityError::ShutdownCommand { path: self.path })
            }
            Ok(Ok(())) => match acknowledgement {
                Some(Ok(Ok(()))) => Ok(()),
                Some(Ok(Err(_)) | Err(_)) | None => {
                    Err(JsonlExplainabilityError::WriterEnded { path: self.path })
                }
            },
            Ok(Err(error)) => Err(error),
            Err(source) => Err(JsonlExplainabilityError::WriterJoin {
                path: self.path,
                source,
            }),
        }
    }
}

#[cfg(test)]
#[derive(Debug)]
struct TestSendProbe {
    entered: Semaphore,
    accepted: Semaphore,
    finish_reserving: Semaphore,
}

#[cfg(test)]
impl TestSendProbe {
    fn new() -> Self {
        Self {
            entered: Semaphore::new(0),
            accepted: Semaphore::new(0),
            finish_reserving: Semaphore::new(0),
        }
    }
}

#[derive(Debug)]
struct JsonlExplainabilitySink {
    sender: mpsc::Sender<JsonlWriterCommand>,
    writer_status: Arc<AtomicU8>,
    admission: Arc<Semaphore>,
    runs: Arc<DashMap<ExplainabilityRunId, Arc<RunGate>>>,
    #[cfg(test)]
    send_probe: Option<Arc<TestSendProbe>>,
}

impl JsonlExplainabilitySink {
    fn new(
        sender: mpsc::Sender<JsonlWriterCommand>,
        writer_status: Arc<AtomicU8>,
        runs: Arc<DashMap<ExplainabilityRunId, Arc<RunGate>>>,
    ) -> Self {
        Self {
            sender,
            writer_status,
            admission: Arc::new(Semaphore::new(1)),
            runs,
            #[cfg(test)]
            send_probe: None,
        }
    }

    #[cfg(test)]
    fn with_send_probe(mut self, send_probe: Arc<TestSendProbe>) -> Self {
        self.send_probe = Some(send_probe);
        self
    }

    fn run_gate(&self, run_id: &ExplainabilityRunId) -> Arc<RunGate> {
        Arc::clone(
            self.runs
                .entry(run_id.clone())
                .or_insert_with(|| Arc::new(RunGate::new()))
                .value(),
        )
    }

    fn writer_error(&self) -> ExplainabilitySinkError {
        if self.writer_status.load(Ordering::Acquire) == WRITER_FAILED {
            ExplainabilitySinkError::WriterFailed
        } else {
            ExplainabilitySinkError::Closed
        }
    }
}

#[async_trait]
impl ExplainabilitySink for JsonlExplainabilitySink {
    async fn emit(&self, record: Arc<ExplainabilityRecord>) -> Result<(), ExplainabilitySinkError> {
        let gate = self.run_gate(&record.run_id);
        let run_admission = gate
            .admission
            .acquire()
            .await
            .map_err(|_| ExplainabilitySinkError::Closed)?;
        if gate.state() != RUN_ACTIVE {
            return Err(ExplainabilitySinkError::RecordNotAccepted);
        }
        let admission = self
            .admission
            .acquire()
            .await
            .map_err(|_| ExplainabilitySinkError::Closed)?;
        if self.writer_status.load(Ordering::Acquire) != WRITER_RUNNING {
            return Err(self.writer_error());
        }
        #[cfg(test)]
        if let Some(probe) = &self.send_probe {
            probe.entered.add_permits(1);
        }
        let result = self
            .sender
            .send(JsonlWriterCommand::Record(record))
            .await
            .map_err(|_| self.writer_error());
        #[cfg(test)]
        if result.is_ok()
            && let Some(probe) = &self.send_probe
        {
            probe.accepted.add_permits(1);
        }
        drop(run_admission);
        drop(admission);
        result
    }

    async fn finish_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<(), ExplainabilitySinkError> {
        let gate = self.run_gate(run_id);
        let mut state_changes = gate.subscribe();
        let run_admission = gate
            .admission
            .acquire()
            .await
            .map_err(|_| ExplainabilitySinkError::Closed)?;
        let result = match gate.state() {
            RUN_ACTIVE => {
                let admission = self
                    .admission
                    .acquire()
                    .await
                    .map_err(|_| ExplainabilitySinkError::Closed)?;
                if self.writer_status.load(Ordering::Acquire) != WRITER_RUNNING {
                    let error = self.writer_error();
                    gate.finish(&error);
                    return Err(error);
                }
                #[cfg(test)]
                if let Some(probe) = &self.send_probe {
                    probe.finish_reserving.add_permits(1);
                }
                let Ok(command_slot) = self.sender.reserve().await else {
                    let error = self.writer_error();
                    gate.finish(&error);
                    return Err(error);
                };
                gate.complete(RUN_FINISHING);
                let (response, receiver) = oneshot::channel();
                command_slot.send(JsonlWriterCommand::FinishRun {
                    run_id: run_id.clone(),
                    gate: Arc::clone(&gate),
                    response,
                });
                drop(admission);
                match receiver.await {
                    Ok(result) => result,
                    Err(_) => finish_state_result(gate.state()).map_err(|_| self.writer_error()),
                }
            }
            RUN_FINISHING => {
                drop(run_admission);
                loop {
                    let state = *state_changes.borrow_and_update();
                    if state != RUN_FINISHING {
                        return finish_state_result(state);
                    }
                    state_changes
                        .changed()
                        .await
                        .map_err(|_| self.writer_error())?;
                }
            }
            state => finish_state_result(state),
        };
        drop(run_admission);
        result
    }
}

#[derive(Debug)]
struct RunGate {
    admission: Semaphore,
    state: watch::Sender<u8>,
}

impl RunGate {
    fn new() -> Self {
        Self {
            admission: Semaphore::new(1),
            state: watch::channel(RUN_ACTIVE).0,
        }
    }

    fn state(&self) -> u8 {
        *self.state.borrow()
    }

    fn subscribe(&self) -> watch::Receiver<u8> {
        self.state.subscribe()
    }

    fn finish(&self, error: &ExplainabilitySinkError) {
        let state = match error {
            ExplainabilitySinkError::WriterFailed => RUN_FAILED_WRITER,
            ExplainabilitySinkError::Closed => RUN_FAILED_CLOSED,
            _ => RUN_FAILED_FINALIZATION,
        };
        self.complete(state);
    }

    fn complete(&self, state: u8) {
        self.state.send_replace(state);
    }
}

fn finish_state_result(state: u8) -> Result<(), ExplainabilitySinkError> {
    match state {
        RUN_FINISHED => Ok(()),
        RUN_FAILED_WRITER => Err(ExplainabilitySinkError::WriterFailed),
        RUN_FAILED_CLOSED => Err(ExplainabilitySinkError::Closed),
        _ => Err(ExplainabilitySinkError::RunFinalizationFailed),
    }
}

#[derive(Debug)]
enum JsonlWriterCommand {
    Record(Arc<ExplainabilityRecord>),
    FinishRun {
        run_id: ExplainabilityRunId,
        gate: Arc<RunGate>,
        response: oneshot::Sender<Result<(), ExplainabilitySinkError>>,
    },
    Shutdown {
        response: oneshot::Sender<Result<(), ExplainabilitySinkError>>,
    },
}

#[derive(Debug, Default)]
struct JsonlWriterState {
    sequences: HashMap<ExplainabilityRunId, u64>,
    finished_runs: HashSet<ExplainabilityRunId>,
}

#[derive(Debug)]
struct WriterStatusGuard {
    writer_status: Arc<AtomicU8>,
    runs: Arc<DashMap<ExplainabilityRunId, Arc<RunGate>>>,
}

impl WriterStatusGuard {
    fn new(
        writer_status: Arc<AtomicU8>,
        runs: Arc<DashMap<ExplainabilityRunId, Arc<RunGate>>>,
    ) -> Self {
        Self {
            writer_status,
            runs,
        }
    }
}

impl Drop for WriterStatusGuard {
    fn drop(&mut self) {
        if self.writer_status.load(Ordering::Acquire) == WRITER_CLOSED {
            return;
        }
        self.writer_status.store(WRITER_FAILED, Ordering::Release);
        for gate in self.runs.iter() {
            if gate.state() == RUN_FINISHING {
                gate.finish(&ExplainabilitySinkError::WriterFailed);
            }
        }
    }
}

fn spawn_writer<W>(
    output: W,
    receiver: mpsc::Receiver<JsonlWriterCommand>,
    path: PathBuf,
    writer_status: Arc<AtomicU8>,
    runs: Arc<DashMap<ExplainabilityRunId, Arc<RunGate>>>,
    state: JsonlWriterState,
) -> JoinHandle<Result<(), JsonlExplainabilityError>>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let _status_guard = WriterStatusGuard::new(Arc::clone(&writer_status), runs);
        run_writer(output, receiver, path, writer_status, state).await
    })
}

async fn prepare_output_path(path: &Path) -> Result<PathBuf, JsonlExplainabilityError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|source| JsonlExplainabilityError::CurrentDirectory { source })?
            .join(path)
    };
    let file_name = absolute
        .file_name()
        .ok_or_else(|| JsonlExplainabilityError::InvalidPath {
            path: absolute.clone(),
        })?;
    let parent = absolute
        .parent()
        .ok_or_else(|| JsonlExplainabilityError::InvalidPath {
            path: absolute.clone(),
        })?;
    tokio::fs::create_dir_all(parent).await.map_err(|source| {
        JsonlExplainabilityError::CreateDirectory {
            path: parent.to_path_buf(),
            source,
        }
    })?;
    let resolved_parent = tokio::fs::canonicalize(parent).await.map_err(|source| {
        JsonlExplainabilityError::ResolveDirectory {
            path: parent.to_path_buf(),
            source,
        }
    })?;
    Ok(resolved_parent.join(file_name))
}

async fn run_writer<W>(
    mut output: W,
    mut receiver: mpsc::Receiver<JsonlWriterCommand>,
    path: PathBuf,
    writer_status: Arc<AtomicU8>,
    mut state: JsonlWriterState,
) -> Result<(), JsonlExplainabilityError>
where
    W: AsyncWrite + Unpin,
{
    while let Some(command) = receiver.recv().await {
        match command {
            JsonlWriterCommand::Record(record) => {
                if state.finished_runs.contains(&record.run_id) {
                    return writer_failure(
                        &writer_status,
                        JsonlExplainabilityError::RecordAfterFinish { path },
                    );
                }
                let sequence = next_sequence(state.sequences.get(&record.run_id).copied(), &path)
                    .map_err(|error| mark_writer_failed(&writer_status, error))?;
                let run_id = record.run_id.clone();
                write_record(&mut output, record, sequence, &path)
                    .await
                    .map_err(|error| mark_writer_failed(&writer_status, error))?;
                state.sequences.insert(run_id, sequence);
            }
            JsonlWriterCommand::FinishRun {
                run_id,
                gate,
                response,
            } => {
                let result = flush_output(&mut output, &path).await;
                if result.is_ok() {
                    state.finished_runs.insert(run_id);
                    gate.complete(RUN_FINISHED);
                    let _ = response.send(Ok(()));
                } else if let Err(error) = result {
                    writer_status.store(WRITER_FAILED, Ordering::Release);
                    gate.finish(&ExplainabilitySinkError::WriterFailed);
                    let _ = response.send(Err(ExplainabilitySinkError::WriterFailed));
                    return Err(error);
                }
            }
            JsonlWriterCommand::Shutdown { response } => {
                let result = flush_output(&mut output, &path).await;
                match result {
                    Ok(()) => {
                        writer_status.store(WRITER_CLOSED, Ordering::Release);
                        let _ = response.send(Ok(()));
                        return Ok(());
                    }
                    Err(error) => {
                        writer_status.store(WRITER_FAILED, Ordering::Release);
                        let _ = response.send(Err(ExplainabilitySinkError::WriterFailed));
                        return Err(error);
                    }
                }
            }
        }
    }
    writer_failure(
        &writer_status,
        JsonlExplainabilityError::WriterEnded { path },
    )
}

fn next_sequence(previous: Option<u64>, path: &Path) -> Result<u64, JsonlExplainabilityError> {
    previous.unwrap_or_default().checked_add(1).ok_or_else(|| {
        JsonlExplainabilityError::SequenceOverflow {
            path: path.to_path_buf(),
        }
    })
}

async fn write_record<W>(
    output: &mut W,
    record: Arc<ExplainabilityRecord>,
    sequence: u64,
    path: &Path,
) -> Result<(), JsonlExplainabilityError>
where
    W: AsyncWrite + Unpin,
{
    let record = match Arc::try_unwrap(record) {
        Ok(record) => record,
        Err(record) => record.as_ref().clone(),
    };
    let envelope = ExplainabilityEnvelope::new(sequence, record).map_err(|source| {
        JsonlExplainabilityError::Envelope {
            path: path.to_path_buf(),
            source,
        }
    })?;
    let mut line =
        serde_json::to_vec(&envelope).map_err(|source| JsonlExplainabilityError::Serialize {
            path: path.to_path_buf(),
            source,
        })?;
    line.push(b'\n');
    output
        .write_all(&line)
        .await
        .map_err(|source| JsonlExplainabilityError::Write {
            path: path.to_path_buf(),
            source,
        })?;
    flush_output(output, path).await
}

async fn flush_output<W>(output: &mut W, path: &Path) -> Result<(), JsonlExplainabilityError>
where
    W: AsyncWrite + Unpin,
{
    output
        .flush()
        .await
        .map_err(|source| JsonlExplainabilityError::Flush {
            path: path.to_path_buf(),
            source,
        })
}

fn mark_writer_failed(
    writer_status: &AtomicU8,
    error: JsonlExplainabilityError,
) -> JsonlExplainabilityError {
    writer_status.store(WRITER_FAILED, Ordering::Release);
    error
}

fn writer_failure(
    writer_status: &AtomicU8,
    error: JsonlExplainabilityError,
) -> Result<(), JsonlExplainabilityError> {
    Err(mark_writer_failed(writer_status, error))
}

#[cfg(test)]
mod tests {
    use std::{
        io::{self, ErrorKind},
        path::{Path, PathBuf},
        pin::Pin,
        str::FromStr,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
        },
        task::{Context, Poll},
    };

    use chrono::Utc;
    use dashmap::DashMap;
    use futures_util::task::AtomicWaker;
    use tempfile::TempDir;
    use tokio::{
        io::AsyncWrite,
        sync::{Semaphore, mpsc},
    };

    use super::{
        JsonlExplainabilityError, JsonlExplainabilityOptions, JsonlExplainabilityRecorder,
        JsonlExplainabilitySink, JsonlWriterCommand, JsonlWriterState, RUN_ACTIVE, RUN_FINISHED,
        TestSendProbe, WRITER_FAILED, WRITER_RUNNING, next_sequence, run_writer, spawn_writer,
    };
    use crate::explainability::{
        ExplainabilityContentMode, ExplainabilityEnvelope, ExplainabilityEvent,
        ExplainabilityQueryMethod, ExplainabilityRecord, ExplainabilityRunId,
        ExplainabilityRunKind, ExplainabilitySink, ExplainabilitySinkError, ExplainabilitySpanId,
        QueryStarted, RunCompleted, RunStarted,
    };

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

    fn record(
        run_id: &str,
        span_id: &str,
        event: ExplainabilityEvent,
    ) -> TestResult<Arc<ExplainabilityRecord>> {
        Ok(Arc::new(ExplainabilityRecord::new(
            ExplainabilityRunId::from_str(run_id)?,
            Utc::now(),
            ExplainabilitySpanId::from_str(span_id)?,
            None,
            event,
        )))
    }

    fn run_started(run_id: &str, span_id: &str) -> TestResult<Arc<ExplainabilityRecord>> {
        record(
            run_id,
            span_id,
            ExplainabilityEvent::RunStarted(RunStarted::new(
                ExplainabilityRunKind::Query,
                ExplainabilityContentMode::Metadata,
            )),
        )
    }

    fn query_started(run_id: &str, span_id: &str) -> TestResult<Arc<ExplainabilityRecord>> {
        record(
            run_id,
            span_id,
            ExplainabilityEvent::QueryStarted(QueryStarted::new(ExplainabilityQueryMethod::Local)),
        )
    }

    fn run_completed(run_id: &str, span_id: &str) -> TestResult<Arc<ExplainabilityRecord>> {
        record(
            run_id,
            span_id,
            ExplainabilityEvent::RunCompleted(RunCompleted::new(12)),
        )
    }

    async fn envelopes(path: &Path) -> TestResult<Vec<ExplainabilityEnvelope>> {
        let bytes = tokio::fs::read(path).await?;
        bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).map_err(Into::into))
            .collect()
    }

    #[tokio::test]
    async fn test_should_write_single_run_as_compact_lf_jsonl() -> TestResult {
        let directory = TempDir::new()?;
        let path = directory.path().join("nested/run.jsonl");
        let recorder =
            JsonlExplainabilityRecorder::create(JsonlExplainabilityOptions::new(path.clone()))
                .await?;
        let sink = recorder.sink();
        let run_id = ExplainabilityRunId::from_str("run-a")?;
        sink.emit(run_started("run-a", "span-1")?).await?;
        sink.emit(query_started("run-a", "span-2")?).await?;
        sink.emit(run_completed("run-a", "span-3")?).await?;
        sink.finish_run(&run_id).await?;

        let bytes = tokio::fs::read(recorder.path()).await?;
        assert!(bytes.ends_with(b"\n"));
        assert!(!bytes.windows(2).any(|window| window == b"\r\n"));
        assert!(!bytes.contains(&b' '));
        let values = envelopes(recorder.path()).await?;
        assert_eq!(
            values
                .iter()
                .map(ExplainabilityEnvelope::sequence)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(values.iter().all(|value| value.schema_version() == 1));
        assert!(values.iter().all(|value| value.record.run_id == run_id));
        assert!(matches!(
            values[0].record.event,
            ExplainabilityEvent::RunStarted(_)
        ));
        assert!(matches!(
            values[1].record.event,
            ExplainabilityEvent::QueryStarted(_)
        ));
        assert!(matches!(
            values[2].record.event,
            ExplainabilityEvent::RunCompleted(_)
        ));
        recorder.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_should_sequence_interleaved_runs_independently() -> TestResult {
        let directory = TempDir::new()?;
        let path = directory.path().join("runs.jsonl");
        let recorder =
            JsonlExplainabilityRecorder::create(JsonlExplainabilityOptions::new(path)).await?;
        let sink = recorder.sink();
        sink.emit(run_started("run-a", "a-1")?).await?;
        sink.emit(run_started("run-b", "b-1")?).await?;
        sink.emit(query_started("run-a", "a-2")?).await?;
        sink.emit(query_started("run-b", "b-2")?).await?;
        sink.finish_run(&ExplainabilityRunId::from_str("run-a")?)
            .await?;
        sink.finish_run(&ExplainabilityRunId::from_str("run-b")?)
            .await?;
        let values = envelopes(recorder.path()).await?;
        assert_eq!(
            values
                .iter()
                .map(|value| (value.record.run_id.as_str(), value.sequence()))
                .collect::<Vec<_>>(),
            vec![("run-a", 1), ("run-b", 1), ("run-a", 2), ("run-b", 2)]
        );
        recorder.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_should_flush_finish_reject_late_record_and_keep_other_run_open() -> TestResult {
        let directory = TempDir::new()?;
        let path = directory.path().join("finish.jsonl");
        let recorder =
            JsonlExplainabilityRecorder::create(JsonlExplainabilityOptions::new(path)).await?;
        let sink = recorder.sink();
        let run_a = ExplainabilityRunId::from_str("run-a")?;
        sink.emit(run_started("run-a", "a-1")?).await?;
        sink.finish_run(&run_a).await?;
        assert_eq!(envelopes(recorder.path()).await?.len(), 1);
        assert_eq!(
            sink.emit(query_started("run-a", "a-2")?).await,
            Err(ExplainabilitySinkError::RecordNotAccepted)
        );
        sink.finish_run(&run_a).await?;
        sink.emit(run_started("run-b", "b-1")?).await?;
        sink.finish_run(&ExplainabilityRunId::from_str("run-b")?)
            .await?;
        let values = envelopes(recorder.path()).await?;
        assert_eq!(values.len(), 2);
        assert_eq!(values[1].sequence(), 1);
        recorder.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_should_make_concurrent_duplicate_finish_stable() -> TestResult {
        let directory = TempDir::new()?;
        let recorder = JsonlExplainabilityRecorder::create(JsonlExplainabilityOptions::new(
            directory.path().join("duplicate.jsonl"),
        ))
        .await?;
        let sink = recorder.sink();
        let run_id = ExplainabilityRunId::from_str("run-a")?;
        sink.emit(run_started("run-a", "a-1")?).await?;
        let first_sink = Arc::clone(&sink);
        let first_run = run_id.clone();
        let first = tokio::spawn(async move { first_sink.finish_run(&first_run).await });
        let second_sink = Arc::clone(&sink);
        let second_run = run_id.clone();
        let second = tokio::spawn(async move { second_sink.finish_run(&second_run).await });
        first.await??;
        second.await??;
        assert_eq!(envelopes(recorder.path()).await?.len(), 1);
        recorder.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_should_preserve_existing_file_and_create_parent_directory() -> TestResult {
        let directory = TempDir::new()?;
        let existing = directory.path().join("existing.jsonl");
        tokio::fs::write(&existing, b"do-not-overwrite").await?;
        let result =
            JsonlExplainabilityRecorder::create(JsonlExplainabilityOptions::new(existing.clone()))
                .await;
        assert!(matches!(
            result,
            Err(JsonlExplainabilityError::OpenOutput { source, .. })
                if source.kind() == ErrorKind::AlreadyExists
        ));
        assert_eq!(tokio::fs::read(&existing).await?, b"do-not-overwrite");

        let nested = directory.path().join("missing/parents/run.jsonl");
        let recorder =
            JsonlExplainabilityRecorder::create(JsonlExplainabilityOptions::new(nested.clone()))
                .await?;
        assert!(nested.parent().is_some_and(Path::is_dir));
        recorder.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_should_shutdown_unfinished_run_without_synthetic_terminal_event() -> TestResult {
        let directory = TempDir::new()?;
        let path = directory.path().join("unfinished.jsonl");
        let recorder =
            JsonlExplainabilityRecorder::create(JsonlExplainabilityOptions::new(path.clone()))
                .await?;
        let sink = recorder.sink();
        sink.emit(run_started("run-a", "a-1")?).await?;
        sink.emit(query_started("run-a", "a-2")?).await?;
        recorder.shutdown().await?;
        let values = envelopes(&path).await?;
        assert_eq!(values.len(), 2);
        assert!(values.iter().all(|value| !matches!(
            value.record.event,
            ExplainabilityEvent::RunCompleted(_) | ExplainabilityEvent::RunFailed(_)
        )));
        Ok(())
    }

    #[tokio::test]
    async fn test_should_apply_bounded_backpressure_without_dropping_records() -> TestResult {
        let (sender, mut receiver) = mpsc::channel(1);
        let status = Arc::new(AtomicU8::new(WRITER_RUNNING));
        let sink = JsonlExplainabilitySink::new(
            sender.clone(),
            Arc::clone(&status),
            Arc::new(DashMap::new()),
        );
        sink.emit(run_started("run-a", "a-1")?).await?;

        let probe = Arc::new(TestSendProbe::new());
        let waiting_sink = JsonlExplainabilitySink::new(sender, status, Arc::new(DashMap::new()))
            .with_send_probe(Arc::clone(&probe));
        let waiting = query_started("run-b", "b-1")?;
        let task = tokio::spawn(async move { waiting_sink.emit(waiting).await });
        let entered = probe.entered.acquire().await?;
        entered.forget();
        assert!(probe.accepted.try_acquire().is_err());

        assert!(matches!(
            receiver.recv().await,
            Some(super::JsonlWriterCommand::Record(_))
        ));
        let accepted = probe.accepted.acquire().await?;
        accepted.forget();
        task.await??;
        assert!(matches!(
            receiver.recv().await,
            Some(super::JsonlWriterCommand::Record(_))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn test_should_retry_finish_cancelled_before_queue_acceptance() -> TestResult {
        let (sender, mut receiver) = mpsc::channel(1);
        let status = Arc::new(AtomicU8::new(WRITER_RUNNING));
        let primary = JsonlExplainabilitySink::new(
            sender.clone(),
            Arc::clone(&status),
            Arc::new(DashMap::new()),
        );
        primary.emit(run_started("run-a", "a-1")?).await?;

        let probe = Arc::new(TestSendProbe::new());
        let runs = Arc::new(DashMap::new());
        let sink = Arc::new(
            JsonlExplainabilitySink::new(sender, status, Arc::clone(&runs))
                .with_send_probe(Arc::clone(&probe)),
        );
        let run_id = ExplainabilityRunId::from_str("run-b")?;
        let first_sink = Arc::clone(&sink);
        let first_run = run_id.clone();
        let first = tokio::spawn(async move { first_sink.finish_run(&first_run).await });
        let reserving = probe.finish_reserving.acquire().await?;
        reserving.forget();
        first.abort();
        assert!(first.await.is_err());
        assert_eq!(sink.run_gate(&run_id).state(), RUN_ACTIVE);

        assert!(matches!(
            receiver.recv().await,
            Some(JsonlWriterCommand::Record(_))
        ));
        let retry_sink = Arc::clone(&sink);
        let retry_run = run_id.clone();
        let retry = tokio::spawn(async move { retry_sink.finish_run(&retry_run).await });
        let Some(JsonlWriterCommand::FinishRun { gate, response, .. }) = receiver.recv().await
        else {
            return Err("expected retried FinishRun command".into());
        };
        gate.complete(RUN_FINISHED);
        let _ = response.send(Ok(()));
        retry.await??;
        assert_eq!(sink.run_gate(&run_id).state(), RUN_FINISHED);
        Ok(())
    }

    #[tokio::test]
    async fn test_should_stop_writer_before_sequence_overflow_output() -> TestResult {
        let path = PathBuf::from("overflow.jsonl");
        assert_eq!(next_sequence(None, &path).ok(), Some(1));
        let run_id = ExplainabilityRunId::from_str("overflow-run")?;
        let (sender, receiver) = mpsc::channel(2);
        sender
            .send(JsonlWriterCommand::Record(run_started(
                "overflow-run",
                "span-1",
            )?))
            .await?;
        sender
            .send(JsonlWriterCommand::Record(query_started(
                "overflow-run",
                "span-2",
            )?))
            .await?;
        drop(sender);
        let writes = Arc::new(AtomicUsize::new(0));
        let writer_status = Arc::new(AtomicU8::new(WRITER_RUNNING));
        let mut state = JsonlWriterState::default();
        state.sequences.insert(run_id, u64::MAX);
        let result = run_writer(
            CountingWriter::new(Arc::clone(&writes)),
            receiver,
            path,
            Arc::clone(&writer_status),
            state,
        )
        .await;
        assert!(matches!(
            result,
            Err(JsonlExplainabilityError::SequenceOverflow { .. })
        ));
        assert_eq!(writer_status.load(Ordering::Acquire), WRITER_FAILED);
        assert_eq!(writes.load(Ordering::Acquire), 0);
        Ok(())
    }

    #[tokio::test]
    async fn test_should_finalize_run_when_finish_future_is_cancelled() -> TestResult {
        let control = Arc::new(FlushControl::new());
        let recorder = test_recorder(
            GatedFlushWriter::new(Arc::clone(&control)),
            PathBuf::from("cancelled-finish.jsonl"),
        );
        let sink = recorder.sink();
        let run_id = ExplainabilityRunId::from_str("run-a")?;
        sink.emit(run_started("run-a", "a-1")?).await?;
        let finishing_sink = Arc::clone(&sink);
        let finishing_run = run_id.clone();
        let task = tokio::spawn(async move { finishing_sink.finish_run(&finishing_run).await });
        let entered = control.entered.acquire().await?;
        entered.forget();
        task.abort();
        assert!(task.await.is_err());
        control.release();
        sink.finish_run(&run_id).await?;
        recorder.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_should_mark_panicked_writer_failed_and_return_join_error() -> TestResult {
        let recorder = test_recorder(PanicWriter, PathBuf::from("panic.jsonl"));
        let sink = recorder.sink();
        sink.emit(run_started("run-a", "a-1")?).await?;
        let run_id = ExplainabilityRunId::from_str("run-a")?;
        assert_eq!(
            sink.finish_run(&run_id).await,
            Err(ExplainabilitySinkError::WriterFailed)
        );
        assert_eq!(
            sink.emit(run_started("run-b", "b-1")?).await,
            Err(ExplainabilitySinkError::WriterFailed)
        );
        assert!(matches!(
            recorder.shutdown().await,
            Err(JsonlExplainabilityError::WriterJoin { .. })
        ));
        Ok(())
    }

    #[tokio::test]
    async fn test_should_return_detailed_write_and_flush_failures() -> TestResult {
        let write = failing_recorder(FailingWriter::write(), PathBuf::from("write.jsonl"));
        let write_sink = write.sink();
        let run_id = ExplainabilityRunId::from_str("run-a")?;
        write_sink.emit(run_started("run-a", "a-1")?).await?;
        assert_eq!(
            write_sink.finish_run(&run_id).await,
            Err(ExplainabilitySinkError::WriterFailed)
        );
        assert_eq!(
            write_sink.finish_run(&run_id).await,
            Err(ExplainabilitySinkError::WriterFailed)
        );
        assert_eq!(
            write_sink.emit(query_started("run-a", "a-2")?).await,
            Err(ExplainabilitySinkError::RecordNotAccepted)
        );
        assert_eq!(
            write_sink.emit(run_started("run-b", "b-1")?).await,
            Err(ExplainabilitySinkError::WriterFailed)
        );
        assert!(matches!(
            write.shutdown().await,
            Err(JsonlExplainabilityError::Write { .. })
        ));

        let flush = failing_recorder(FailingWriter::flush(), PathBuf::from("flush.jsonl"));
        let flush_sink = flush.sink();
        let flush_run = ExplainabilityRunId::from_str("run-c")?;
        flush_sink.emit(run_started("run-c", "c-1")?).await?;
        assert_eq!(
            flush_sink.finish_run(&flush_run).await,
            Err(ExplainabilitySinkError::WriterFailed)
        );
        assert_eq!(
            flush_sink.finish_run(&flush_run).await,
            Err(ExplainabilitySinkError::WriterFailed)
        );
        assert!(matches!(
            flush.shutdown().await,
            Err(JsonlExplainabilityError::Flush { .. })
        ));
        Ok(())
    }

    fn failing_recorder(output: FailingWriter, path: PathBuf) -> JsonlExplainabilityRecorder {
        test_recorder(output, path)
    }

    fn test_recorder<W>(output: W, path: PathBuf) -> JsonlExplainabilityRecorder
    where
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel(1);
        let status = Arc::new(AtomicU8::new(WRITER_RUNNING));
        let runs = Arc::new(DashMap::new());
        let sink = Arc::new(JsonlExplainabilitySink::new(
            sender,
            Arc::clone(&status),
            Arc::clone(&runs),
        ));
        let writer = spawn_writer(
            output,
            receiver,
            path.clone(),
            status,
            runs,
            JsonlWriterState::default(),
        );
        JsonlExplainabilityRecorder { path, sink, writer }
    }

    #[derive(Debug)]
    struct FlushControl {
        entered: Semaphore,
        released: AtomicBool,
        waker: AtomicWaker,
    }

    impl FlushControl {
        fn new() -> Self {
            Self {
                entered: Semaphore::new(0),
                released: AtomicBool::new(false),
                waker: AtomicWaker::new(),
            }
        }

        fn release(&self) {
            self.released.store(true, Ordering::Release);
            self.waker.wake();
        }
    }

    #[derive(Debug)]
    struct GatedFlushWriter {
        control: Arc<FlushControl>,
        completed_flushes: usize,
        announced: bool,
    }

    impl GatedFlushWriter {
        fn new(control: Arc<FlushControl>) -> Self {
            Self {
                control,
                completed_flushes: 0,
                announced: false,
            }
        }
    }

    impl AsyncWrite for GatedFlushWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Ready(Ok(buffer.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
            let writer = self.get_mut();
            if writer.completed_flushes == 0 {
                writer.completed_flushes = 1;
                return Poll::Ready(Ok(()));
            }
            if writer.control.released.load(Ordering::Acquire) {
                writer.completed_flushes = writer.completed_flushes.saturating_add(1);
                return Poll::Ready(Ok(()));
            }
            if !writer.announced {
                writer.announced = true;
                writer.control.entered.add_permits(1);
            }
            writer.control.waker.register(context.waker());
            if writer.control.released.load(Ordering::Acquire) {
                context.waker().wake_by_ref();
            }
            Poll::Pending
        }

        fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[derive(Debug)]
    struct CountingWriter {
        writes: Arc<AtomicUsize>,
    }

    impl CountingWriter {
        fn new(writes: Arc<AtomicUsize>) -> Self {
            Self { writes }
        }
    }

    impl AsyncWrite for CountingWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.writes.fetch_add(1, Ordering::AcqRel);
            Poll::Ready(Ok(buffer.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[derive(Debug)]
    struct PanicWriter;

    impl AsyncWrite for PanicWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            _buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            panic!("forced writer panic")
        }

        fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[derive(Debug)]
    struct FailingWriter {
        fail_write: bool,
        fail_flush: bool,
    }

    impl FailingWriter {
        const fn write() -> Self {
            Self {
                fail_write: true,
                fail_flush: false,
            }
        }

        const fn flush() -> Self {
            Self {
                fail_write: false,
                fail_flush: true,
            }
        }
    }

    impl AsyncWrite for FailingWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            if self.fail_write {
                Poll::Ready(Err(io::Error::new(
                    ErrorKind::BrokenPipe,
                    "forced JSONL write failure",
                )))
            } else {
                Poll::Ready(Ok(buffer.len()))
            }
        }

        fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            if self.fail_flush {
                Poll::Ready(Err(io::Error::new(
                    ErrorKind::BrokenPipe,
                    "forced JSONL flush failure",
                )))
            } else {
                Poll::Ready(Ok(()))
            }
        }

        fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
}
