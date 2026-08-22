//! Stable, provider-neutral contracts for explaining `GraphLoom` operations.
//!
//! Core orchestration will create [`ExplainabilityRecord`] values with business identity and
//! span relationships. Sinks accept shared records asynchronously, applying bounded-queue
//! backpressure and reporting delivery or finalization failures. A persistence adapter assigns
//! the per-run sequence and creates an [`ExplainabilityEnvelope`]. The JSONL Recorder in this
//! module and the [`StoreExplainabilityRecorder`] provide bounded, single-writer persistence
//! adapters (file-backed and Store-backed respectively). An optional [`ExplainabilityLiveHub`]
//! fans out the exact Store-backed envelope after persistence succeeds and the writer commits its
//! sequence; realtime lag never weakens the reliable Core-to-Store boundary.
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
mod drift;
mod dto;
mod event;
mod jsonl;
mod live_hub;
mod record;
mod sink;
#[cfg(feature = "sqlite-store")]
mod sqlite;
mod store;
mod store_recorder;
mod validation;

pub use content_mode::ExplainabilityContentMode;
pub use drift::{
    DriftActionAttemptCompleted, DriftActionAttemptStarted, DriftActionContextBuilt,
    DriftDepthActionsSelected, DriftExplorationStarted, DriftHydeCompleted, DriftHydeStarted,
    DriftPrimerCompleted, DriftPrimerFoldCompleted, DriftPrimerFoldStarted, DriftPrimerStarted,
    DriftRankedReportEvidence, DriftReduceContextBuilt, DriftReportsRanked,
};
pub use dto::{
    ContextSectionKind, DynamicCommunityRatingEvidence, ExplainabilityCandidate,
    ExplainabilityContextSection, ExplainabilityQueryMethod, ExplainabilityRecordType,
    ExplainabilityRun, ExplainabilityRunKind, ExplainabilityRunStatus, ExplainabilityScore,
    GlobalMapPointDecision, GlobalMapPointDecisionReason, GlobalMapPointEvidence, SelectionReason,
};
pub use event::{
    BasicRetrievalSkipReason, BasicRetrievalSkipped, CandidatesFiltered, CandidatesRetrieved,
    CommunityReportsSelected, ContextBudgetAllocated, ContextCompleted, ContextSectionBudget,
    ContextSectionBuilt, CovariatesSelected, DynamicCommunityRatingAttemptStarted,
    DynamicCommunitySelectionCompleted, DynamicCommunitySelectionStarted,
    DynamicCommunityTraversalWaveStarted, DynamicTraversalWaveSource, EmbeddingCompleted,
    EmbeddingStarted, EntitiesSelected, ExplainabilityEvent, ExplainabilityWarning,
    GlobalContextBuilt, GlobalMapBatchBuilt, GlobalMapPointsProduced, GlobalMapStarted,
    GlobalReduceContextBuilt, GlobalReduceSkipReason, GlobalReduceSkipped, GraphExpansionStarted,
    LlmRequestCompleted, LlmRequestStarted, MappingQueryBuilt, QueryStarted, RelationshipsSelected,
    RunCompleted, RunFailed, RunStarted, TextUnitsSelected,
};
pub use jsonl::{
    JsonlExplainabilityError, JsonlExplainabilityOptions, JsonlExplainabilityRecorder,
};
pub use live_hub::{
    ExplainabilityLiveHub, ExplainabilityLiveHubOptions, ExplainabilityLiveRecvError,
    ExplainabilityLiveSubscription,
};
pub use record::{
    EXPLAINABILITY_SCHEMA_VERSION, ExplainabilityContractError, ExplainabilityEnvelope,
    ExplainabilityRecord, ExplainabilityRunId, ExplainabilitySpanId,
};
pub use sink::{
    ExplainabilitySink, ExplainabilitySinkChain, ExplainabilitySinkError,
    ExplainabilitySinkFailure, ExplainabilitySinkOperation, NoopExplainabilitySink,
};
#[cfg(feature = "sqlite-store")]
pub use sqlite::SqliteExplainabilityStore;
pub use store::{
    DEFAULT_EVENT_QUERY_LIMIT, DEFAULT_RUN_QUERY_LIMIT, EventQuery, ExplainabilityStore,
    ExplainabilityStoreError, InMemoryExplainabilityStore, MAX_EVENT_QUERY_LIMIT,
    MAX_RUN_QUERY_LIMIT, RunCompletion, RunListCursor, RunQuery,
};
pub use store_recorder::{
    StoreExplainabilityError, StoreExplainabilityOperation, StoreExplainabilityOptions,
    StoreExplainabilityRecorder,
};
