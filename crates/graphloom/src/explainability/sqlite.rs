//! Durable [`ExplainabilityStore`] backed by a single `SQLite` connection.
//!
//! `SqliteExplainabilityStore` implements the frozen `ExplainabilityStore`
//! Version 1 contract with true file persistence. All rusqlite I/O runs on
//! `tokio::task::spawn_blocking` threads; no `SQLite` call executes on a Tokio
//! worker. A `std::sync::Mutex<Connection>` serializes access to the one
//! non-`Sync` connection owned by this instance, while `SQLite`
//! `BEGIN IMMEDIATE` transactions provide cross-instance and cross-process
//! write linearization. Business semantics never depend on the Rust mutex.
//!
//! The physical database has its own version ([`SQLITE_STORE_SCHEMA_VERSION`])
//! that is independent of [`EXPLAINABILITY_SCHEMA_VERSION`], which versions
//! the transport/event schema.

#[allow(
    clippy::disallowed_types,
    reason = "tokio::fs has no advisory-lock API; std file locking runs on a spawn_blocking thread"
)]
use std::fs::{File, OpenOptions};
use std::{
    fmt,
    path::Path,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params, types::Value};
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;
use tokio::sync::Mutex as AsyncMutex;

use super::{
    EXPLAINABILITY_SCHEMA_VERSION, EventQuery, ExplainabilityEnvelope, ExplainabilityEvent,
    ExplainabilityQueryMethod, ExplainabilityRecord, ExplainabilityRun, ExplainabilityRunId,
    ExplainabilityRunKind, ExplainabilityRunStatus, ExplainabilitySpanId, ExplainabilityStore,
    ExplainabilityStoreError, MAX_EVENT_QUERY_LIMIT, MAX_RUN_QUERY_LIMIT, RunCompletion, RunQuery,
    store::{is_terminal, validate_create_run, validate_limit},
};

/// Version of the physical `SQLite` database schema.
///
/// This is independent of [`EXPLAINABILITY_SCHEMA_VERSION`]: the former
/// versions the `explainability_store_meta` tables, the latter versions the
/// envelope/event transport schema. The two may evolve independently.
const SQLITE_STORE_SCHEMA_VERSION: u32 = 1;

/// How long a `SQLite` operation waits for a contended lock before failing.
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const SQLITE_BUSY_TIMEOUT_MS: i64 = 5_000;

const OPEN_OPERATION: &str = "open SQLite explainability store";
const CREATE_OPERATION: &str = "create explainability run";
const APPEND_OPERATION: &str = "append explainability events";
const COMPLETE_OPERATION: &str = "complete explainability run";
const GET_OPERATION: &str = "read explainability run";
const LIST_OPERATION: &str = "list explainability runs";
const LOAD_OPERATION: &str = "load explainability events";
const DELETE_OPERATION: &str = "delete explainability run";

const SCHEMA_SQL: &str = r"
CREATE TABLE explainability_store_meta (
    singleton INTEGER PRIMARY KEY
        CHECK (singleton = 1),

    schema_version INTEGER NOT NULL
);

CREATE TABLE explainability_runs (
    run_id TEXT PRIMARY KEY COLLATE BINARY,

    kind TEXT NOT NULL,
    status TEXT NOT NULL,

    query TEXT,
    query_method TEXT,

    started_at TEXT NOT NULL COLLATE BINARY,
    completed_at TEXT,

    compatibility_profile TEXT,

    event_count INTEGER NOT NULL
        CHECK (event_count >= 0)
);

CREATE TABLE explainability_events (
    run_id TEXT NOT NULL COLLATE BINARY,
    sequence INTEGER NOT NULL
        CHECK (sequence > 0),

    schema_version INTEGER NOT NULL,

    span_id TEXT NOT NULL,
    parent_span_id TEXT,

    timestamp TEXT NOT NULL COLLATE BINARY,
    event_type TEXT NOT NULL,

    payload_json TEXT NOT NULL,

    PRIMARY KEY (run_id, sequence),

    FOREIGN KEY (run_id)
        REFERENCES explainability_runs(run_id)
        ON DELETE CASCADE
);

CREATE INDEX explainability_runs_by_started_at
ON explainability_runs(
    started_at DESC,
    run_id DESC
);
";

const RUN_SELECT_SQL: &str = "
SELECT
    run_id,
    kind,
    status,
    query,
    query_method,
    started_at,
    completed_at,
    compatibility_profile,
    event_count
FROM explainability_runs
";

const EVENT_INSERT_SQL: &str = "
INSERT INTO explainability_events (
    run_id,
    sequence,
    schema_version,
    span_id,
    parent_span_id,
    timestamp,
    event_type,
    payload_json
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
";

const EVENT_SELECT_SQL: &str = "
SELECT
    run_id,
    sequence,
    schema_version,
    span_id,
    parent_span_id,
    timestamp,
    event_type,
    payload_json
FROM explainability_events
WHERE run_id = ?1
  AND sequence > ?2
ORDER BY sequence ASC
LIMIT ?3
";

/// Durable [`ExplainabilityStore`] implementation backed by one `SQLite` file.
///
/// The connection is owned by this instance and serialized through a
/// `std::sync::Mutex`; all actual database work runs inside
/// `tokio::task::spawn_blocking`. The instance never exposes the database
/// path, SQL, or stored payloads through `Debug`, errors, or tracing.
#[non_exhaustive]
pub struct SqliteExplainabilityStore {
    connection: Arc<Mutex<Connection>>,
    operation_gate: AsyncMutex<()>,
}

impl fmt::Debug for SqliteExplainabilityStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SqliteExplainabilityStore { .. }")
    }
}

impl SqliteExplainabilityStore {
    /// Open (creating when missing) the `SQLite` database at `path`.
    ///
    /// The file is created when it does not exist. Parent directories are
    /// never created automatically; the caller owns the directory lifecycle.
    ///
    /// # Errors
    ///
    /// Returns `ExplainabilityStoreError::Internal` when the file cannot be
    /// opened, a PRAGMA cannot be applied or verified, the schema is missing,
    /// partial, or from an unsupported version, or any initialization step
    /// fails.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, ExplainabilityStoreError> {
        let path = path.as_ref().to_path_buf();
        let opened = tokio::task::spawn_blocking(move || {
            // `PRAGMA journal_mode = WAL` does not invoke the SQLite busy
            // handler when another connection is inside a write transaction,
            // so concurrent first opens must be serialized by an advisory
            // sidecar lock instead of relying on the busy timeout.
            let _open_lock = OpenLock::acquire(&path)?;
            let mut connection =
                Connection::open(&path).map_err(SqliteStoreBackendError::Sqlite)?;
            configure_connection(&mut connection)?;
            initialize_schema(&mut connection)?;
            Ok::<_, SqliteStoreBackendError>(connection)
        })
        .await;
        let connection = match opened {
            Ok(Ok(connection)) => connection,
            Ok(Err(error)) => return Err(internal(OPEN_OPERATION, error)),
            Err(_) => return Err(internal(OPEN_OPERATION, SqliteStoreBackendError::Worker)),
        };
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            operation_gate: AsyncMutex::new(()),
        })
    }

    /// Run one `SQLite` operation on a blocking thread.
    ///
    /// The closure receives the single owned connection behind the instance
    /// mutex. Poisoned mutexes and cancelled/panicked worker tasks are
    /// converted to safe backend errors.
    async fn with_connection<T, F>(
        &self,
        operation: &'static str,
        function: F,
    ) -> Result<T, ExplainabilityStoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, SqliteStoreFailure> + Send + 'static,
    {
        let connection = Arc::clone(&self.connection);
        let result = tokio::task::spawn_blocking(move || {
            let mut guard = connection.lock().map_err(|_| {
                SqliteStoreFailure::Backend(SqliteStoreBackendError::ConnectionUnavailable)
            })?;
            function(&mut guard)
        })
        .await
        .map_err(|_| internal(operation, SqliteStoreBackendError::Worker))?;
        match result {
            Ok(value) => Ok(value),
            Err(SqliteStoreFailure::Backend(error)) => Err(internal(operation, error)),
            Err(SqliteStoreFailure::Store(error)) => Err(error),
        }
    }
}

/// Private backend failure of the `SQLite` store.
#[derive(Debug, Error)]
enum SqliteStoreBackendError {
    /// A `rusqlite` or `SQLite` operation failed.
    #[error("SQLite operation failed")]
    Sqlite(#[source] rusqlite::Error),
    /// The sidecar open lock could not be acquired.
    #[error("SQLite open lock failed")]
    OpenLock(#[source] std::io::Error),
    /// The blocking worker task was cancelled or panicked.
    #[error("SQLite worker task failed")]
    Worker,
    /// The connection mutex was poisoned.
    #[error("SQLite connection state is unavailable")]
    ConnectionUnavailable,
    /// A persisted value could not be interpreted.
    #[error("SQLite persisted value is invalid")]
    InvalidPersistedValue,
    /// A stored or requested integer exceeded `SQLite`'s signed range.
    #[error("SQLite integer value for {field} is outside the supported range")]
    IntegerOutOfRange {
        /// Low-cardinality field name, never user content.
        field: &'static str,
    },
    /// The physical database schema version is not supported.
    #[error("unsupported SQLite explainability schema version")]
    UnsupportedSchemaVersion,
    /// The physical schema is missing required objects.
    #[error("SQLite explainability schema is incomplete")]
    IncompleteSchema,
    /// A required PRAGMA did not verify to its configured value.
    #[error("SQLite pragma verification failed")]
    PragmaVerificationFailed,
    /// Explainability data could not be serialized or deserialized.
    #[error("failed to encode explainability data")]
    Serialization(#[source] serde_json::Error),
}

/// Result of one closure inside [`SqliteExplainabilityStore::with_connection`].
#[derive(Debug)]
enum SqliteStoreFailure {
    /// Backend-level failure mapped to `ExplainabilityStoreError::Internal`.
    Backend(SqliteStoreBackendError),
    /// Business failure returned verbatim from the store contract.
    Store(ExplainabilityStoreError),
}

impl From<SqliteStoreBackendError> for SqliteStoreFailure {
    fn from(value: SqliteStoreBackendError) -> Self {
        Self::Backend(value)
    }
}

impl From<rusqlite::Error> for SqliteStoreBackendError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}

/// Advisory sidecar lock serializing database initialization.
///
/// The lock file is `<database>.lock` and is held only while the connection
/// is configured and the schema is initialized. The lock is released when the
/// file handle drops, including after a process crash.
#[allow(
    clippy::disallowed_types,
    reason = "tokio::fs::File has no advisory-lock API; OpenLock runs on a spawn_blocking thread, \
              so the blocking std::fs lock is intentional (see \
              crates/graphloom/src/explainability/sqlite.rs:317)"
)]
struct OpenLock {
    _file: File,
}

#[allow(
    clippy::disallowed_types,
    reason = "std::fs::OpenOptions is required to create the sidecar lock file without \
              truncation; the call runs on a spawn_blocking thread, not an async worker"
)]
impl OpenLock {
    fn acquire(path: &Path) -> Result<Self, SqliteStoreBackendError> {
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(std::path::PathBuf::from(lock_path))
            .map_err(SqliteStoreBackendError::OpenLock)?;
        file.lock().map_err(SqliteStoreBackendError::OpenLock)?;
        Ok(Self { _file: file })
    }
}

fn internal(operation: &'static str, source: SqliteStoreBackendError) -> ExplainabilityStoreError {
    ExplainabilityStoreError::Internal {
        operation,
        source: Box::new(source),
    }
}

fn configure_connection(connection: &mut Connection) -> Result<(), SqliteStoreBackendError> {
    connection
        .busy_timeout(SQLITE_BUSY_TIMEOUT)
        .map_err(SqliteStoreBackendError::Sqlite)?;
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;\nPRAGMA journal_mode = WAL;\nPRAGMA synchronous = FULL;\n",
        )
        .map_err(SqliteStoreBackendError::Sqlite)?;

    if query_pragma_integer(connection, "PRAGMA foreign_keys")? != 1 {
        return Err(SqliteStoreBackendError::PragmaVerificationFailed);
    }
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(SqliteStoreBackendError::Sqlite)?;
    if journal_mode != "wal" {
        return Err(SqliteStoreBackendError::PragmaVerificationFailed);
    }
    if query_pragma_integer(connection, "PRAGMA synchronous")? != 2 {
        return Err(SqliteStoreBackendError::PragmaVerificationFailed);
    }
    if query_pragma_integer(connection, "PRAGMA busy_timeout")? != SQLITE_BUSY_TIMEOUT_MS {
        return Err(SqliteStoreBackendError::PragmaVerificationFailed);
    }
    Ok(())
}

fn query_pragma_integer(
    connection: &Connection,
    pragma: &str,
) -> Result<i64, SqliteStoreBackendError> {
    connection
        .query_row(pragma, [], |row| row.get(0))
        .map_err(SqliteStoreBackendError::Sqlite)
}

fn initialize_schema(connection: &mut Connection) -> Result<(), SqliteStoreBackendError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(SqliteStoreBackendError::Sqlite)?;
    let meta_exists = table_exists(&transaction, "explainability_store_meta")?;
    let runs_exists = table_exists(&transaction, "explainability_runs")?;
    let events_exists = table_exists(&transaction, "explainability_events")?;

    if meta_exists {
        let version = schema_version(&transaction)?;
        match version {
            SQLITE_STORE_SCHEMA_VERSION => {
                if !runs_exists || !events_exists {
                    return Err(SqliteStoreBackendError::IncompleteSchema);
                }
            }
            _ => return Err(SqliteStoreBackendError::UnsupportedSchemaVersion),
        }
    } else if runs_exists || events_exists {
        return Err(SqliteStoreBackendError::IncompleteSchema);
    } else {
        create_schema(&transaction)?;
    }

    transaction
        .commit()
        .map_err(SqliteStoreBackendError::Sqlite)
}

fn table_exists(
    transaction: &rusqlite::Transaction<'_>,
    name: &str,
) -> Result<bool, SqliteStoreBackendError> {
    let count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![name],
            |row| row.get(0),
        )
        .map_err(SqliteStoreBackendError::Sqlite)?;
    Ok(count == 1)
}

fn schema_version(transaction: &rusqlite::Transaction<'_>) -> Result<u32, SqliteStoreBackendError> {
    let version = transaction
        .query_row(
            "SELECT schema_version FROM explainability_store_meta WHERE singleton = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(SqliteStoreBackendError::Sqlite)?
        .ok_or(SqliteStoreBackendError::IncompleteSchema)?;
    u32::try_from(version).map_err(|_| SqliteStoreBackendError::InvalidPersistedValue)
}

fn create_schema(transaction: &rusqlite::Transaction<'_>) -> Result<(), SqliteStoreBackendError> {
    transaction
        .execute_batch(SCHEMA_SQL)
        .map_err(SqliteStoreBackendError::Sqlite)?;
    transaction
        .execute(
            "INSERT INTO explainability_store_meta (singleton, schema_version) VALUES (1, ?1)",
            params![SQLITE_STORE_SCHEMA_VERSION],
        )
        .map_err(SqliteStoreBackendError::Sqlite)?;
    Ok(())
}

#[async_trait::async_trait]
impl ExplainabilityStore for SqliteExplainabilityStore {
    async fn create_run(&self, run: ExplainabilityRun) -> Result<(), ExplainabilityStoreError> {
        validate_create_run(&run)?;
        let _gate = self.operation_gate.lock().await;
        self.with_connection(CREATE_OPERATION, move |connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(SqliteStoreBackendError::Sqlite)?;
            insert_run(&transaction, &run)?;
            transaction
                .commit()
                .map_err(SqliteStoreBackendError::Sqlite)?;
            Ok(())
        })
        .await
    }

    async fn append_events(
        &self,
        events: &[ExplainabilityEnvelope],
    ) -> Result<(), ExplainabilityStoreError> {
        let Some(first) = events.first() else {
            return Ok(());
        };
        let first_run = first.record.run_id.clone();
        if let Some(mixed) = events
            .iter()
            .find(|envelope| envelope.record.run_id != first_run)
        {
            return Err(ExplainabilityStoreError::MixedRunBatch {
                first: first_run,
                second: mixed.record.run_id.clone(),
            });
        }
        let batch = events.to_vec();
        let _gate = self.operation_gate.lock().await;
        self.with_connection(APPEND_OPERATION, move |connection| {
            let prepared = batch
                .iter()
                .map(prepare_event)
                .collect::<Result<Vec<_>, _>>()?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(SqliteStoreBackendError::Sqlite)?;
            append_prepared_events(&transaction, &first_run, &prepared)?;
            transaction
                .commit()
                .map_err(SqliteStoreBackendError::Sqlite)?;
            Ok(())
        })
        .await
    }

    async fn complete_run(
        &self,
        completion: RunCompletion,
    ) -> Result<(), ExplainabilityStoreError> {
        if !is_terminal(completion.status()) {
            return Err(ExplainabilityStoreError::InvalidCompletionStatus {
                status: completion.status(),
            });
        }
        let run_id = completion.run_id().clone();
        let status = completion.status();
        let completed_at = completion.completed_at();
        let _gate = self.operation_gate.lock().await;
        self.with_connection(COMPLETE_OPERATION, move |connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(SqliteStoreBackendError::Sqlite)?;
            let run_row = transaction
                .query_row(
                    "SELECT status, started_at, completed_at
                     FROM explainability_runs
                     WHERE run_id = ?1",
                    params![run_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(SqliteStoreBackendError::Sqlite)?;
            let Some((status_text, started_at_text, completed_at_text)) = run_row else {
                return Err(SqliteStoreFailure::Store(
                    ExplainabilityStoreError::RunNotFound {
                        run_id: run_id.clone(),
                    },
                ));
            };
            let stored_status = enum_from_sql_text::<ExplainabilityRunStatus>(&status_text)?;
            let started_at = datetime_from_sql_text(&started_at_text)?;
            let stored_completed_at = completed_at_text
                .as_deref()
                .map(datetime_from_sql_text)
                .transpose()?;
            if completed_at < started_at {
                return Err(SqliteStoreFailure::Store(
                    ExplainabilityStoreError::InvalidCompletionTime {
                        run_id: run_id.clone(),
                        completed_at,
                        started_at,
                    },
                ));
            }
            if stored_completed_at.is_some() || is_terminal(stored_status) {
                if stored_status == status && stored_completed_at == Some(completed_at) {
                    return Ok(());
                }
                return Err(SqliteStoreFailure::Store(
                    ExplainabilityStoreError::CompletionConflict {
                        run_id: run_id.clone(),
                    },
                ));
            }
            let updated = transaction
                .execute(
                    "UPDATE explainability_runs
                     SET status = ?1, completed_at = ?2
                     WHERE run_id = ?3",
                    params![
                        enum_to_sql_text(status)?,
                        datetime_to_sql_text(&completed_at),
                        run_id.as_str(),
                    ],
                )
                .map_err(SqliteStoreBackendError::Sqlite)?;
            if updated != 1 {
                return Err(SqliteStoreBackendError::InvalidPersistedValue.into());
            }
            transaction
                .commit()
                .map_err(SqliteStoreBackendError::Sqlite)?;
            Ok(())
        })
        .await
    }

    async fn get_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<Option<ExplainabilityRun>, ExplainabilityStoreError> {
        let run_id = run_id.clone();
        let _gate = self.operation_gate.lock().await;
        self.with_connection(GET_OPERATION, move |connection| {
            let mut statement = connection
                .prepare(&format!("{RUN_SELECT_SQL} WHERE run_id = ?1"))
                .map_err(SqliteStoreBackendError::Sqlite)?;
            let mut rows = statement
                .query(params![run_id.as_str()])
                .map_err(SqliteStoreBackendError::Sqlite)?;
            let run = match rows.next().map_err(SqliteStoreBackendError::Sqlite)? {
                Some(row) => Some(run_from_row(row)?),
                None => None,
            };
            Ok(run)
        })
        .await
    }

    async fn list_runs(
        &self,
        query: &RunQuery,
    ) -> Result<Vec<ExplainabilityRun>, ExplainabilityStoreError> {
        validate_limit(query.limit(), 1, MAX_RUN_QUERY_LIMIT, "run history")?;
        let query = query.clone();
        let _gate = self.operation_gate.lock().await;
        self.with_connection(LIST_OPERATION, move |connection| {
            let mut sql = RUN_SELECT_SQL.to_owned();
            let mut conditions = Vec::new();
            let mut values: Vec<Value> = Vec::new();
            if let Some(kind) = query.kind_filter() {
                conditions.push("kind = ?");
                values.push(Value::Text(enum_to_sql_text(kind)?));
            }
            if let Some(status) = query.status_filter() {
                conditions.push("status = ?");
                values.push(Value::Text(enum_to_sql_text(status)?));
            }
            if let Some(method) = query.query_method_filter() {
                conditions.push("query_method = ?");
                values.push(Value::Text(enum_to_sql_text(method)?));
            }
            if let Some(cursor) = query.before_cursor() {
                conditions.push("(started_at < ? OR (started_at = ? AND run_id < ?))");
                let started_at = datetime_to_sql_text(&cursor.started_at());
                values.push(Value::Text(started_at.clone()));
                values.push(Value::Text(started_at));
                values.push(Value::Text(cursor.run_id().as_str().to_owned()));
            }
            if !conditions.is_empty() {
                sql.push_str(" WHERE ");
                sql.push_str(&conditions.join(" AND "));
            }
            sql.push_str(" ORDER BY started_at DESC, run_id DESC LIMIT ?");
            values.push(Value::Integer(i64::from(query.limit())));

            let mut statement = connection
                .prepare(&sql)
                .map_err(SqliteStoreBackendError::Sqlite)?;
            let mut rows = statement
                .query(rusqlite::params_from_iter(values))
                .map_err(SqliteStoreBackendError::Sqlite)?;
            let mut runs = Vec::new();
            while let Some(row) = rows.next().map_err(SqliteStoreBackendError::Sqlite)? {
                runs.push(run_from_row(row)?);
            }
            Ok(runs)
        })
        .await
    }

    async fn load_events(
        &self,
        run_id: &ExplainabilityRunId,
        query: &EventQuery,
    ) -> Result<Vec<ExplainabilityEnvelope>, ExplainabilityStoreError> {
        validate_limit(query.limit(), 1, MAX_EVENT_QUERY_LIMIT, "event replay")?;
        let run_id = run_id.clone();
        let after = query.after_sequence_bound().unwrap_or(0);
        let limit = query.limit();
        let _gate = self.operation_gate.lock().await;
        self.with_connection(LOAD_OPERATION, move |connection| {
            let exists = connection
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM explainability_runs WHERE run_id = ?1
                     )",
                    params![run_id.as_str()],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(SqliteStoreBackendError::Sqlite)?;
            if !exists {
                return Err(SqliteStoreFailure::Store(
                    ExplainabilityStoreError::RunNotFound {
                        run_id: run_id.clone(),
                    },
                ));
            }
            // Stored sequences never exceed SQLite's signed range, so any
            // exclusive bound that does not fit into i64 can match nothing.
            let Ok(after_sqlite) = i64::try_from(after) else {
                return Ok(Vec::new());
            };
            let mut statement = connection
                .prepare(EVENT_SELECT_SQL)
                .map_err(SqliteStoreBackendError::Sqlite)?;
            let mut rows = statement
                .query(params![run_id.as_str(), after_sqlite, i64::from(limit)])
                .map_err(SqliteStoreBackendError::Sqlite)?;
            let mut envelopes = Vec::new();
            while let Some(row) = rows.next().map_err(SqliteStoreBackendError::Sqlite)? {
                envelopes.push(envelope_from_row(row)?);
            }
            Ok(envelopes)
        })
        .await
    }

    async fn delete_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<(), ExplainabilityStoreError> {
        let run_id = run_id.clone();
        let _gate = self.operation_gate.lock().await;
        self.with_connection(DELETE_OPERATION, move |connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(SqliteStoreBackendError::Sqlite)?;
            transaction
                .execute(
                    "DELETE FROM explainability_runs WHERE run_id = ?1",
                    params![run_id.as_str()],
                )
                .map_err(SqliteStoreBackendError::Sqlite)?;
            transaction
                .commit()
                .map_err(SqliteStoreBackendError::Sqlite)?;
            Ok(())
        })
        .await
    }
}

fn run_exists(
    transaction: &rusqlite::Transaction<'_>,
    run_id: &str,
) -> Result<bool, SqliteStoreBackendError> {
    let exists = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM explainability_runs WHERE run_id = ?1)",
            params![run_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(SqliteStoreBackendError::Sqlite)?;
    Ok(exists)
}

fn insert_run(
    transaction: &rusqlite::Transaction<'_>,
    run: &ExplainabilityRun,
) -> Result<(), SqliteStoreFailure> {
    let run_id = run.run_id.clone();
    if run_exists(transaction, run_id.as_str())? {
        return Err(SqliteStoreFailure::Store(
            ExplainabilityStoreError::RunAlreadyExists { run_id },
        ));
    }
    match transaction.execute(
        "INSERT INTO explainability_runs (
            run_id,
            kind,
            status,
            query,
            query_method,
            started_at,
            completed_at,
            compatibility_profile,
            event_count
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, 0)",
        params![
            run_id.as_str(),
            enum_to_sql_text(run.kind)?,
            enum_to_sql_text(run.status)?,
            run.query.as_deref(),
            run.query_method.map(enum_to_sql_text).transpose()?,
            datetime_to_sql_text(&run.started_at),
            run.compatibility_profile.as_deref(),
        ],
    ) {
        Ok(_) => Ok(()),
        Err(error) => {
            if run_exists(transaction, run_id.as_str())? {
                return Err(SqliteStoreFailure::Store(
                    ExplainabilityStoreError::RunAlreadyExists { run_id },
                ));
            }
            Err(SqliteStoreBackendError::Sqlite(error).into())
        }
    }
}

fn append_prepared_events(
    transaction: &rusqlite::Transaction<'_>,
    run_id: &ExplainabilityRunId,
    prepared: &[PreparedEvent],
) -> Result<(), SqliteStoreFailure> {
    let run_row = transaction
        .query_row(
            "SELECT status, event_count FROM explainability_runs WHERE run_id = ?1",
            params![run_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(SqliteStoreBackendError::Sqlite)?;
    let Some((status_text, event_count_sqlite)) = run_row else {
        return Err(SqliteStoreFailure::Store(
            ExplainabilityStoreError::RunNotFound {
                run_id: run_id.clone(),
            },
        ));
    };
    let status = enum_from_sql_text::<ExplainabilityRunStatus>(&status_text)?;
    if is_terminal(status) {
        return Err(SqliteStoreFailure::Store(
            ExplainabilityStoreError::RunAlreadyTerminal {
                run_id: run_id.clone(),
            },
        ));
    }
    let event_count = sqlite_integer_to_u64(event_count_sqlite, "event_count")?;

    let mut next_sequence = event_count;
    for item in prepared {
        let expected = next_sequence.checked_add(1).ok_or_else(|| {
            SqliteStoreFailure::Store(ExplainabilityStoreError::SequenceOverflow {
                run_id: run_id.clone(),
            })
        })?;
        if item.sequence != expected {
            return Err(SqliteStoreFailure::Store(
                ExplainabilityStoreError::SequenceConflict {
                    run_id: run_id.clone(),
                    expected,
                    actual: item.sequence,
                },
            ));
        }
        next_sequence = expected;
    }
    let final_count = next_sequence;
    let final_count_sqlite = u64_to_sqlite_integer(final_count, "event_count")?;

    {
        let mut insert = transaction
            .prepare(EVENT_INSERT_SQL)
            .map_err(SqliteStoreBackendError::Sqlite)?;
        for item in prepared {
            insert
                .execute(params![
                    item.run_id.as_str(),
                    item.sequence_sqlite,
                    item.schema_version_sqlite,
                    item.span_id.as_str(),
                    item.parent_span_id.as_deref(),
                    item.timestamp.as_str(),
                    item.event_type.as_str(),
                    item.payload_json.as_str(),
                ])
                .map_err(SqliteStoreBackendError::Sqlite)?;
        }
    }

    let updated = transaction
        .execute(
            "UPDATE explainability_runs
             SET event_count = ?1
             WHERE run_id = ?2
               AND event_count = ?3",
            params![final_count_sqlite, run_id.as_str(), event_count_sqlite],
        )
        .map_err(SqliteStoreBackendError::Sqlite)?;
    if updated != 1 {
        return Err(SqliteStoreBackendError::InvalidPersistedValue.into());
    }
    Ok(())
}

fn run_from_row(row: &rusqlite::Row<'_>) -> Result<ExplainabilityRun, SqliteStoreBackendError> {
    let run_id_text: String = row.get(0)?;
    let kind_text: String = row.get(1)?;
    let status_text: String = row.get(2)?;
    let query: Option<String> = row.get(3)?;
    let query_method_text: Option<String> = row.get(4)?;
    let started_at_text: String = row.get(5)?;
    let completed_at_text: Option<String> = row.get(6)?;
    let compatibility_profile: Option<String> = row.get(7)?;
    let event_count_sqlite: i64 = row.get(8)?;

    let run_id = ExplainabilityRunId::from_str(&run_id_text)
        .map_err(|_| SqliteStoreBackendError::InvalidPersistedValue)?;
    let kind = enum_from_sql_text::<ExplainabilityRunKind>(&kind_text)?;
    let status = enum_from_sql_text::<ExplainabilityRunStatus>(&status_text)?;
    let query_method = query_method_text
        .as_deref()
        .map(enum_from_sql_text::<ExplainabilityQueryMethod>)
        .transpose()?;
    let started_at = datetime_from_sql_text(&started_at_text)?;
    let completed_at = completed_at_text
        .as_deref()
        .map(datetime_from_sql_text)
        .transpose()?;
    let event_count = sqlite_integer_to_u64(event_count_sqlite, "event_count")?;

    if kind != ExplainabilityRunKind::Query && query_method.is_some() {
        return Err(SqliteStoreBackendError::InvalidPersistedValue);
    }
    if is_terminal(status) != completed_at.is_some() {
        return Err(SqliteStoreBackendError::InvalidPersistedValue);
    }
    if let Some(completed_at) = completed_at
        && completed_at < started_at
    {
        return Err(SqliteStoreBackendError::InvalidPersistedValue);
    }

    Ok(ExplainabilityRun {
        run_id,
        kind,
        status,
        query,
        query_method,
        started_at,
        completed_at,
        compatibility_profile,
        event_count,
    })
}

struct PreparedEvent {
    run_id: String,
    sequence: u64,
    sequence_sqlite: i64,
    schema_version_sqlite: i64,
    span_id: String,
    parent_span_id: Option<String>,
    timestamp: String,
    event_type: String,
    payload_json: String,
}

fn prepare_event(envelope: &ExplainabilityEnvelope) -> Result<PreparedEvent, SqliteStoreFailure> {
    if envelope.schema_version() != EXPLAINABILITY_SCHEMA_VERSION {
        return Err(SqliteStoreBackendError::InvalidPersistedValue.into());
    }
    let payload = serde_json::to_value(&envelope.record.event)
        .map_err(SqliteStoreBackendError::Serialization)?;
    let event_type = payload
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or(SqliteStoreBackendError::InvalidPersistedValue)?
        .to_owned();
    let payload_json =
        serde_json::to_string(&payload).map_err(SqliteStoreBackendError::Serialization)?;
    let sequence = envelope.sequence();
    let sequence_sqlite = u64_to_sqlite_integer(sequence, "sequence")?;
    let schema_version_sqlite = i64::from(envelope.schema_version());
    let run_id = envelope.record.run_id.as_str().to_owned();
    let span_id = envelope.record.span_id.as_str().to_owned();
    let parent_span_id = envelope
        .record
        .parent_span_id
        .as_ref()
        .map(ExplainabilitySpanId::as_str)
        .map(str::to_owned);
    let timestamp = datetime_to_sql_text(&envelope.record.timestamp);
    Ok(PreparedEvent {
        run_id,
        sequence,
        sequence_sqlite,
        schema_version_sqlite,
        span_id,
        parent_span_id,
        timestamp,
        event_type,
        payload_json,
    })
}

fn envelope_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<ExplainabilityEnvelope, SqliteStoreBackendError> {
    let run_id_text: String = row.get(0)?;
    let sequence_sqlite: i64 = row.get(1)?;
    let schema_version_sqlite: i64 = row.get(2)?;
    let span_id_text: String = row.get(3)?;
    let parent_span_id_text: Option<String> = row.get(4)?;
    let timestamp_text: String = row.get(5)?;
    let event_type: String = row.get(6)?;
    let payload_json: String = row.get(7)?;

    let run_id = ExplainabilityRunId::from_str(&run_id_text)
        .map_err(|_| SqliteStoreBackendError::InvalidPersistedValue)?;
    let span_id = ExplainabilitySpanId::from_str(&span_id_text)
        .map_err(|_| SqliteStoreBackendError::InvalidPersistedValue)?;
    let parent_span_id = parent_span_id_text
        .map(|text| {
            ExplainabilitySpanId::from_str(&text)
                .map_err(|_| SqliteStoreBackendError::InvalidPersistedValue)
        })
        .transpose()?;
    let timestamp = datetime_from_sql_text(&timestamp_text)?;
    let sequence = sqlite_integer_to_u64(sequence_sqlite, "sequence")?;
    let schema_version = u32::try_from(schema_version_sqlite)
        .map_err(|_| SqliteStoreBackendError::InvalidPersistedValue)?;
    if schema_version != EXPLAINABILITY_SCHEMA_VERSION {
        return Err(SqliteStoreBackendError::InvalidPersistedValue);
    }
    let payload: serde_json::Value =
        serde_json::from_str(&payload_json).map_err(SqliteStoreBackendError::Serialization)?;
    let payload_type = payload
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or(SqliteStoreBackendError::InvalidPersistedValue)?;
    if payload_type != event_type.as_str() {
        return Err(SqliteStoreBackendError::InvalidPersistedValue);
    }
    let event: ExplainabilityEvent =
        serde_json::from_value(payload).map_err(SqliteStoreBackendError::Serialization)?;
    let record = ExplainabilityRecord::new(run_id, timestamp, span_id, parent_span_id, event);
    ExplainabilityEnvelope::new(sequence, record)
        .map_err(|_| SqliteStoreBackendError::InvalidPersistedValue)
}

fn enum_to_sql_text<T>(value: T) -> Result<String, SqliteStoreBackendError>
where
    T: Serialize,
{
    let json = serde_json::to_value(value).map_err(SqliteStoreBackendError::Serialization)?;
    json.as_str()
        .map(str::to_owned)
        .ok_or(SqliteStoreBackendError::InvalidPersistedValue)
}

fn enum_from_sql_text<T>(value: &str) -> Result<T, SqliteStoreBackendError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|_| SqliteStoreBackendError::InvalidPersistedValue)
}

fn datetime_to_sql_text(value: &DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn datetime_from_sql_text(value: &str) -> Result<DateTime<Utc>, SqliteStoreBackendError> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|_| SqliteStoreBackendError::InvalidPersistedValue)
}

fn u64_to_sqlite_integer(value: u64, field: &'static str) -> Result<i64, SqliteStoreBackendError> {
    i64::try_from(value).map_err(|_| SqliteStoreBackendError::IntegerOutOfRange { field })
}

fn sqlite_integer_to_u64(value: i64, field: &'static str) -> Result<u64, SqliteStoreBackendError> {
    u64::try_from(value).map_err(|_| SqliteStoreBackendError::IntegerOutOfRange { field })
}

#[cfg(test)]
mod tests {
    use std::{
        str::FromStr,
        sync::{Arc, Mutex},
    };

    use chrono::{TimeZone, Utc};
    use rusqlite::Connection;
    use tempfile::TempDir;
    use tokio::sync::Mutex as AsyncMutex;

    use super::{
        EXPLAINABILITY_SCHEMA_VERSION, SQLITE_BUSY_TIMEOUT_MS, SQLITE_STORE_SCHEMA_VERSION,
        SqliteExplainabilityStore, datetime_from_sql_text, datetime_to_sql_text,
        enum_from_sql_text, enum_to_sql_text, envelope_from_row, prepare_event, run_from_row,
        sqlite_integer_to_u64, u64_to_sqlite_integer,
    };
    use crate::explainability::{
        ExplainabilityContentMode, ExplainabilityEnvelope, ExplainabilityEvent,
        ExplainabilityQueryMethod, ExplainabilityRecord, ExplainabilityRunId,
        ExplainabilityRunKind, ExplainabilityRunStatus, ExplainabilitySpanId, QueryStarted,
        RunStarted,
    };

    fn run_id(value: &str) -> ExplainabilityRunId {
        value.parse().expect("run id")
    }

    fn timestamp() -> chrono::DateTime<chrono::Utc> {
        Utc.with_ymd_and_hms(2026, 8, 7, 12, 34, 56)
            .single()
            .expect("timestamp")
            + chrono::Duration::nanoseconds(123_456_789)
    }

    fn envelope(
        id: ExplainabilityRunId,
        sequence: u64,
        event: ExplainabilityEvent,
    ) -> ExplainabilityEnvelope {
        ExplainabilityEnvelope::new(
            sequence,
            ExplainabilityRecord::new(
                id,
                timestamp(),
                ExplainabilitySpanId::from_str("span-1").expect("span id"),
                None,
                event,
            ),
        )
        .expect("envelope")
    }

    #[test]
    fn test_should_convert_u64_to_sqlite_integer_at_boundaries() {
        let max = u64::try_from(i64::MAX).expect("fixture");
        for value in [0_u64, 1, max] {
            assert_eq!(
                u64_to_sqlite_integer(value, "sequence").expect("in range"),
                i64::try_from(value).expect("fixture")
            );
        }
        assert!(u64_to_sqlite_integer(max + 1, "sequence").is_err());
        assert!(u64_to_sqlite_integer(u64::MAX, "sequence").is_err());
    }

    #[test]
    fn test_should_reject_negative_sqlite_integer_on_conversion() {
        assert_eq!(sqlite_integer_to_u64(0, "event_count").expect("zero"), 0);
        assert_eq!(sqlite_integer_to_u64(1, "event_count").expect("one"), 1);
        assert_eq!(
            sqlite_integer_to_u64(i64::MAX, "event_count").expect("max"),
            u64::try_from(i64::MAX).expect("fixture")
        );
        assert!(sqlite_integer_to_u64(-1, "event_count").is_err());
    }

    #[test]
    fn test_should_convert_enums_without_json_quotes() {
        assert_eq!(
            enum_to_sql_text(ExplainabilityRunKind::Query).expect("kind"),
            "query"
        );
        assert_eq!(
            enum_to_sql_text(ExplainabilityRunStatus::Running).expect("status"),
            "running"
        );
        assert_eq!(
            enum_to_sql_text(ExplainabilityQueryMethod::Local).expect("method"),
            "local"
        );
        assert_eq!(
            enum_from_sql_text::<ExplainabilityRunKind>("query").expect("kind"),
            ExplainabilityRunKind::Query
        );
        assert!(
            enum_from_sql_text::<ExplainabilityRunKind>("\"query\"").is_err(),
            "stored text must never contain JSON quotes"
        );
        assert!(enum_from_sql_text::<ExplainabilityRunStatus>("future").is_err());
    }

    #[test]
    fn test_should_format_and_parse_utc_timestamp_with_nanos_and_z() {
        let value = timestamp();
        let text = datetime_to_sql_text(&value);
        assert_eq!(text, "2026-08-07T12:34:56.123456789Z");
        assert_eq!(datetime_from_sql_text(&text).expect("parse"), value);
        assert!(datetime_from_sql_text("2026-08-07 12:34:56").is_err());
    }

    #[test]
    fn test_should_extract_event_type_from_serialized_payload() {
        let id = run_id("prep-run");
        let prepared = prepare_event(&envelope(
            id,
            1,
            ExplainabilityEvent::QueryStarted(QueryStarted::new(ExplainabilityQueryMethod::Local)),
        ))
        .expect("prepare");
        assert_eq!(prepared.event_type, "query_started");
        assert_eq!(prepared.sequence_sqlite, 1);
        assert_eq!(
            prepared.schema_version_sqlite,
            i64::from(EXPLAINABILITY_SCHEMA_VERSION)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_should_verify_configured_pragmas_on_real_connection() {
        let directory = TempDir::new().expect("tempdir");
        let store = SqliteExplainabilityStore::open(directory.path().join("pragma.sqlite"))
            .await
            .expect("open");
        let connection = store.connection.lock().expect("connection");
        let foreign_keys: i64 = connection
            .query_row("PRAGMA foreign_keys;", [], |row| row.get(0))
            .expect("foreign keys");
        let journal_mode: String = connection
            .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
            .expect("journal mode");
        let synchronous: i64 = connection
            .query_row("PRAGMA synchronous;", [], |row| row.get(0))
            .expect("synchronous");
        let busy_timeout: i64 = connection
            .query_row("PRAGMA busy_timeout;", [], |row| row.get(0))
            .expect("busy timeout");
        assert_eq!(foreign_keys, 1);
        assert_eq!(journal_mode, "wal");
        assert_eq!(synchronous, 2);
        assert_eq!(busy_timeout, SQLITE_BUSY_TIMEOUT_MS);
        assert_eq!(SQLITE_STORE_SCHEMA_VERSION, 1);
    }

    #[test]
    fn test_should_redact_debug_output_and_path() {
        let path = "SQLITE_DB_PATH_SECRET_SENTINEL/explainability.sqlite";
        let debug = format!(
            "{:?}",
            SqliteExplainabilityStore {
                connection: Arc::new(Mutex::new(Connection::open_in_memory().expect("memory"))),
                operation_gate: AsyncMutex::new(()),
            }
        );
        assert!(!debug.contains(path));
        assert_eq!(debug, "SqliteExplainabilityStore { .. }");
    }

    #[test]
    fn test_should_keep_event_type_matching_serialized_payload() {
        let id = run_id("type-run");
        let prepared = prepare_event(&envelope(
            id,
            1,
            ExplainabilityEvent::RunStarted(RunStarted::new(
                ExplainabilityRunKind::Query,
                ExplainabilityContentMode::Metadata,
            )),
        ))
        .expect("prepare");
        assert_eq!(prepared.event_type, "run_started");
        let payload: serde_json::Value =
            serde_json::from_str(&prepared.payload_json).expect("payload");
        assert_eq!(
            payload.get("type").and_then(serde_json::Value::as_str),
            Some(prepared.event_type.as_str())
        );
    }

    #[test]
    fn test_should_reject_corrupted_run_id_when_decoding_row() {
        let connection = Connection::open_in_memory().expect("memory");
        connection
            .execute_batch(
                "CREATE TABLE runs (
                    run_id TEXT,
                    kind TEXT,
                    status TEXT,
                    query TEXT,
                    query_method TEXT,
                    started_at TEXT,
                    completed_at TEXT,
                    compatibility_profile TEXT,
                    event_count INTEGER
                );
                INSERT INTO runs VALUES (
                    'bad/id',
                    'query',
                    'running',
                    NULL,
                    NULL,
                    '2026-08-07T12:34:56.123456789Z',
                    NULL,
                    NULL,
                    0
                );",
            )
            .expect("fixture");
        let mut statement = connection
            .prepare(
                "SELECT run_id, kind, status, query, query_method, started_at, completed_at, \
                 compatibility_profile, event_count FROM runs",
            )
            .expect("prepare");
        let mut rows = statement.query([]).expect("query");
        let row = rows.next().expect("row").expect("first row");
        assert!(run_from_row(row).is_err());
    }

    #[test]
    fn test_should_reject_invalid_event_sequence_when_decoding_row() {
        let connection = Connection::open_in_memory().expect("memory");
        connection
            .execute_batch(
                "CREATE TABLE events (
                    run_id TEXT,
                    sequence INTEGER,
                    schema_version INTEGER,
                    span_id TEXT,
                    parent_span_id TEXT,
                    timestamp TEXT,
                    event_type TEXT,
                    payload_json TEXT
                );
                INSERT INTO events VALUES (
                    'run-1', -1, 1, 'span-1', NULL,
                    '2026-08-07T12:34:56.123456789Z', 'run_started', '{\"type\":\"run_started\"}'
                );
                INSERT INTO events VALUES (
                    'run-1', 0, 1, 'span-1', NULL,
                    '2026-08-07T12:34:56.123456789Z', 'run_started', '{\"type\":\"run_started\"}'
                );",
            )
            .expect("fixture");
        let mut statement = connection
            .prepare(
                "SELECT run_id, sequence, schema_version, span_id, parent_span_id, timestamp, \
                 event_type, payload_json FROM events",
            )
            .expect("prepare");
        let mut rows = statement.query([]).expect("query");
        while let Some(row) = rows.next().expect("row") {
            assert!(envelope_from_row(row).is_err());
        }
    }

    #[test]
    fn test_should_reject_invalid_event_run_id_when_decoding_row() {
        let connection = Connection::open_in_memory().expect("memory");
        connection
            .execute_batch(
                "CREATE TABLE events (
                    run_id TEXT,
                    sequence INTEGER,
                    schema_version INTEGER,
                    span_id TEXT,
                    parent_span_id TEXT,
                    timestamp TEXT,
                    event_type TEXT,
                    payload_json TEXT
                );
                INSERT INTO events VALUES (
                    'bad/id', 1, 1, 'span-1', NULL,
                    '2026-08-07T12:34:56.123456789Z', 'run_started', '{\"type\":\"run_started\"}'
                );",
            )
            .expect("fixture");
        let mut statement = connection
            .prepare(
                "SELECT run_id, sequence, schema_version, span_id, parent_span_id, timestamp, \
                 event_type, payload_json FROM events",
            )
            .expect("prepare");
        let mut rows = statement.query([]).expect("query");
        let row = rows.next().expect("row").expect("first row");
        assert!(envelope_from_row(row).is_err());
    }
}
