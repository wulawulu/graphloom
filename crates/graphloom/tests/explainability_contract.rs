use std::{str::FromStr, sync::Arc};

use chrono::Utc;
use graphloom::explainability::{
    ContextSectionKind, EXPLAINABILITY_SCHEMA_VERSION, ExplainabilityCandidate,
    ExplainabilityContentMode, ExplainabilityContextSection, ExplainabilityEnvelope,
    ExplainabilityEvent, ExplainabilityQueryMethod, ExplainabilityRecord, ExplainabilityRecordType,
    ExplainabilityRun, ExplainabilityRunId, ExplainabilityRunKind, ExplainabilityRunStatus,
    ExplainabilityScore, ExplainabilitySink, ExplainabilitySinkChain, ExplainabilitySpanId,
    NoopExplainabilitySink, QueryStarted, RunStarted, SelectionReason,
};

#[test]
fn test_should_expose_foundational_contracts_to_external_crates()
-> Result<(), Box<dyn std::error::Error>> {
    let run_id = ExplainabilityRunId::from_str("public-run-1")?;
    let span_id = ExplainabilitySpanId::from_str("public-span-1")?;
    let started = ExplainabilityEvent::RunStarted(RunStarted::new(
        ExplainabilityRunKind::Query,
        ExplainabilityContentMode::Metadata,
    ));
    let record = ExplainabilityRecord::new(run_id, Utc::now(), span_id, None, started);

    let noop: Arc<dyn ExplainabilitySink> = Arc::new(NoopExplainabilitySink::new());
    let chain = ExplainabilitySinkChain::new(vec![noop]);
    chain.emit(&record);

    let envelope = ExplainabilityEnvelope::new(1, record)?;
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

    let mut candidate =
        ExplainabilityCandidate::new("entity-1".to_owned(), ExplainabilityRecordType::Entity);
    candidate.score = Some(ExplainabilityScore::try_from(0.91)?);
    candidate.rank = Some(1);
    candidate.selected = true;
    candidate.reason = Some(SelectionReason::AnnResult);
    let candidate_json = serde_json::to_value(&candidate)?;
    assert_eq!(
        serde_json::from_value::<ExplainabilityCandidate>(candidate_json)?,
        candidate
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
