//! Bounded, single-writer persistence adapter from [`ExplainabilitySink`] to
//! an [`ExplainabilityStore`].
//!
//! `StoreExplainabilityRecorder` owns one bounded command queue and one writer
//! task. The host explicitly creates run metadata through
//! [`StoreExplainabilityRecorder::create_run`], Core emits immutable records
//! through the sink, `finish_run` acts as a persistence barrier only, and the
//! host explicitly transitions Store run metadata with
//! [`StoreExplainabilityRecorder::complete_run`]. The writer owns sequence
//! allocation and constructs every [`ExplainabilityEnvelope`]; nothing else
//! assigns or rewrites sequences.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

use async_trait::async_trait;
use dashmap::DashMap;
use thiserror::Error;
use tokio::{
    sync::{Semaphore, mpsc, oneshot, watch},
    task::{JoinError, JoinHandle},
};

use super::{
    ExplainabilityContractError, ExplainabilityEnvelope, ExplainabilityRecord, ExplainabilityRun,
    ExplainabilityRunId, ExplainabilitySink, ExplainabilitySinkError, ExplainabilityStore,
    ExplainabilityStoreError, RunCompletion, live_hub::ExplainabilityLiveHub,
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

/// Configuration for a [`StoreExplainabilityRecorder`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct StoreExplainabilityOptions {
    queue_capacity: NonZeroUsize,
}

impl StoreExplainabilityOptions {
    /// Create options with a bounded queue capacity of 256 commands.
    #[must_use]
    pub fn new() -> Self {
        Self {
            queue_capacity: NonZeroUsize::new(DEFAULT_QUEUE_CAPACITY).unwrap_or(NonZeroUsize::MIN),
        }
    }

    /// Override the bounded command queue capacity.
    #[must_use]
    pub const fn with_queue_capacity(mut self, queue_capacity: NonZeroUsize) -> Self {
        self.queue_capacity = queue_capacity;
        self
    }

    /// Return the bounded command queue capacity.
    #[must_use]
    pub const fn queue_capacity(&self) -> NonZeroUsize {
        self.queue_capacity
    }
}

impl Default for StoreExplainabilityOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Low-cardinality Store operation performed on behalf of the writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreExplainabilityOperation {
    /// Create a new run.
    CreateRun,
    /// Append one persisted envelope.
    AppendEvents,
    /// Complete one run with terminal metadata.
    CompleteRun,
}

impl fmt::Display for StoreExplainabilityOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CreateRun => "create run",
            Self::AppendEvents => "append events",
            Self::CompleteRun => "complete run",
        })
    }
}

/// Safe recorder-level failure. Never contains record payloads, query text,
/// event content, database paths, or secrets.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StoreExplainabilityError {
    /// The run is already registered with this recorder.
    #[error("explainability run is already registered with this recorder")]
    RunAlreadyRegistered,
    /// The run is not registered with this recorder.
    #[error("explainability run is not registered with this recorder")]
    RunNotRegistered,
    /// The run must be finalized before completion metadata can be applied.
    #[error("explainability run must be finalized before completion")]
    RunNotFinalized,
    /// An envelope could not be constructed.
    #[error("construct explainability envelope failed")]
    Envelope {
        /// Underlying contract failure.
        #[source]
        source: ExplainabilityContractError,
    },
    /// A Store operation failed.
    #[error("store operation {operation} failed: {source}")]
    Store {
        /// Store operation being performed.
        operation: StoreExplainabilityOperation,
        /// Underlying Store failure.
        #[source]
        source: ExplainabilityStoreError,
    },
    /// The per-run sequence space is exhausted.
    #[error("explainability sequence overflow")]
    SequenceOverflow,
    /// The writer received a record after that run was finalized.
    #[error("explainability writer received a record after run finalization")]
    RecordAfterFinish,
    /// A writer command could not be accepted because the writer is closing.
    #[error("send explainability writer command failed")]
    ShutdownCommand,
    /// The writer ended before confirming a command.
    #[error("explainability writer ended before command confirmation")]
    WriterEnded,
    /// The writer task panicked or was cancelled.
    #[error("join explainability writer failed")]
    WriterJoin {
        /// Tokio task join failure.
        #[source]
        source: JoinError,
    },
    /// No Tokio runtime is available to host the persistence writer.
    #[error("a Tokio runtime is required to start the explainability persistence writer")]
    RuntimeUnavailable,
}

/// Owner of one bounded Store Sink and its single writer task.
///
/// `create_run` and `complete_run` are explicit control operations; `sink()`
/// provides the reliable Core-facing [`ExplainabilitySink`]. The Recorder must
/// be consumed with [`Self::shutdown`] so writer failures can be observed.
#[non_exhaustive]
pub struct StoreExplainabilityRecorder {
    sink: Arc<StoreExplainabilitySink>,
    writer: JoinHandle<Result<(), StoreExplainabilityError>>,
}

impl fmt::Debug for StoreExplainabilityRecorder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StoreExplainabilityRecorder { .. }")
    }
}

impl StoreExplainabilityRecorder {
    /// Start a bounded Store persistence writer.
    ///
    /// The writer task is spawned on the current Tokio runtime, so this
    /// constructor must be called from inside an active runtime.
    ///
    /// # Errors
    ///
    /// Returns [`StoreExplainabilityError::RuntimeUnavailable`] when called
    /// outside an active Tokio runtime. In that case no writer task is
    /// spawned, no background resource is created, and the Store is not
    /// touched.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "public constructor takes owned options so callers can chain builder methods"
    )]
    pub fn new(
        store: Arc<dyn ExplainabilityStore>,
        options: StoreExplainabilityOptions,
    ) -> Result<Self, StoreExplainabilityError> {
        Self::start(store, None, &options)
    }

    /// Start a bounded Store persistence writer with post-persistence realtime fan-out.
    ///
    /// The Hub is a runtime dependency rather than recorder configuration. The writer registers
    /// runs after Store creation, publishes only after durable append and sequence commit, and
    /// closes its registered channels on finish, shutdown, or fatal failure.
    ///
    /// # Errors
    ///
    /// Returns [`StoreExplainabilityError::RuntimeUnavailable`] when called outside an active
    /// Tokio runtime. Its runtime behavior otherwise matches [`Self::new`].
    #[allow(
        clippy::needless_pass_by_value,
        reason = "public constructor takes owned options so callers can chain builder methods"
    )]
    pub fn new_with_live_hub(
        store: Arc<dyn ExplainabilityStore>,
        live_hub: Arc<ExplainabilityLiveHub>,
        options: StoreExplainabilityOptions,
    ) -> Result<Self, StoreExplainabilityError> {
        Self::start(store, Some(live_hub), &options)
    }

    fn start(
        store: Arc<dyn ExplainabilityStore>,
        live_hub: Option<Arc<ExplainabilityLiveHub>>,
        options: &StoreExplainabilityOptions,
    ) -> Result<Self, StoreExplainabilityError> {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| StoreExplainabilityError::RuntimeUnavailable)?;
        let (sender, receiver) = mpsc::channel(options.queue_capacity().get());
        let writer_status = Arc::new(AtomicU8::new(WRITER_RUNNING));
        let runs = Arc::new(DashMap::new());
        let sink = Arc::new(StoreExplainabilitySink::new(
            sender,
            Arc::clone(&writer_status),
            Arc::clone(&runs),
        ));
        let writer = runtime.spawn(async move {
            let _status_guard = WriterStatusGuard::new(
                Arc::clone(&writer_status),
                Arc::clone(&runs),
                live_hub.clone(),
            );
            run_writer(store, live_hub, receiver, writer_status, runs).await
        });
        Ok(Self { sink, writer })
    }

    /// Return the shared reliable Sink owned by this Recorder.
    #[must_use]
    pub fn sink(&self) -> Arc<dyn ExplainabilitySink> {
        Arc::clone(&self.sink) as Arc<dyn ExplainabilitySink>
    }

    /// Explicitly create run metadata in the Store before any event is emitted.
    ///
    /// Returns only after the Store confirmed the run exists. Emits for this
    /// run are rejected until this future resolves successfully.
    ///
    /// # Errors
    ///
    /// Returns a recorder error when the run is already registered, the
    /// writer is shutting down, or the Store rejected the run.
    pub async fn create_run(&self, run: ExplainabilityRun) -> Result<(), StoreExplainabilityError> {
        let admission = Arc::clone(&self.sink.admission)
            .acquire_owned()
            .await
            .map_err(|_| StoreExplainabilityError::ShutdownCommand)?;
        let response = if self.sink.writer_status.load(Ordering::Acquire) == WRITER_RUNNING {
            let (response, receiver) = oneshot::channel();
            match self
                .sink
                .sender
                .send(StoreWriterCommand::CreateRun { run, response })
                .await
            {
                Ok(()) => Some(receiver),
                Err(_) => None,
            }
        } else {
            None
        };
        drop(admission);
        match response {
            Some(receiver) => receiver
                .await
                .map_err(|_| StoreExplainabilityError::WriterEnded)?,
            None => Err(StoreExplainabilityError::ShutdownCommand),
        }
    }

    /// Explicitly transition Store run metadata to a terminal state.
    ///
    /// The run must already be finalized through [`ExplainabilitySink::finish_run`].
    ///
    /// # Errors
    ///
    /// Returns a recorder error when the run is not registered, not finalized,
    /// the writer is shutting down, or the Store rejected the completion.
    pub async fn complete_run(
        &self,
        completion: RunCompletion,
    ) -> Result<(), StoreExplainabilityError> {
        let run_id = completion.run_id().clone();
        let gate = self
            .sink
            .runs
            .get(&run_id)
            .map(|gate| Arc::clone(gate.value()))
            .ok_or(StoreExplainabilityError::RunNotRegistered)?;
        if gate.state() != RUN_FINISHED {
            return Err(StoreExplainabilityError::RunNotFinalized);
        }
        let admission = Arc::clone(&self.sink.admission)
            .acquire_owned()
            .await
            .map_err(|_| StoreExplainabilityError::ShutdownCommand)?;
        let response = if self.sink.writer_status.load(Ordering::Acquire) == WRITER_RUNNING {
            let (response, receiver) = oneshot::channel();
            match self
                .sink
                .sender
                .send(StoreWriterCommand::CompleteRun {
                    completion,
                    response,
                })
                .await
            {
                Ok(()) => Some(receiver),
                Err(_) => None,
            }
        } else {
            None
        };
        drop(admission);
        match response {
            Some(receiver) => receiver
                .await
                .map_err(|_| StoreExplainabilityError::WriterEnded)?,
            None => Err(StoreExplainabilityError::ShutdownCommand),
        }
    }

    /// Stop accepting commands, drain accepted work, and await the writer.
    ///
    /// Shutdown never creates a terminal event or completion; unfinished runs
    /// remain truthful unfinished prefixes.
    ///
    /// # Errors
    ///
    /// Returns the root writer error when the writer failed while processing
    /// accepted records, or a shutdown/join error when the writer ended
    /// abnormally.
    pub async fn shutdown(self) -> Result<(), StoreExplainabilityError> {
        let admission = Arc::clone(&self.sink.admission)
            .acquire_owned()
            .await
            .map_err(|_| StoreExplainabilityError::ShutdownCommand)?;
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
                    command_slot.send(StoreWriterCommand::Shutdown { response });
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
            Ok(Ok(())) if shutdown_send_failed => Err(StoreExplainabilityError::ShutdownCommand),
            Ok(Ok(())) => match acknowledgement {
                Some(Ok(Ok(()))) => Ok(()),
                Some(Ok(Err(error))) => Err(error),
                Some(Err(_)) | None => Err(StoreExplainabilityError::WriterEnded),
            },
            Ok(Err(error)) => Err(error),
            Err(source) => Err(StoreExplainabilityError::WriterJoin { source }),
        }
    }
}

/// Shared Sink accepting records into the bounded writer queue.
struct StoreExplainabilitySink {
    sender: mpsc::Sender<StoreWriterCommand>,
    writer_status: Arc<AtomicU8>,
    admission: Arc<Semaphore>,
    runs: Arc<DashMap<ExplainabilityRunId, Arc<RunGate>>>,
}

impl fmt::Debug for StoreExplainabilitySink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StoreExplainabilitySink { .. }")
    }
}

impl StoreExplainabilitySink {
    fn new(
        sender: mpsc::Sender<StoreWriterCommand>,
        writer_status: Arc<AtomicU8>,
        runs: Arc<DashMap<ExplainabilityRunId, Arc<RunGate>>>,
    ) -> Self {
        Self {
            sender,
            writer_status,
            admission: Arc::new(Semaphore::new(1)),
            runs,
        }
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
impl ExplainabilitySink for StoreExplainabilitySink {
    async fn emit(&self, record: Arc<ExplainabilityRecord>) -> Result<(), ExplainabilitySinkError> {
        let gate = match self.runs.get(&record.run_id) {
            Some(gate) => Arc::clone(gate.value()),
            None => return Err(ExplainabilitySinkError::RecordNotAccepted),
        };
        let run_admission = gate
            .admission
            .acquire()
            .await
            .map_err(|_| ExplainabilitySinkError::Closed)?;
        let admission = self
            .admission
            .acquire()
            .await
            .map_err(|_| ExplainabilitySinkError::Closed)?;
        if self.writer_status.load(Ordering::Acquire) != WRITER_RUNNING {
            return Err(self.writer_error());
        }
        if gate.state() != RUN_ACTIVE {
            return Err(ExplainabilitySinkError::RecordNotAccepted);
        }
        let result = self
            .sender
            .send(StoreWriterCommand::Record(record))
            .await
            .map_err(|_| self.writer_error());
        drop(run_admission);
        drop(admission);
        result
    }

    async fn finish_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<(), ExplainabilitySinkError> {
        let gate = match self.runs.get(run_id) {
            Some(gate) => Arc::clone(gate.value()),
            None => return Err(ExplainabilitySinkError::RunFinalizationFailed),
        };
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
                let Ok(command_slot) = self.sender.reserve().await else {
                    let error = self.writer_error();
                    gate.finish(&error);
                    return Err(error);
                };
                gate.complete(RUN_FINISHING);
                let (response, receiver) = oneshot::channel();
                command_slot.send(StoreWriterCommand::FinishRun {
                    run_id: run_id.clone(),
                    gate: Arc::clone(&gate),
                    response,
                });
                drop(admission);
                match receiver.await {
                    Ok(result) => result,
                    Err(_) => finish_state_result(gate.state()),
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
            RUN_FINISHED => Ok(()),
            _ => Err(ExplainabilitySinkError::RunFinalizationFailed),
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
            ExplainabilitySinkError::Closed => RUN_FAILED_CLOSED,
            _ => RUN_FAILED_WRITER,
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
enum StoreWriterCommand {
    CreateRun {
        run: ExplainabilityRun,
        response: oneshot::Sender<Result<(), StoreExplainabilityError>>,
    },
    Record(Arc<ExplainabilityRecord>),
    FinishRun {
        run_id: ExplainabilityRunId,
        gate: Arc<RunGate>,
        response: oneshot::Sender<Result<(), ExplainabilitySinkError>>,
    },
    CompleteRun {
        completion: RunCompletion,
        response: oneshot::Sender<Result<(), StoreExplainabilityError>>,
    },
    Shutdown {
        response: oneshot::Sender<Result<(), StoreExplainabilityError>>,
    },
}

#[derive(Debug, Default)]
struct StoreWriterState {
    sequences: HashMap<ExplainabilityRunId, u64>,
    finished_runs: HashSet<ExplainabilityRunId>,
}

#[derive(Debug)]
struct WriterStatusGuard {
    writer_status: Arc<AtomicU8>,
    runs: Arc<DashMap<ExplainabilityRunId, Arc<RunGate>>>,
    live_hub: Option<Arc<ExplainabilityLiveHub>>,
}

impl WriterStatusGuard {
    fn new(
        writer_status: Arc<AtomicU8>,
        runs: Arc<DashMap<ExplainabilityRunId, Arc<RunGate>>>,
        live_hub: Option<Arc<ExplainabilityLiveHub>>,
    ) -> Self {
        Self {
            writer_status,
            runs,
            live_hub,
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
        close_live_runs(self.live_hub.as_deref(), &self.runs);
    }
}

async fn run_writer(
    store: Arc<dyn ExplainabilityStore>,
    live_hub: Option<Arc<ExplainabilityLiveHub>>,
    mut receiver: mpsc::Receiver<StoreWriterCommand>,
    writer_status: Arc<AtomicU8>,
    runs: Arc<DashMap<ExplainabilityRunId, Arc<RunGate>>>,
) -> Result<(), StoreExplainabilityError> {
    let mut state = StoreWriterState::default();
    while let Some(command) = receiver.recv().await {
        match command {
            StoreWriterCommand::CreateRun { run, response } => {
                let result =
                    create_run_for_writer(store.as_ref(), live_hub.as_deref(), &runs, run).await;
                let _ = response.send(result);
            }
            StoreWriterCommand::Record(record) => {
                if let Err(error) = persist_record(
                    store.as_ref(),
                    live_hub.as_deref(),
                    &runs,
                    &mut state,
                    record,
                )
                .await
                {
                    return Err(mark_writer_failed(&writer_status, error));
                }
            }
            StoreWriterCommand::FinishRun {
                run_id,
                gate,
                response,
            } => {
                let result = finish_run_for_writer(live_hub.as_deref(), &mut state, &gate, &run_id);
                let _ = response.send(result);
            }
            StoreWriterCommand::CompleteRun {
                completion,
                response,
            } => {
                let result = complete_run_for_writer(store.as_ref(), &mut state, completion).await;
                let _ = response.send(result);
            }
            StoreWriterCommand::Shutdown { response } => {
                close_live_runs(live_hub.as_deref(), &runs);
                writer_status.store(WRITER_CLOSED, Ordering::Release);
                let _ = response.send(Ok(()));
                return Ok(());
            }
        }
    }
    writer_failure(&writer_status, StoreExplainabilityError::WriterEnded)
}

async fn create_run_for_writer(
    store: &dyn ExplainabilityStore,
    live_hub: Option<&ExplainabilityLiveHub>,
    runs: &DashMap<ExplainabilityRunId, Arc<RunGate>>,
    run: ExplainabilityRun,
) -> Result<(), StoreExplainabilityError> {
    let run_id = run.run_id.clone();
    if runs.contains_key(&run_id) {
        return Err(StoreExplainabilityError::RunAlreadyRegistered);
    }
    store
        .create_run(run)
        .await
        .map_err(|source| StoreExplainabilityError::Store {
            operation: StoreExplainabilityOperation::CreateRun,
            source,
        })?;
    if let Some(live_hub) = live_hub {
        live_hub.register_run(run_id.clone());
    }
    runs.insert(run_id, Arc::new(RunGate::new()));
    Ok(())
}

async fn persist_record(
    store: &dyn ExplainabilityStore,
    live_hub: Option<&ExplainabilityLiveHub>,
    runs: &DashMap<ExplainabilityRunId, Arc<RunGate>>,
    state: &mut StoreWriterState,
    record: Arc<ExplainabilityRecord>,
) -> Result<(), StoreExplainabilityError> {
    let run_id = record.run_id.clone();
    if !runs.contains_key(&run_id) {
        return Err(StoreExplainabilityError::RunNotRegistered);
    }
    // The sink rejects new emits as soon as finish begins, and the writer
    // processes commands in FIFO order, so records accepted before FinishRun
    // may still be processed after the shared gate shows FINISHING. The
    // writer-owned finished set is the correct late-record defense.
    if state.finished_runs.contains(&run_id) {
        return Err(StoreExplainabilityError::RecordAfterFinish);
    }
    let current = state.sequences.get(&run_id).copied().unwrap_or(0);
    let sequence = current
        .checked_add(1)
        .ok_or(StoreExplainabilityError::SequenceOverflow)?;
    let record = match Arc::try_unwrap(record) {
        Ok(record) => record,
        Err(record) => record.as_ref().clone(),
    };
    let envelope = ExplainabilityEnvelope::new(sequence, record)
        .map_err(|source| StoreExplainabilityError::Envelope { source })?;
    store
        .append_events(std::slice::from_ref(&envelope))
        .await
        .map_err(|source| StoreExplainabilityError::Store {
            operation: StoreExplainabilityOperation::AppendEvents,
            source,
        })?;
    // Persistence and writer sequence state are now committed.
    state.sequences.insert(run_id, sequence);
    if let Some(live_hub) = live_hub {
        live_hub.publish(Arc::new(envelope));
    }
    Ok(())
}

fn finish_run_for_writer(
    live_hub: Option<&ExplainabilityLiveHub>,
    state: &mut StoreWriterState,
    gate: &RunGate,
    run_id: &ExplainabilityRunId,
) -> Result<(), ExplainabilitySinkError> {
    if gate.state() != RUN_FINISHING {
        return Err(ExplainabilitySinkError::RunFinalizationFailed);
    }
    state.finished_runs.insert(run_id.clone());
    gate.complete(RUN_FINISHED);
    if let Some(live_hub) = live_hub {
        live_hub.close_run(run_id);
    }
    Ok(())
}

fn close_live_runs(
    live_hub: Option<&ExplainabilityLiveHub>,
    runs: &DashMap<ExplainabilityRunId, Arc<RunGate>>,
) {
    if let Some(live_hub) = live_hub {
        for run in runs {
            live_hub.close_run(run.key());
        }
    }
}

async fn complete_run_for_writer(
    store: &dyn ExplainabilityStore,
    state: &mut StoreWriterState,
    completion: RunCompletion,
) -> Result<(), StoreExplainabilityError> {
    let run_id = completion.run_id().clone();
    if !state.finished_runs.contains(&run_id) {
        return Err(StoreExplainabilityError::RunNotFinalized);
    }
    store
        .complete_run(completion)
        .await
        .map_err(|source| StoreExplainabilityError::Store {
            operation: StoreExplainabilityOperation::CompleteRun,
            source,
        })
}

fn mark_writer_failed(
    writer_status: &AtomicU8,
    error: StoreExplainabilityError,
) -> StoreExplainabilityError {
    writer_status.store(WRITER_FAILED, Ordering::Release);
    error
}

fn writer_failure(
    writer_status: &AtomicU8,
    error: StoreExplainabilityError,
) -> Result<(), StoreExplainabilityError> {
    Err(mark_writer_failed(writer_status, error))
}

#[cfg(test)]
mod tests {
    use std::{str::FromStr, sync::Arc};

    use chrono::Utc;

    use super::{
        DEFAULT_QUEUE_CAPACITY, RUN_ACTIVE, StoreExplainabilityError, StoreExplainabilityOptions,
        StoreExplainabilityRecorder, StoreWriterState, persist_record,
    };
    use crate::explainability::{
        EventQuery, ExplainabilityEvent, ExplainabilityQueryMethod, ExplainabilityRecord,
        ExplainabilityRun, ExplainabilityRunId, ExplainabilityRunKind, ExplainabilitySpanId,
        ExplainabilityStore, InMemoryExplainabilityStore, QueryStarted,
    };

    fn run_id(value: &str) -> ExplainabilityRunId {
        value.parse().expect("run id")
    }

    fn record(run_id: &ExplainabilityRunId) -> Arc<ExplainabilityRecord> {
        Arc::new(ExplainabilityRecord::new(
            run_id.clone(),
            Utc::now(),
            ExplainabilitySpanId::from_str("span-1").expect("span id"),
            None,
            ExplainabilityEvent::QueryStarted(QueryStarted::new(ExplainabilityQueryMethod::Local)),
        ))
    }

    #[test]
    fn test_should_default_queue_capacity_to_256() {
        let options = StoreExplainabilityOptions::new();
        assert_eq!(options.queue_capacity().get(), DEFAULT_QUEUE_CAPACITY);
        assert_eq!(
            StoreExplainabilityOptions::default().queue_capacity().get(),
            DEFAULT_QUEUE_CAPACITY
        );
        let custom = std::num::NonZeroUsize::new(7).expect("fixture");
        assert_eq!(
            options.with_queue_capacity(custom).queue_capacity().get(),
            7
        );
    }

    #[test]
    fn test_should_reject_store_recorder_creation_without_tokio_runtime() {
        let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
        let result = StoreExplainabilityRecorder::new(store, StoreExplainabilityOptions::new());
        assert!(matches!(
            result,
            Err(StoreExplainabilityError::RuntimeUnavailable)
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_should_redact_recorder_debug_output() {
        let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
        let recorder = StoreExplainabilityRecorder::new(store, StoreExplainabilityOptions::new())
            .expect("recorder");
        let debug = format!("{recorder:?}");
        let sink_debug = format!("{:?}", recorder.sink());
        assert_eq!(debug, "StoreExplainabilityRecorder { .. }");
        assert_eq!(sink_debug, "StoreExplainabilitySink { .. }");
        recorder.shutdown().await.expect("shutdown");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_should_fail_sequence_overflow_without_advancing_state() {
        let id = run_id("overflow");
        let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
        store
            .create_run(ExplainabilityRun::new(
                id.clone(),
                ExplainabilityRunKind::Query,
                Utc::now(),
            ))
            .await
            .expect("create run");
        let runs = dashmap::DashMap::new();
        let gate = super::RunGate::new();
        assert_eq!(gate.state(), RUN_ACTIVE);
        runs.insert(id.clone(), Arc::new(gate));
        let mut state = StoreWriterState::default();
        state.sequences.insert(id.clone(), u64::MAX);

        let result = persist_record(store.as_ref(), None, &runs, &mut state, record(&id)).await;
        assert!(matches!(
            result,
            Err(super::StoreExplainabilityError::SequenceOverflow)
        ));
        assert_eq!(state.sequences.get(&id).copied(), Some(u64::MAX));
        assert!(
            store
                .load_events(&id, &EventQuery::new())
                .await
                .expect("load events")
                .is_empty()
        );
    }
}
