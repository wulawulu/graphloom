//! LLM extraction and materialization for community reports.

use std::{collections::BTreeMap, path::Path};

use futures_util::{StreamExt, stream};
use graphloom_input::gen_sha512_hash;
use graphloom_llm::{
    ChatMessage, CommunityReport, CompletionModel, CompletionRequest, DefaultPrompt, PromptLoader,
    Tokenizer, parse_community_report,
};
use serde::Serialize;

use super::{
    ClaimContextRow, CommunityInputRow, CommunityLocalContext, CommunityReportFindingRow,
    CommunityReportRow, ContextRecords, EntityContextRow, RelationshipContextRow, ReportContextRow,
    build_local_contexts,
};
use crate::{Result, dataframe::invalid_data};

const COMMUNITY_REPORTS_CONTEXT: &str = "create_community_reports";

#[derive(Debug, Clone, Copy)]
pub(crate) struct CommunityReportExtractionConfig<'a> {
    pub(crate) prompt_path: Option<&'a str>,
    pub(crate) max_report_length: usize,
    pub(crate) max_input_length: usize,
    pub(crate) concurrency: usize,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CommunityReportOperationInput<'a> {
    pub(crate) entities: &'a [EntityContextRow],
    pub(crate) relationships: &'a [RelationshipContextRow],
    pub(crate) communities: &'a [CommunityInputRow],
    pub(crate) claims: &'a [ClaimContextRow],
}

pub(crate) struct CommunityReportCallbacks<'a> {
    pub(crate) progress: &'a (dyn Fn(usize, usize) + Sync),
    pub(crate) warning: &'a (dyn Fn(&str) + Sync),
}

#[derive(Debug, Serialize)]
struct ReportPromptValues<'a> {
    input_text: &'a str,
    max_report_length: usize,
}

#[derive(Debug, Clone)]
struct ReportTask {
    index: usize,
    community: CommunityInputRow,
    context: String,
}

pub(crate) async fn create_community_reports(
    model: &dyn CompletionModel,
    prompt_loader: &PromptLoader,
    tokenizer: &dyn Tokenizer,
    input: CommunityReportOperationInput<'_>,
    config: CommunityReportExtractionConfig<'_>,
    callbacks: CommunityReportCallbacks<'_>,
) -> Result<Vec<CommunityReportRow>> {
    let local_contexts = build_local_contexts(
        input.communities,
        input.entities,
        input.relationships,
        input.claims,
        tokenizer,
        config.max_input_length,
    )?;
    let mut communities_by_level: BTreeMap<i64, Vec<CommunityInputRow>> = BTreeMap::new();
    for community in input.communities {
        communities_by_level
            .entry(community.level)
            .or_default()
            .push(community.clone());
    }
    for rows in communities_by_level.values_mut() {
        rows.sort_by_key(|community| community.community);
    }

    let total = input.communities.len();
    let mut completed = 0usize;
    let mut reports = Vec::new();
    let mut reports_by_community = BTreeMap::new();
    for (_, level_communities) in communities_by_level.iter().rev() {
        let mut tasks = Vec::with_capacity(level_communities.len());
        for community in level_communities {
            let context = resolve_report_context(
                community,
                &local_contexts,
                &reports_by_community,
                tokenizer,
                config.max_input_length,
            )?;
            tasks.push(ReportTask {
                index: tasks.len(),
                community: community.clone(),
                context,
            });
        }

        let mut level_results = stream::iter(tasks)
            .map(|task| async move {
                let index = task.index;
                let community_id = task.community.community;
                let result = extract_report_for_community(
                    model,
                    prompt_loader,
                    tokenizer,
                    task,
                    config,
                    callbacks.warning,
                )
                .await?;
                Ok::<_, crate::GraphLoomError>((index, community_id, result))
            })
            .buffer_unordered(config.concurrency.max(1));

        let mut completed_level = Vec::new();
        while let Some(result) = level_results.next().await {
            let result = result?;
            completed = completed.saturating_add(1);
            (callbacks.progress)(completed, total);
            completed_level.push(result);
        }
        completed_level.sort_by_key(|(index, _, _)| *index);
        for (_, community_id, maybe_report) in completed_level {
            if let Some(report) = maybe_report {
                reports_by_community.insert(community_id, report.clone());
                reports.push(report);
            }
        }
    }
    Ok(reports)
}

fn resolve_report_context(
    community: &CommunityInputRow,
    local_contexts: &BTreeMap<i64, CommunityLocalContext>,
    reports_by_community: &BTreeMap<i64, CommunityReportRow>,
    tokenizer: &dyn Tokenizer,
    max_input_length: usize,
) -> Result<String> {
    let Some(local_context) = local_contexts.get(&community.community) else {
        return Ok(String::new());
    };
    if !local_context.was_truncated && local_context.full_token_count <= max_input_length {
        return Ok(local_context.context.clone());
    }

    let mut children_by_size = community
        .children
        .iter()
        .filter_map(|child| {
            local_contexts
                .get(child)
                .map(|context| (*child, context.token_count))
        })
        .collect::<Vec<_>>();
    children_by_size.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    if children_by_size.is_empty() {
        return Ok(local_context.context.clone());
    }

    let mut report_children = Vec::new();
    let mut detail_children = stable_children(community);
    for (child, _) in &children_by_size {
        if reports_by_community.contains_key(child) {
            report_children.push(*child);
            detail_children.retain(|detail_child| detail_child != child);
            let candidate = render_mixed_child_context(
                &report_children,
                &detail_children,
                local_contexts,
                reports_by_community,
            )?;
            if tokenizer.count(&candidate)? <= max_input_length {
                return Ok(candidate);
            }
        }
    }

    let mut report_records = ContextRecords::default();
    let mut best = String::new();
    for (child, _) in children_by_size {
        if let Some(report) = reports_by_community.get(&child) {
            let mut candidate_records = report_records.clone();
            candidate_records.add_report(ReportContextRow {
                community: child,
                full_content: report.full_content.clone(),
            });
            let candidate = super::context::render_context(&candidate_records)?;
            if tokenizer.count(&candidate)? > max_input_length {
                continue;
            }
            report_records = candidate_records;
            best = candidate;
        }
    }
    if best.is_empty() {
        Ok(local_context.context.clone())
    } else {
        Ok(best)
    }
}

fn render_mixed_child_context(
    report_children: &[i64],
    detail_children: &[i64],
    local_contexts: &BTreeMap<i64, CommunityLocalContext>,
    reports_by_community: &BTreeMap<i64, CommunityReportRow>,
) -> Result<String> {
    let mut records = ContextRecords::default();
    for community in report_children {
        if let Some(report) = reports_by_community.get(community) {
            records.add_report(ReportContextRow {
                community: *community,
                full_content: report.full_content.clone(),
            });
        }
    }
    let mut detail_children = detail_children.to_vec();
    detail_children.sort_unstable();
    for child in detail_children {
        if let Some(context) = local_contexts.get(&child) {
            records.merge_details(&context.full_records);
        }
    }
    super::context::render_context(&records)
}

fn stable_children(community: &CommunityInputRow) -> Vec<i64> {
    let mut children = community.children.clone();
    children.sort_unstable();
    children
}

async fn extract_report_for_community(
    model: &dyn CompletionModel,
    prompt_loader: &PromptLoader,
    tokenizer: &dyn Tokenizer,
    task: ReportTask,
    config: CommunityReportExtractionConfig<'_>,
    warning: &(dyn Fn(&str) + Sync),
) -> Result<Option<CommunityReportRow>> {
    if task.context.trim().is_empty() {
        warning(&format!(
            "community report {} skipped because context is empty",
            task.community.community
        ));
        return Ok(None);
    }
    let token_count = tokenizer.count(&task.context)?;
    if token_count > config.max_input_length {
        return Err(invalid_data(
            COMMUNITY_REPORTS_CONTEXT,
            &format!(
                "community report {} context exceeded max_input_length: {token_count} > {}",
                task.community.community, config.max_input_length
            ),
        ));
    }
    let rendered_prompt = prompt_loader
        .render(
            DefaultPrompt::CommunityReport,
            config.prompt_path.map(Path::new),
            &ReportPromptValues {
                input_text: &task.context,
                max_report_length: config.max_report_length,
            },
        )
        .await?;
    let response = match model
        .complete(CompletionRequest {
            messages: vec![ChatMessage::user(rendered_prompt)],
            temperature: None,
            top_p: None,
            max_tokens: None,
            response_format: Some("json_object".to_owned()),
            cache_namespace: None,
        })
        .await
    {
        Ok(response) => response,
        Err(source) => {
            warning(&format!(
                "community report {} failed: {source}",
                task.community.community
            ));
            return Ok(None);
        }
    };
    if response.content.trim().is_empty() {
        warning(&format!(
            "community report {} returned an empty response",
            task.community.community
        ));
        return Ok(None);
    }
    let report = match parse_community_report(&response.content) {
        Ok(report) => report,
        Err(source) => {
            warning(&format!(
                "community report {} returned invalid JSON: {source}",
                task.community.community
            ));
            return Ok(None);
        }
    };
    if let Err(source) = validate_report(&report) {
        warning(&format!(
            "community report {} failed validation: {source}",
            task.community.community
        ));
        return Ok(None);
    }
    materialize_report(&task.community, &report).map(Some)
}

fn validate_report(report: &CommunityReport) -> Result<()> {
    if report.title.trim().is_empty() {
        return Err(invalid_data(
            COMMUNITY_REPORTS_CONTEXT,
            "report title is empty",
        ));
    }
    if report.summary.trim().is_empty() {
        return Err(invalid_data(
            COMMUNITY_REPORTS_CONTEXT,
            "report summary is empty",
        ));
    }
    if !report.rating.is_finite() || !(0.0..=10.0).contains(&report.rating) {
        return Err(invalid_data(
            COMMUNITY_REPORTS_CONTEXT,
            "report rating must be finite and between 0 and 10",
        ));
    }
    for finding in &report.findings {
        if finding.summary.trim().is_empty() || finding.explanation.trim().is_empty() {
            return Err(invalid_data(
                COMMUNITY_REPORTS_CONTEXT,
                "report finding summary and explanation must be non-empty",
            ));
        }
    }
    Ok(())
}

fn materialize_report(
    community: &CommunityInputRow,
    report: &CommunityReport,
) -> Result<CommunityReportRow> {
    let full_content = full_content(report);
    Ok(CommunityReportRow {
        id: gen_sha512_hash([full_content.as_str()]),
        human_readable_id: community.community,
        community: community.community,
        level: community.level,
        parent: community.parent,
        children: community.children.clone(),
        title: report.title.clone(),
        summary: report.summary.clone(),
        full_content,
        rank: report.rating,
        rating_explanation: report.rating_explanation.clone(),
        findings: report
            .findings
            .iter()
            .map(|finding| CommunityReportFindingRow {
                summary: finding.summary.clone(),
                explanation: finding.explanation.clone(),
            })
            .collect(),
        full_content_json: serde_json::to_string_pretty(report)?,
        period: community.period.clone(),
        size: community.size,
    })
}

fn full_content(report: &CommunityReport) -> String {
    let mut sections = vec![format!("# {}", report.title), report.summary.clone()];
    for finding in &report.findings {
        sections.push(format!("## {}", finding.summary));
        sections.push(finding.explanation.clone());
    }
    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use async_trait::async_trait;
    use graphloom_llm::{CompletionResponse, LlmError, TiktokenTokenizer, Tokenizer};
    use tokio::time::sleep;

    use super::*;

    #[test]
    fn test_should_materialize_report_with_snake_case_json_and_sha_id() {
        let community = CommunityInputRow {
            community: 7,
            level: 1,
            parent: 0,
            children: vec![8],
            entity_ids: vec!["e1".to_owned()],
            period: "2026-07-08".to_owned(),
            size: 1,
        };
        let report = parse_community_report(
            "{\"title\":\"Community \
             Title\",\"summary\":\"Summary\",\"rating\":4.5,\"rating_explanation\":\"Reason\",\"\
             findings\":[{\"summary\":\"Finding\",\"explanation\":\"Explanation\"}]}",
        )
        .expect("report should parse");

        let row = materialize_report(&community, &report).expect("row");

        assert_eq!(
            row.full_content,
            "# Community Title\n\nSummary\n\n## Finding\n\nExplanation"
        );
        assert_eq!(row.id, gen_sha512_hash([row.full_content.as_str()]));
        assert_eq!(row.human_readable_id, 7);
        assert!(row.full_content_json.contains("rating_explanation"));
        assert!(!row.full_content_json.contains("ratingExplanation"));
    }

    #[test]
    fn test_should_reject_rating_outside_valid_range() {
        let report = CommunityReport {
            title: "Title".to_owned(),
            summary: "Summary".to_owned(),
            rating: 11.0,
            rating_explanation: "Reason".to_owned(),
            findings: Vec::new(),
        };

        assert!(validate_report(&report).is_err());
    }

    #[tokio::test]
    async fn test_should_keep_order_after_out_of_order_parallel_reports() {
        let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");
        let in_flight = Arc::new(AtomicUsize::new(0));
        let max_in_flight = Arc::new(AtomicUsize::new(0));
        let model = DelayedReportModel {
            in_flight: Arc::clone(&in_flight),
            max_in_flight: Arc::clone(&max_in_flight),
            fail_alice: false,
        };
        let progress = Arc::new(Mutex::new(Vec::new()));

        let rows = create_community_reports(
            &model,
            &PromptLoader::new("."),
            &tokenizer,
            CommunityReportOperationInput {
                entities: &test_entities(),
                relationships: &[],
                communities: &test_communities(),
                claims: &[],
            },
            CommunityReportExtractionConfig {
                prompt_path: None,
                max_report_length: 2_000,
                max_input_length: 8_000,
                concurrency: 2,
            },
            CommunityReportCallbacks {
                progress: &|completed, total| {
                    progress
                        .lock()
                        .expect("progress lock")
                        .push((completed, total));
                },
                warning: &|_| {},
            },
        )
        .await
        .expect("reports should build");

        assert_eq!(
            rows.iter()
                .map(|row| (row.community, row.title.as_str()))
                .collect::<Vec<_>>(),
            vec![(0, "Alice Report"), (1, "Bob Report")]
        );
        assert_eq!(max_in_flight.load(Ordering::SeqCst), 2);
        assert_eq!(
            progress.lock().expect("progress lock").last().copied(),
            Some((2, 2))
        );
    }

    #[tokio::test]
    async fn test_should_skip_single_failed_community_report() {
        let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");
        let model = DelayedReportModel {
            in_flight: Arc::new(AtomicUsize::new(0)),
            max_in_flight: Arc::new(AtomicUsize::new(0)),
            fail_alice: true,
        };
        let warnings = Arc::new(Mutex::new(Vec::new()));

        let rows = create_community_reports(
            &model,
            &PromptLoader::new("."),
            &tokenizer,
            CommunityReportOperationInput {
                entities: &test_entities(),
                relationships: &[],
                communities: &test_communities(),
                claims: &[],
            },
            CommunityReportExtractionConfig {
                prompt_path: None,
                max_report_length: 2_000,
                max_input_length: 8_000,
                concurrency: 2,
            },
            CommunityReportCallbacks {
                progress: &|_, _| {},
                warning: &|message| {
                    warnings
                        .lock()
                        .expect("warnings lock")
                        .push(message.to_owned());
                },
            },
        )
        .await
        .expect("operation should continue after one community fails");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].community, 1);
        assert!(
            warnings
                .lock()
                .expect("warnings lock")
                .iter()
                .any(|warning| warning.contains("community report 0 failed"))
        );
    }

    #[tokio::test]
    async fn test_should_skip_empty_context_without_calling_model() {
        let tokenizer = WordCountTokenizer;
        let calls = Arc::new(AtomicUsize::new(0));
        let model = CapturingReportModel {
            calls: Arc::clone(&calls),
            prompts: Arc::new(Mutex::new(Vec::new())),
            fail_marker: None,
        };
        let warnings = Arc::new(Mutex::new(Vec::new()));
        let progress = Arc::new(Mutex::new(Vec::new()));
        let entities = vec![entity_with_description(
            "entity-a",
            0,
            "OVERSIZED",
            1,
            "one two three",
        )];
        let communities = vec![community(7, 0, -1, Vec::new(), vec!["entity-a"])];

        let rows = create_community_reports(
            &model,
            &PromptLoader::new("."),
            &tokenizer,
            CommunityReportOperationInput {
                entities: &entities,
                relationships: &[],
                communities: &communities,
                claims: &[],
            },
            CommunityReportExtractionConfig {
                prompt_path: None,
                max_report_length: 2_000,
                max_input_length: 0,
                concurrency: 1,
            },
            CommunityReportCallbacks {
                progress: &|completed, total| {
                    progress
                        .lock()
                        .expect("progress lock")
                        .push((completed, total));
                },
                warning: &|message| {
                    warnings
                        .lock()
                        .expect("warnings lock")
                        .push(message.to_owned());
                },
            },
        )
        .await
        .expect("operation should skip empty context");

        assert!(rows.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            progress.lock().expect("progress lock").last().copied(),
            Some((1, 1))
        );
        assert!(
            warnings
                .lock()
                .expect("warnings lock")
                .iter()
                .any(|warning| warning.contains("community report 7 skipped"))
        );
    }

    #[tokio::test]
    async fn test_should_use_parent_local_context_when_full_context_fits() {
        let tokenizer = WordCountTokenizer;
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let model = CapturingReportModel {
            calls: Arc::new(AtomicUsize::new(0)),
            prompts: Arc::clone(&prompts),
            fail_marker: None,
        };
        let entities = vec![
            entity_with_description("entity-a", 0, "ALICE", 2, "alice"),
            entity_with_description("entity-b", 1, "BOB", 1, "bob"),
        ];
        let communities = vec![
            community(1, 1, 0, Vec::new(), vec!["entity-a"]),
            community(0, 0, -1, vec![1], vec!["entity-a", "entity-b"]),
        ];

        let rows = create_community_reports(
            &model,
            &PromptLoader::new("."),
            &tokenizer,
            CommunityReportOperationInput {
                entities: &entities,
                relationships: &[],
                communities: &communities,
                claims: &[],
            },
            CommunityReportExtractionConfig {
                prompt_path: None,
                max_report_length: 2_000,
                max_input_length: 100,
                concurrency: 1,
            },
            CommunityReportCallbacks {
                progress: &|_, _| {},
                warning: &|_| {},
            },
        )
        .await
        .expect("reports should build");

        assert_eq!(rows.len(), 2);
        let prompts = prompts.lock().expect("prompts lock");
        let parent_prompt = prompts.last().expect("parent prompt");
        assert!(!parent_prompt.contains("----Reports-----"));
        assert!(parent_prompt.contains("ALICE"));
        assert!(parent_prompt.contains("BOB"));
    }

    #[tokio::test]
    async fn test_should_replace_oversized_child_detail_with_report() {
        let tokenizer = WordCountTokenizer;
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let model = CapturingReportModel {
            calls: Arc::new(AtomicUsize::new(0)),
            prompts: Arc::clone(&prompts),
            fail_marker: None,
        };
        let entities = vec![
            entity_with_description(
                "entity-a",
                0,
                "ALICE",
                3,
                "alpha one two three four five six seven eight nine ten eleven twelve",
            ),
            entity_with_description("entity-b", 1, "BOB", 2, "beta one two three four"),
        ];
        let communities = vec![
            community(1, 1, 0, Vec::new(), vec!["entity-a"]),
            community(2, 1, 0, Vec::new(), vec!["entity-b"]),
            community(0, 0, -1, vec![1, 2], vec!["entity-a", "entity-b"]),
        ];

        let rows = create_community_reports(
            &model,
            &PromptLoader::new("."),
            &tokenizer,
            CommunityReportOperationInput {
                entities: &entities,
                relationships: &[],
                communities: &communities,
                claims: &[],
            },
            CommunityReportExtractionConfig {
                prompt_path: None,
                max_report_length: 2_000,
                max_input_length: 26,
                concurrency: 1,
            },
            CommunityReportCallbacks {
                progress: &|_, _| {},
                warning: &|_| {},
            },
        )
        .await
        .expect("reports should build");

        assert_eq!(rows.len(), 3);
        let prompts = prompts.lock().expect("prompts lock");
        let parent_prompt = prompts.last().expect("parent prompt");
        assert!(parent_prompt.contains("----Reports-----"));
        assert!(parent_prompt.contains("# Captured 1"));
        assert!(!parent_prompt.contains("alpha one two three four five six seven eight"));
        assert!(parent_prompt.contains("BOB"));
    }

    #[tokio::test]
    async fn test_should_keep_failed_child_detail_and_use_other_child_report() {
        let tokenizer = WordCountTokenizer;
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let model = CapturingReportModel {
            calls: Arc::new(AtomicUsize::new(0)),
            prompts: Arc::clone(&prompts),
            fail_marker: Some("FAIL_CHILD"),
        };
        let entities = vec![
            entity_with_description("entity-a", 0, "FAIL_CHILD", 3, "failed small"),
            entity_with_description(
                "entity-b",
                1,
                "BOB",
                2,
                "beta one two three four five six seven eight",
            ),
        ];
        let communities = vec![
            community(1, 1, 0, Vec::new(), vec!["entity-a"]),
            community(2, 1, 0, Vec::new(), vec!["entity-b"]),
            community(0, 0, -1, vec![1, 2], vec!["entity-a", "entity-b"]),
        ];

        let rows = create_community_reports(
            &model,
            &PromptLoader::new("."),
            &tokenizer,
            CommunityReportOperationInput {
                entities: &entities,
                relationships: &[],
                communities: &communities,
                claims: &[],
            },
            CommunityReportExtractionConfig {
                prompt_path: None,
                max_report_length: 2_000,
                max_input_length: 21,
                concurrency: 1,
            },
            CommunityReportCallbacks {
                progress: &|_, _| {},
                warning: &|_| {},
            },
        )
        .await
        .expect("operation should continue");

        assert_eq!(rows.len(), 2);
        let prompts = prompts.lock().expect("prompts lock");
        let parent_prompt = prompts.last().expect("parent prompt");
        assert!(parent_prompt.contains("# Captured 2"));
        assert!(parent_prompt.contains("FAIL_CHILD"));
        assert!(!parent_prompt.contains("# Captured 1"));
    }

    #[test]
    fn test_should_substitute_children_by_token_count_then_community() {
        let tokenizer = WordCountTokenizer;
        let entities = vec![
            entity_with_description("entity-a", 0, "ALICE", 1, "one two three four five six"),
            entity_with_description("entity-b", 1, "BOB", 1, "one two three four five six"),
            entity_with_description("entity-c", 2, "CAROL", 1, "tiny"),
        ];
        let communities = vec![
            community(1, 1, 0, Vec::new(), vec!["entity-a"]),
            community(2, 1, 0, Vec::new(), vec!["entity-b"]),
            community(3, 1, 0, Vec::new(), vec!["entity-c"]),
            community(
                0,
                0,
                -1,
                vec![1, 2, 3],
                vec!["entity-a", "entity-b", "entity-c"],
            ),
        ];
        let local_contexts =
            build_local_contexts(&communities, &entities, &[], &[], &tokenizer, 100)
                .expect("contexts");
        let reports = reports_by_community(&[(1, "first"), (2, "second"), (3, "third")]);

        let context = resolve_report_context(
            communities.last().expect("parent"),
            &local_contexts,
            &reports,
            &tokenizer,
            24,
        )
        .expect("context");

        assert!(
            context.find("# Report 1").expect("report 1")
                < context.find("# Report 2").expect("report 2")
        );
        assert!(context.contains("CAROL"));
    }

    #[test]
    fn test_should_use_reports_only_fallback_with_token_limit() {
        let tokenizer = WordCountTokenizer;
        let entities = vec![
            entity_with_description("entity-a", 0, "ALICE", 1, "one two three four"),
            entity_with_description("entity-b", 1, "BOB", 1, "one two three four"),
        ];
        let communities = vec![
            community(1, 1, 0, Vec::new(), vec!["entity-a"]),
            community(2, 1, 0, Vec::new(), vec!["entity-b"]),
            community(0, 0, -1, vec![1, 2], vec!["entity-a", "entity-b"]),
        ];
        let local_contexts =
            build_local_contexts(&communities, &entities, &[], &[], &tokenizer, 100)
                .expect("contexts");
        let reports = reports_by_community(&[
            (1, "one two three four five six seven eight nine ten"),
            (2, "small"),
        ]);

        let context = resolve_report_context(
            communities.last().expect("parent"),
            &local_contexts,
            &reports,
            &tokenizer,
            8,
        )
        .expect("context");

        assert!(context.contains("----Reports-----"));
        assert!(!context.contains("one two three four five six seven eight nine ten"));
        assert!(context.contains("# Report 2"));
        assert!(tokenizer.count(&context).expect("count") <= 8);
        assert!(!context.contains("-----Entities-----"));
    }

    #[test]
    fn test_should_render_each_mixed_detail_section_once() {
        let tokenizer = WordCountTokenizer;
        let entities = vec![
            entity_with_description("entity-a", 0, "ALICE", 1, "one two three four"),
            entity_with_description("entity-b", 1, "BOB", 1, "two three four five"),
            entity_with_description("entity-c", 2, "CAROL", 1, "three four five six"),
        ];
        let relationships = vec![
            relationship(10, "ALICE", "BOB", "works", 5),
            relationship(11, "BOB", "CAROL", "knows", 4),
        ];
        let claims = vec![claim(1, "ALICE"), claim(2, "BOB")];
        let communities = vec![
            community(1, 1, 0, Vec::new(), vec!["entity-a", "entity-b"]),
            community(2, 1, 0, Vec::new(), vec!["entity-b", "entity-c"]),
            community(
                0,
                0,
                -1,
                vec![1, 2],
                vec!["entity-a", "entity-b", "entity-c"],
            ),
        ];
        let local_contexts = build_local_contexts(
            &communities,
            &entities,
            &relationships,
            &claims,
            &tokenizer,
            100,
        )
        .expect("contexts");
        let reports = reports_by_community(&[(1, "small")]);

        let context =
            render_mixed_child_context(&[1], &[2], &local_contexts, &reports).expect("context");

        assert_eq!(context.matches("-----Entities-----").count(), 1);
        assert_eq!(context.matches("-----Claims-----").count(), 1);
        assert_eq!(context.matches("-----Relationships-----").count(), 1);
    }

    #[derive(Debug)]
    struct DelayedReportModel {
        in_flight: Arc<AtomicUsize>,
        max_in_flight: Arc<AtomicUsize>,
        fail_alice: bool,
    }

    #[derive(Debug)]
    struct CapturingReportModel {
        calls: Arc<AtomicUsize>,
        prompts: Arc<Mutex<Vec<String>>>,
        fail_marker: Option<&'static str>,
    }

    #[async_trait]
    impl CompletionModel for CapturingReportModel {
        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> graphloom_llm::Result<CompletionResponse> {
            let prompt = request
                .messages
                .first()
                .map(|message| message.content.clone())
                .unwrap_or_default();
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            self.prompts
                .lock()
                .expect("prompts lock")
                .push(prompt.clone());
            if call == 1
                && self
                    .fail_marker
                    .is_some_and(|marker| prompt.contains(marker))
            {
                return Err(LlmError::InvalidResponse {
                    model_instance: "test".to_owned(),
                    operation: "completion",
                    message: "forced failure".to_owned(),
                });
            }
            Ok(CompletionResponse {
                content: json_report(&format!("Captured {call}")),
                usage: None,
                request_id: None,
            })
        }
    }

    #[async_trait]
    impl CompletionModel for DelayedReportModel {
        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> graphloom_llm::Result<CompletionResponse> {
            let current = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            update_max(&self.max_in_flight, current);
            let prompt = request
                .messages
                .first()
                .map(|message| message.content.as_str())
                .unwrap_or_default();
            let is_alice = prompt.contains("ALICE");
            if is_alice {
                sleep(Duration::from_millis(40)).await;
            } else {
                sleep(Duration::from_millis(5)).await;
            }
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            if is_alice && self.fail_alice {
                return Err(LlmError::InvalidResponse {
                    model_instance: "test".to_owned(),
                    operation: "completion",
                    message: "forced failure".to_owned(),
                });
            }
            let title = if is_alice {
                "Alice Report"
            } else {
                "Bob Report"
            };
            Ok(CompletionResponse {
                content: format!(
                    "{{\"title\":\"{title}\",\"summary\":\"Summary\",\"rating\":5,\"\
                     rating_explanation\":\"Reason\",\"findings\":[{{\"summary\":\"Finding\",\"\
                     explanation\":\"Explanation\"}}]}}"
                ),
                usage: None,
                request_id: None,
            })
        }
    }

    fn update_max(max_in_flight: &AtomicUsize, current: usize) {
        let mut observed = max_in_flight.load(Ordering::SeqCst);
        while current > observed {
            match max_in_flight.compare_exchange(
                observed,
                current,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(value) => observed = value,
            }
        }
    }

    fn test_entities() -> Vec<EntityContextRow> {
        vec![
            EntityContextRow {
                id: "entity-a".to_owned(),
                human_readable_id: 0,
                title: "ALICE".to_owned(),
                description: "Alice".to_owned(),
                degree: 1,
            },
            EntityContextRow {
                id: "entity-b".to_owned(),
                human_readable_id: 1,
                title: "BOB".to_owned(),
                description: "Bob".to_owned(),
                degree: 1,
            },
        ]
    }

    fn test_communities() -> Vec<CommunityInputRow> {
        vec![
            CommunityInputRow {
                community: 0,
                level: 0,
                parent: -1,
                children: Vec::new(),
                entity_ids: vec!["entity-a".to_owned()],
                period: "2026-07-08".to_owned(),
                size: 1,
            },
            CommunityInputRow {
                community: 1,
                level: 0,
                parent: -1,
                children: Vec::new(),
                entity_ids: vec!["entity-b".to_owned()],
                period: "2026-07-08".to_owned(),
                size: 1,
            },
        ]
    }

    fn community(
        community: i64,
        level: i64,
        parent: i64,
        children: Vec<i64>,
        entity_ids: Vec<&str>,
    ) -> CommunityInputRow {
        CommunityInputRow {
            community,
            level,
            parent,
            children,
            entity_ids: entity_ids.into_iter().map(str::to_owned).collect(),
            period: "2026-07-08".to_owned(),
            size: 1,
        }
    }

    fn entity_with_description(
        id: &str,
        human_readable_id: i64,
        title: &str,
        degree: i64,
        description: &str,
    ) -> EntityContextRow {
        EntityContextRow {
            id: id.to_owned(),
            human_readable_id,
            title: title.to_owned(),
            description: description.to_owned(),
            degree,
        }
    }

    fn relationship(
        human_readable_id: i64,
        source: &str,
        target: &str,
        description: &str,
        combined_degree: i64,
    ) -> RelationshipContextRow {
        RelationshipContextRow {
            id: format!("rel-{human_readable_id}"),
            human_readable_id,
            source: source.to_owned(),
            target: target.to_owned(),
            description: description.to_owned(),
            combined_degree,
        }
    }

    fn claim(human_readable_id: i64, subject_id: &str) -> ClaimContextRow {
        ClaimContextRow {
            human_readable_id,
            subject_id: subject_id.to_owned(),
            claim_type: "TYPE".to_owned(),
            status: "TRUE".to_owned(),
            description: "claim".to_owned(),
        }
    }

    fn reports_by_community(rows: &[(i64, &str)]) -> BTreeMap<i64, CommunityReportRow> {
        rows.iter()
            .map(|(community, summary)| {
                (
                    *community,
                    CommunityReportRow {
                        id: format!("report-{community}"),
                        human_readable_id: *community,
                        community: *community,
                        level: 1,
                        parent: 0,
                        children: Vec::new(),
                        title: format!("Report {community}"),
                        summary: (*summary).to_owned(),
                        full_content: format!("# Report {community}\n\n{summary}"),
                        rank: 5.0,
                        rating_explanation: "ok".to_owned(),
                        findings: Vec::new(),
                        full_content_json: "{}".to_owned(),
                        period: "2026-07-08".to_owned(),
                        size: 1,
                    },
                )
            })
            .collect()
    }

    fn json_report(title: &str) -> String {
        format!(
            r#"{{"title":"{title}","summary":"Summary","rating":5,"rating_explanation":"Reason","findings":[{{"summary":"Finding","explanation":"Explanation"}}]}}"#
        )
    }

    #[derive(Debug)]
    struct WordCountTokenizer;

    impl Tokenizer for WordCountTokenizer {
        fn encode(&self, text: &str) -> graphloom_llm::Result<Vec<u32>> {
            Ok(vec![0; self.count(text)?])
        }

        fn decode(&self, _tokens: &[u32]) -> graphloom_llm::Result<String> {
            Ok(String::new())
        }

        fn count(&self, text: &str) -> graphloom_llm::Result<usize> {
            Ok(text
                .split(|character: char| !character.is_alphanumeric())
                .filter(|token| !token.is_empty())
                .count())
        }
    }
}
