//! Stable, provider-neutral contracts for explaining `GraphLoom` operations.
//!
//! Core orchestration will create [`ExplainabilityRecord`] values with business identity and
//! span relationships. Sinks accept shared records asynchronously, applying bounded-queue
//! backpressure and reporting delivery or finalization failures. A persistence adapter assigns
//! the per-run sequence and creates an [`ExplainabilityEnvelope`]. The JSONL Recorder in this
//! module provides one bounded, single-writer persistence adapter.
//!
//! The JSON schema is versioned by [`EXPLAINABILITY_SCHEMA_VERSION`]. Adding optional fields is a
//! backward-compatible change. Removing fields or changing their meaning requires a schema-version
//! increase. Adding an [`ExplainabilityEvent`] variant is also additive, but this version's
//! strongly typed deserializer reports an unknown variant; long-lived readers should preserve or
//! report unknown event records until a future tolerant-reader contract is introduced.
//!
//! Persisted short metadata strings are limited to 256 bytes, safe diagnostic messages to 4 KiB,
//! and explicitly enabled content fields to 1 MiB. One event may contain at most 10,000 candidates
//! or record IDs and at most 32 context-section budgets. Serialization and deserialization both
//! enforce these limits.

mod content_mode;
mod dto;
mod event;
mod jsonl;
mod record;
mod sink;
mod store;
mod validation;

pub use content_mode::ExplainabilityContentMode;
pub use dto::{
    ContextSectionKind, ExplainabilityCandidate, ExplainabilityContextSection,
    ExplainabilityQueryMethod, ExplainabilityRecordType, ExplainabilityRun, ExplainabilityRunKind,
    ExplainabilityRunStatus, ExplainabilityScore, SelectionReason,
};
pub use event::{
    CandidatesFiltered, CandidatesRetrieved, CommunityReportsSelected, ContextBudgetAllocated,
    ContextCompleted, ContextSectionBudget, ContextSectionBuilt, CovariatesSelected,
    EmbeddingCompleted, EmbeddingStarted, EntitiesSelected, ExplainabilityEvent,
    ExplainabilityWarning, GraphExpansionStarted, LlmRequestCompleted, LlmRequestStarted,
    MappingQueryBuilt, QueryStarted, RelationshipsSelected, RunCompleted, RunFailed, RunStarted,
    TextUnitsSelected,
};
pub use jsonl::{
    JsonlExplainabilityError, JsonlExplainabilityOptions, JsonlExplainabilityRecorder,
};
pub use record::{
    EXPLAINABILITY_SCHEMA_VERSION, ExplainabilityContractError, ExplainabilityEnvelope,
    ExplainabilityRecord, ExplainabilityRunId, ExplainabilitySpanId,
};
pub use sink::{
    ExplainabilitySink, ExplainabilitySinkChain, ExplainabilitySinkError,
    ExplainabilitySinkFailure, ExplainabilitySinkOperation, NoopExplainabilitySink,
};
pub use store::{
    DEFAULT_EVENT_QUERY_LIMIT, DEFAULT_RUN_QUERY_LIMIT, EventQuery, ExplainabilityStore,
    ExplainabilityStoreError, InMemoryExplainabilityStore, MAX_EVENT_QUERY_LIMIT,
    MAX_RUN_QUERY_LIMIT, RunCompletion, RunListCursor, RunQuery,
};
