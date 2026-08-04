use std::{
    error::Error,
    str::FromStr,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use chrono::Utc;
use graphloom::{
    explainability::{
        CandidatesFiltered, CandidatesRetrieved, CommunityReportsSelected, ContextSectionKind,
        CovariatesSelected, EXPLAINABILITY_SCHEMA_VERSION, EntitiesSelected,
        ExplainabilityCandidate, ExplainabilityContentMode, ExplainabilityContextSection,
        ExplainabilityContractError, ExplainabilityEnvelope, ExplainabilityEvent,
        ExplainabilityQueryMethod, ExplainabilityRecord, ExplainabilityRecordType,
        ExplainabilityRun, ExplainabilityRunId, ExplainabilityRunKind, ExplainabilityRunStatus,
        ExplainabilityScore, ExplainabilitySink, ExplainabilitySinkChain, ExplainabilitySinkError,
        ExplainabilitySinkOperation, ExplainabilitySpanId, JsonlExplainabilityOptions,
        JsonlExplainabilityRecorder, NoopExplainabilitySink, QueryStarted, RelationshipsSelected,
        RunStarted, SelectionReason, TextUnitsSelected,
    },
    query::{QueryExplainabilityOptions, QueryOptions, SearchMethod},
};
use serde_json::json;

type TestResult = Result<(), Box<dyn Error>>;

#[derive(Debug, Clone)]
enum ObservedCall {
    Emit {
        sink_name: &'static str,
        record: Arc<ExplainabilityRecord>,
    },
    FinishRun {
        sink_name: &'static str,
        run_id: ExplainabilityRunId,
    },
}

#[derive(Debug)]
struct TestSink {
    name: &'static str,
    calls: Arc<Mutex<Vec<ObservedCall>>>,
    emit_error: Option<ExplainabilitySinkError>,
    finish_error: Option<ExplainabilitySinkError>,
}

impl TestSink {
    fn successful(name: &'static str, calls: Arc<Mutex<Vec<ObservedCall>>>) -> Self {
        Self {
            name,
            calls,
            emit_error: None,
            finish_error: None,
        }
    }

    fn failing(
        name: &'static str,
        calls: Arc<Mutex<Vec<ObservedCall>>>,
        emit_error: Option<ExplainabilitySinkError>,
        finish_error: Option<ExplainabilitySinkError>,
    ) -> Self {
        Self {
            name,
            calls,
            emit_error,
            finish_error,
        }
    }
}

#[async_trait]
impl ExplainabilitySink for TestSink {
    async fn emit(&self, record: Arc<ExplainabilityRecord>) -> Result<(), ExplainabilitySinkError> {
        lock_recovering_poison(&self.calls).push(ObservedCall::Emit {
            sink_name: self.name,
            record,
        });
        match &self.emit_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    async fn finish_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<(), ExplainabilitySinkError> {
        lock_recovering_poison(&self.calls).push(ObservedCall::FinishRun {
            sink_name: self.name,
            run_id: run_id.clone(),
        });
        match &self.finish_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}

fn lock_recovering_poison<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn sample_record() -> Result<Arc<ExplainabilityRecord>, ExplainabilityContractError> {
    Ok(Arc::new(ExplainabilityRecord::new(
        ExplainabilityRunId::from_str("public-run-1")?,
        Utc::now(),
        ExplainabilitySpanId::from_str("public-span-1")?,
        None,
        ExplainabilityEvent::RunStarted(RunStarted::new(
            ExplainabilityRunKind::Query,
            ExplainabilityContentMode::Metadata,
        )),
    )))
}

#[tokio::test]
async fn test_should_persist_public_records_through_jsonl_recorder() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("public/records.jsonl");
    let recorder =
        JsonlExplainabilityRecorder::create(JsonlExplainabilityOptions::new(path.clone())).await?;
    let sink = ExplainabilitySinkChain::new(vec![
        recorder.sink(),
        Arc::new(NoopExplainabilitySink::new()),
    ]);
    let record = sample_record()?;
    let run_id = record.run_id.clone();
    sink.emit(Arc::clone(&record)).await?;
    sink.finish_run(&run_id).await?;
    let bytes = tokio::fs::read(recorder.path()).await?;
    let line = bytes.strip_suffix(b"\n").ok_or("missing JSONL LF")?;
    let envelope: ExplainabilityEnvelope = serde_json::from_slice(line)?;
    assert_eq!(envelope.schema_version(), EXPLAINABILITY_SCHEMA_VERSION);
    assert_eq!(envelope.sequence(), 1);
    assert_eq!(envelope.record.run_id, run_id);
    recorder.shutdown().await?;
    Ok(())
}

fn candidate(id: &str, record_type: ExplainabilityRecordType) -> ExplainabilityCandidate {
    ExplainabilityCandidate::new(id.to_owned(), record_type)
}

fn observed_labels(calls: &[ObservedCall]) -> Vec<String> {
    calls
        .iter()
        .map(|call| match call {
            ObservedCall::Emit { sink_name, .. } => format!("{sink_name}:emit"),
            ObservedCall::FinishRun { sink_name, .. } => format!("{sink_name}:finish"),
        })
        .collect()
}

#[tokio::test]
async fn test_should_expose_async_foundational_contracts_to_external_crates() -> TestResult {
    let record = sample_record()?;
    let run_id = record.run_id.clone();
    let noop: Arc<dyn ExplainabilitySink> = Arc::new(NoopExplainabilitySink::new());
    noop.emit(Arc::clone(&record)).await?;
    noop.emit(Arc::clone(&record)).await?;
    noop.finish_run(&run_id).await?;
    noop.finish_run(&run_id).await?;

    let empty_chain = ExplainabilitySinkChain::default();
    assert!(empty_chain.is_empty());
    empty_chain.emit(Arc::clone(&record)).await?;
    empty_chain.finish_run(&run_id).await?;

    let chain = ExplainabilitySinkChain::new(vec![noop]);
    assert_eq!(chain.len(), 1);
    chain.emit(Arc::clone(&record)).await?;
    chain.finish_run(&run_id).await?;
    assert_eq!(record.run_id, run_id);

    let envelope = ExplainabilityEnvelope::new(1, record.as_ref().clone())?;
    assert_eq!(envelope.schema_version(), EXPLAINABILITY_SCHEMA_VERSION);
    assert_eq!(envelope.sequence(), 1);
    assert_eq!(envelope.record.run_id.as_str(), "public-run-1");

    let query =
        ExplainabilityEvent::QueryStarted(QueryStarted::new(ExplainabilityQueryMethod::Local));
    assert_eq!(
        serde_json::to_value(query)?
            .get("type")
            .and_then(serde_json::Value::as_str),
        Some("query_started")
    );

    let mut explainability_candidate = candidate("entity-1", ExplainabilityRecordType::Entity);
    explainability_candidate.score = Some(ExplainabilityScore::try_from(0.91)?);
    explainability_candidate.rank = Some(1);
    explainability_candidate.selected = true;
    explainability_candidate.reason = Some(SelectionReason::AnnResult);
    let candidate_json = serde_json::to_value(&explainability_candidate)?;
    assert_eq!(
        serde_json::from_value::<ExplainabilityCandidate>(candidate_json)?,
        explainability_candidate
    );

    let mut section = ExplainabilityContextSection::new(ContextSectionKind::Entities, 2_048);
    section.tokens_used = 128;
    section.candidate_count = 1;
    section.selected_count = 1;
    section.selected_record_ids.push("entity-1".to_owned());
    let section_json = serde_json::to_value(&section)?;
    assert_eq!(
        serde_json::from_value::<ExplainabilityContextSection>(section_json)?,
        section
    );

    let mut run = ExplainabilityRun::new(
        ExplainabilityRunId::from_str("public-run-2")?,
        ExplainabilityRunKind::Query,
        Utc::now(),
    );
    run.status = ExplainabilityRunStatus::Running;
    run.query_method = Some(ExplainabilityQueryMethod::Local);
    let run_json = serde_json::to_value(&run)?;
    assert_eq!(serde_json::from_value::<ExplainabilityRun>(run_json)?, run);
    Ok(())
}

#[test]
fn test_should_expose_request_scoped_query_explainability_options() -> TestResult {
    let sink: Arc<dyn ExplainabilitySink> = Arc::new(NoopExplainabilitySink::new());
    let run_id = ExplainabilityRunId::from_str("studio-run")?;
    let explainability = QueryExplainabilityOptions::new(
        run_id.clone(),
        ExplainabilityContentMode::Content,
        Arc::clone(&sink),
    );
    assert_eq!(explainability.run_id(), &run_id);
    assert_eq!(
        explainability.content_mode(),
        ExplainabilityContentMode::Content
    );
    assert!(Arc::ptr_eq(explainability.sink(), &sink));

    let defaults = QueryOptions::new(
        std::path::PathBuf::from("project"),
        "query".to_owned(),
        SearchMethod::Local,
    );
    assert!(defaults.explainability.is_none());
    let configured = defaults.with_explainability(explainability);
    assert_eq!(
        configured
            .explainability
            .as_ref()
            .map(QueryExplainabilityOptions::run_id),
        Some(&run_id)
    );

    let generated = QueryExplainabilityOptions::generated(
        ExplainabilityContentMode::Metadata,
        Arc::new(NoopExplainabilitySink::new()),
    );
    assert!(!generated.run_id().as_str().is_empty());
    Ok(())
}

#[tokio::test]
async fn test_should_fan_out_shared_record_and_finish_in_registration_order() -> TestResult {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sinks = ["sink-1", "sink-2", "sink-3"]
        .into_iter()
        .map(|name| {
            Arc::new(TestSink::successful(name, Arc::clone(&calls))) as Arc<dyn ExplainabilitySink>
        })
        .collect();
    let chain = ExplainabilitySinkChain::new(sinks);
    let record = sample_record()?;
    let run_id = record.run_id.clone();

    chain.emit(Arc::clone(&record)).await?;
    chain.finish_run(&run_id).await?;

    let observed = lock_recovering_poison(&calls);
    assert_eq!(
        observed_labels(&observed),
        [
            "sink-1:emit",
            "sink-2:emit",
            "sink-3:emit",
            "sink-1:finish",
            "sink-2:finish",
            "sink-3:finish",
        ]
    );
    assert!(observed.iter().all(|call| match call {
        ObservedCall::Emit {
            record: observed_record,
            ..
        } => Arc::ptr_eq(observed_record, &record),
        ObservedCall::FinishRun {
            run_id: observed_run_id,
            ..
        } => observed_run_id == &run_id,
    }));
    Ok(())
}

#[tokio::test]
async fn test_should_aggregate_emit_failures_and_continue_all_sinks() -> TestResult {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sinks: Vec<Arc<dyn ExplainabilitySink>> = vec![
        Arc::new(TestSink::failing(
            "sink-1",
            Arc::clone(&calls),
            Some(ExplainabilitySinkError::Closed),
            None,
        )),
        Arc::new(TestSink::failing(
            "sink-2",
            Arc::clone(&calls),
            Some(ExplainabilitySinkError::Unavailable),
            None,
        )),
        Arc::new(TestSink::successful("sink-3", Arc::clone(&calls))),
    ];
    let result = ExplainabilitySinkChain::new(sinks)
        .emit(sample_record()?)
        .await;
    let error = match result {
        Ok(()) => return Err("failing sinks must produce an aggregate error".into()),
        Err(error) => error,
    };

    assert_eq!(
        observed_labels(&lock_recovering_poison(&calls)),
        ["sink-1:emit", "sink-2:emit", "sink-3:emit"]
    );
    match error {
        ExplainabilitySinkError::Chain {
            operation,
            failures,
        } => {
            assert_eq!(operation, ExplainabilitySinkOperation::Emit);
            assert_eq!(failures.len(), 2);
            let first = failures.first().ok_or("first failure must be retained")?;
            assert_eq!(first.sink_index(), 0);
            assert_eq!(first.error(), &ExplainabilitySinkError::Closed);
            let second = failures.get(1).ok_or("second failure must be retained")?;
            assert_eq!(second.sink_index(), 1);
            assert_eq!(second.error(), &ExplainabilitySinkError::Unavailable);
        }
        _ => return Err("expected an emit chain error".into()),
    }
    Ok(())
}

#[tokio::test]
async fn test_should_call_sinks_before_and_after_a_middle_emit_failure() -> TestResult {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sinks: Vec<Arc<dyn ExplainabilitySink>> = vec![
        Arc::new(TestSink::successful("sink-1", Arc::clone(&calls))),
        Arc::new(TestSink::failing(
            "sink-2",
            Arc::clone(&calls),
            Some(ExplainabilitySinkError::WriterFailed),
            None,
        )),
        Arc::new(TestSink::successful("sink-3", Arc::clone(&calls))),
    ];
    let result = ExplainabilitySinkChain::new(sinks)
        .emit(sample_record()?)
        .await;
    let error = match result {
        Ok(()) => return Err("the middle sink must produce an aggregate error".into()),
        Err(error) => error,
    };

    assert_eq!(
        observed_labels(&lock_recovering_poison(&calls)),
        ["sink-1:emit", "sink-2:emit", "sink-3:emit"]
    );
    match error {
        ExplainabilitySinkError::Chain {
            operation,
            failures,
        } => {
            assert_eq!(operation, ExplainabilitySinkOperation::Emit);
            assert_eq!(failures.len(), 1);
            let failure = failures.first().ok_or("middle failure must be retained")?;
            assert_eq!(failure.sink_index(), 1);
            assert_eq!(failure.error(), &ExplainabilitySinkError::WriterFailed);
        }
        _ => return Err("expected a middle-sink chain error".into()),
    }
    Ok(())
}

#[tokio::test]
async fn test_should_aggregate_finish_failures_and_continue_all_sinks() -> TestResult {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sinks: Vec<Arc<dyn ExplainabilitySink>> = vec![
        Arc::new(TestSink::failing(
            "sink-1",
            Arc::clone(&calls),
            None,
            Some(ExplainabilitySinkError::WriterFailed),
        )),
        Arc::new(TestSink::successful("sink-2", Arc::clone(&calls))),
        Arc::new(TestSink::failing(
            "sink-3",
            Arc::clone(&calls),
            None,
            Some(ExplainabilitySinkError::RunFinalizationFailed),
        )),
    ];
    let run_id = ExplainabilityRunId::from_str("finish-run")?;
    let result = ExplainabilitySinkChain::new(sinks)
        .finish_run(&run_id)
        .await;
    let error = match result {
        Ok(()) => return Err("failing sinks must produce an aggregate error".into()),
        Err(error) => error,
    };

    assert_eq!(
        observed_labels(&lock_recovering_poison(&calls)),
        ["sink-1:finish", "sink-2:finish", "sink-3:finish"]
    );
    match error {
        ExplainabilitySinkError::Chain {
            operation,
            failures,
        } => {
            assert_eq!(operation, ExplainabilitySinkOperation::FinishRun);
            assert_eq!(failures.len(), 2);
            let first = failures.first().ok_or("first failure must be retained")?;
            assert_eq!(first.sink_index(), 0);
            assert_eq!(first.error(), &ExplainabilitySinkError::WriterFailed);
            let second = failures.get(1).ok_or("second failure must be retained")?;
            assert_eq!(second.sink_index(), 2);
            assert_eq!(
                second.error(),
                &ExplainabilitySinkError::RunFinalizationFailed
            );
        }
        _ => return Err("expected a finish chain error".into()),
    }
    Ok(())
}

#[tokio::test]
async fn test_should_report_single_sink_error_with_stable_index() -> TestResult {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sink: Arc<dyn ExplainabilitySink> = Arc::new(TestSink::failing(
        "only",
        Arc::clone(&calls),
        Some(ExplainabilitySinkError::RecordNotAccepted),
        None,
    ));
    let result = ExplainabilitySinkChain::new(vec![sink])
        .emit(sample_record()?)
        .await;
    let error = match result {
        Ok(()) => return Err("the failing sink must return an error".into()),
        Err(error) => error,
    };

    match error {
        ExplainabilitySinkError::Chain {
            operation,
            failures,
        } => {
            assert_eq!(operation, ExplainabilitySinkOperation::Emit);
            assert_eq!(failures.len(), 1);
            let failure = failures.first().ok_or("single failure must be retained")?;
            assert_eq!(failure.sink_index(), 0);
            assert_eq!(failure.error(), &ExplainabilitySinkError::RecordNotAccepted);
        }
        _ => return Err("expected a single-entry chain error".into()),
    }
    Ok(())
}

#[test]
fn test_should_construct_homogeneous_candidate_collections() -> TestResult {
    let entity = candidate("entity-1", ExplainabilityRecordType::Entity);
    let retrieved =
        CandidatesRetrieved::try_new(ExplainabilityRecordType::Entity, vec![entity.clone()])?;
    assert_eq!(retrieved.record_type(), ExplainabilityRecordType::Entity);
    assert_eq!(retrieved.candidates(), &[entity]);

    let relationship = candidate("relationship-1", ExplainabilityRecordType::Relationship);
    let filtered = CandidatesFiltered::try_new(
        ExplainabilityRecordType::Relationship,
        vec![relationship.clone()],
    )?;
    assert_eq!(
        filtered.record_type(),
        ExplainabilityRecordType::Relationship
    );
    assert_eq!(filtered.candidates(), &[relationship]);

    let empty = CandidatesRetrieved::try_new(ExplainabilityRecordType::TextUnit, Vec::new())?;
    assert_eq!(empty.record_type(), ExplainabilityRecordType::TextUnit);
    assert!(empty.candidates().is_empty());
    Ok(())
}

#[test]
fn test_should_reject_mismatched_and_mixed_candidate_collections() {
    let mismatch = CandidatesRetrieved::try_new(
        ExplainabilityRecordType::Entity,
        vec![candidate(
            "relationship-1",
            ExplainabilityRecordType::Relationship,
        )],
    );
    assert!(matches!(
        mismatch,
        Err(ExplainabilityContractError::CandidateTypeMismatch {
            expected: ExplainabilityRecordType::Entity,
            actual: ExplainabilityRecordType::Relationship,
            candidate_index: 0,
        })
    ));

    let mixed = CandidatesFiltered::try_new(
        ExplainabilityRecordType::Entity,
        vec![
            candidate("entity-1", ExplainabilityRecordType::Entity),
            candidate("text-unit-1", ExplainabilityRecordType::TextUnit),
        ],
    );
    assert!(matches!(
        mixed,
        Err(ExplainabilityContractError::CandidateTypeMismatch {
            expected: ExplainabilityRecordType::Entity,
            actual: ExplainabilityRecordType::TextUnit,
            candidate_index: 1,
        })
    ));
}

#[test]
fn test_should_validate_candidate_types_during_deserialization() {
    let contradictory = json!({
        "record_type": "entity",
        "candidates": [{
            "id": "relationship-1",
            "record_type": "relationship",
            "selected": false,
        }],
    });
    assert!(serde_json::from_value::<CandidatesRetrieved>(contradictory.clone()).is_err());
    assert!(serde_json::from_value::<CandidatesFiltered>(contradictory).is_err());

    let mixed_event = json!({
        "type": "candidates_filtered",
        "record_type": "entity",
        "candidates": [
            {
                "id": "entity-1",
                "record_type": "entity",
                "selected": true,
            },
            {
                "id": "relationship-1",
                "record_type": "relationship",
                "selected": false,
            },
        ],
    });
    assert!(serde_json::from_value::<ExplainabilityEvent>(mixed_event).is_err());
}

#[test]
fn test_should_round_trip_candidate_events_without_changing_schema() -> TestResult {
    let retrieved = ExplainabilityEvent::CandidatesRetrieved(CandidatesRetrieved::try_new(
        ExplainabilityRecordType::Entity,
        vec![candidate("entity-1", ExplainabilityRecordType::Entity)],
    )?);
    let filtered = ExplainabilityEvent::CandidatesFiltered(CandidatesFiltered::try_new(
        ExplainabilityRecordType::Relationship,
        vec![candidate(
            "relationship-1",
            ExplainabilityRecordType::Relationship,
        )],
    )?);

    for (event, discriminator) in [
        (retrieved, "candidates_retrieved"),
        (filtered, "candidates_filtered"),
    ] {
        let value = serde_json::to_value(&event)?;
        assert_eq!(
            value.get("type").and_then(serde_json::Value::as_str),
            Some(discriminator)
        );
        assert_eq!(serde_json::from_value::<ExplainabilityEvent>(value)?, event);
    }

    let empty = CandidatesFiltered::try_new(ExplainabilityRecordType::Community, Vec::new())?;
    let empty_value = serde_json::to_value(&empty)?;
    assert_eq!(empty_value.get("record_type"), Some(&json!("community")));
    assert_eq!(empty_value.get("candidates"), Some(&json!([])));
    assert_eq!(
        serde_json::from_value::<CandidatesFiltered>(empty_value)?,
        empty
    );
    assert_eq!(EXPLAINABILITY_SCHEMA_VERSION, 1);
    Ok(())
}

macro_rules! typed_selection_contract {
    (
        $test_name:ident,
        $payload:ty,
        $field:ident,
        $variant:ident,
        $record_type:expr,
        $wrong_type:expr,
        $discriminator:literal
    ) => {
        #[test]
        fn $test_name() -> TestResult {
            let expected = candidate("record-1", $record_type);
            let payload = <$payload>::try_new(vec![expected.clone()])?;
            assert_eq!(payload.$field(), &[expected]);
            assert!(<$payload>::try_new(Vec::new())?.$field().is_empty());

            let mismatch = <$payload>::try_new(vec![candidate("wrong-1", $wrong_type)]);
            let Err(ExplainabilityContractError::CandidateTypeMismatch {
                expected,
                actual,
                candidate_index,
            }) = mismatch
            else {
                return Err("wrong candidate type must be rejected".into());
            };
            assert_eq!(expected, $record_type);
            assert_eq!(actual, $wrong_type);
            assert_eq!(candidate_index, 0);

            let contradictory = json!({
                stringify!($field): [{
                    "id": "wrong-1",
                    "record_type": serde_json::to_value($wrong_type)?,
                    "selected": true,
                }],
            });
            assert!(serde_json::from_value::<$payload>(contradictory.clone()).is_err());
            let mut contradictory_event = contradictory;
            contradictory_event
                .as_object_mut()
                .ok_or("selection payload must be an object")?
                .insert("type".to_owned(), json!($discriminator));
            assert!(
                serde_json::from_value::<ExplainabilityEvent>(contradictory_event).is_err()
            );

            let event = ExplainabilityEvent::$variant(payload);
            let value = serde_json::to_value(&event)?;
            assert_eq!(
                value.get("type").and_then(serde_json::Value::as_str),
                Some($discriminator)
            );
            assert_eq!(serde_json::from_value::<ExplainabilityEvent>(value)?, event);
            Ok(())
        }
    };
}

typed_selection_contract!(
    test_should_enforce_entities_selected_candidate_type,
    EntitiesSelected,
    entities,
    EntitiesSelected,
    ExplainabilityRecordType::Entity,
    ExplainabilityRecordType::Relationship,
    "entities_selected"
);
typed_selection_contract!(
    test_should_enforce_relationships_selected_candidate_type,
    RelationshipsSelected,
    relationships,
    RelationshipsSelected,
    ExplainabilityRecordType::Relationship,
    ExplainabilityRecordType::Entity,
    "relationships_selected"
);
typed_selection_contract!(
    test_should_enforce_community_reports_selected_candidate_type,
    CommunityReportsSelected,
    community_reports,
    CommunityReportsSelected,
    ExplainabilityRecordType::CommunityReport,
    ExplainabilityRecordType::Community,
    "community_reports_selected"
);
typed_selection_contract!(
    test_should_enforce_covariates_selected_candidate_type,
    CovariatesSelected,
    covariates,
    CovariatesSelected,
    ExplainabilityRecordType::Covariate,
    ExplainabilityRecordType::TextUnit,
    "covariates_selected"
);
typed_selection_contract!(
    test_should_enforce_text_units_selected_candidate_type,
    TextUnitsSelected,
    text_units,
    TextUnitsSelected,
    ExplainabilityRecordType::TextUnit,
    ExplainabilityRecordType::Covariate,
    "text_units_selected"
);
