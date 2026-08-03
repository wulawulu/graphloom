//! Stable, provider-neutral contracts for explaining `GraphLoom` operations.
//!
//! Core orchestration will create [`ExplainabilityRecord`] values with business identity and
//! span relationships. Sinks consume those records synchronously and must hand blocking work to
//! another execution context. A persistence adapter later assigns the per-run sequence and creates
//! an [`ExplainabilityEnvelope`]; this module deliberately contains no writer or allocator.
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
mod record;
mod sink;
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
pub use record::{
    EXPLAINABILITY_SCHEMA_VERSION, ExplainabilityContractError, ExplainabilityEnvelope,
    ExplainabilityRecord, ExplainabilityRunId, ExplainabilitySpanId,
};
pub use sink::{ExplainabilitySink, ExplainabilitySinkChain, NoopExplainabilitySink};
