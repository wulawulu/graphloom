//! LLM extraction and materialization for community reports.

use std::{collections::BTreeMap, path::Path};

use futures_util::{StreamExt, stream};
use graphloom_input::gen_sha512_hash;
use graphloom_llm::{
    ChatMessage, CommunityReport, CompletionModel, CompletionRequest, Tokenizer,
    parse_community_report,
};
use serde::Serialize;

use super::{
    ClaimContextRow, CommunityInputRow, CommunityReportFindingRow, CommunityReportRow,
    EntityContextRow, RelationshipContextRow, build_local_contexts,
};
use crate::{
    Result,
    prompts::{PromptKind, PromptRepository, PromptTemplate},
};

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
    prompt_repository: &PromptRepository,
    tokenizer: &dyn Tokenizer,
    input: CommunityReportOperationInput<'_>,
    config: CommunityReportExtractionConfig<'_>,
    callbacks: CommunityReportCallbacks<'_>,
) -> Result<Vec<CommunityReportRow>> {
    let report_template = prompt_repository
        .load(
            PromptKind::CommunityReportGraph,
            config.prompt_path.map(Path::new),
        )
        .await?;
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
    for (_, level_communities) in communities_by_level.iter().rev() {
        let mut tasks = Vec::with_capacity(level_communities.len());
        for community in level_communities {
            let context = local_contexts
                .get(&community.community)
                .map(|local_context| local_context.context.clone())
                .unwrap_or_default();
            tasks.push(ReportTask {
                index: tasks.len(),
                community: community.clone(),
                context,
            });
        }

        let mut level_results = stream::iter(tasks)
            .map(|task| {
                let report_template = report_template.clone();
                async move {
                    let index = task.index;
                    let result = extract_report_for_community(
                        model,
                        &report_template,
                        task,
                        config,
                        callbacks.warning,
                    )
                    .await?;
                    Ok::<_, crate::GraphLoomError>((index, result))
                }
            })
            .buffer_unordered(config.concurrency.max(1));

        let mut completed_level = Vec::new();
        while let Some(result) = level_results.next().await {
            let result = result?;
            completed = completed.saturating_add(1);
            (callbacks.progress)(completed, total);
            completed_level.push(result);
        }
        completed_level.sort_by_key(|(index, _)| *index);
        for (_, maybe_report) in completed_level {
            if let Some(report) = maybe_report {
                reports.push(report);
            }
        }
    }
    Ok(reports)
}

async fn extract_report_for_community(
    model: &dyn CompletionModel,
    report_template: &PromptTemplate,
    task: ReportTask,
    config: CommunityReportExtractionConfig<'_>,
    warning: &(dyn Fn(&str) + Sync),
) -> Result<Option<CommunityReportRow>> {
    let rendered_prompt = report_template
        .bind(&ReportPromptValues {
            input_text: &task.context,
            max_report_length: config.max_report_length,
        })?
        .render()?;
    let response = match model
        .complete(CompletionRequest::new(vec![ChatMessage::user(
            rendered_prompt,
        )]))
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
    let Ok(content) = response.content() else {
        warning(&format!(
            "community report {} returned an empty response",
            task.community.community
        ));
        return Ok(None);
    };
    let report = match parse_community_report(content) {
        Ok(report) => report,
        Err(source) => {
            warning(&format!(
                "community report {} returned invalid JSON: {source}",
                task.community.community
            ));
            return Ok(None);
        }
    };
    materialize_report(&task.community, &report).map(Some)
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
            &PromptRepository::new("."),
            &tokenizer,
            CommunityReportOperationInput {
                entities: &test_entities(),
                relationships: &test_relationships(),
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
            &PromptRepository::new("."),
            &tokenizer,
            CommunityReportOperationInput {
                entities: &test_entities(),
                relationships: &test_relationships(),
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
    async fn test_should_skip_invalid_cached_response_like_graphrag() {
        let tokenizer = WordCountTokenizer;
        let warnings = Arc::new(Mutex::new(Vec::new()));
        let communities = vec![community(7, 0, -1, Vec::new(), vec!["entity-a"])];
        let entities = vec![entity_with_description("entity-a", 0, "ALICE", 1, "alice")];

        let rows = create_community_reports(
            &InvalidReportModel,
            &PromptRepository::new("."),
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
                max_input_length: 8_000,
                concurrency: 1,
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
        .expect("invalid response should not fail the workflow");

        assert!(rows.is_empty());
        assert!(
            warnings
                .lock()
                .expect("warnings lock")
                .iter()
                .any(|warning| warning.contains("returned invalid JSON"))
        );
    }

    #[tokio::test]
    async fn test_should_call_model_for_empty_context_without_response_format() {
        let tokenizer = WordCountTokenizer;
        let calls = Arc::new(AtomicUsize::new(0));
        let response_formats = Arc::new(Mutex::new(Vec::new()));
        let model = CapturingReportModel {
            calls: Arc::clone(&calls),
            prompts: Arc::new(Mutex::new(Vec::new())),
            response_formats: Arc::clone(&response_formats),
            fail_marker: None,
        };
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
            &PromptRepository::new("."),
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
                warning: &|_| {},
            },
        )
        .await
        .expect("operation should report empty context");

        assert_eq!(rows.len(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            response_formats
                .lock()
                .expect("response formats lock")
                .as_slice(),
            &[None]
        );
        assert_eq!(
            progress.lock().expect("progress lock").last().copied(),
            Some((1, 1))
        );
    }

    #[tokio::test]
    async fn test_should_not_substitute_child_reports_into_parent_context() {
        let tokenizer = WordCountTokenizer;
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let model = CapturingReportModel {
            calls: Arc::new(AtomicUsize::new(0)),
            prompts: Arc::clone(&prompts),
            response_formats: Arc::new(Mutex::new(Vec::new())),
            fail_marker: None,
        };
        let entities = vec![
            entity_with_description("entity-a", 0, "ALICE", 2, "alice"),
            entity_with_description("entity-b", 1, "BOB", 1, "bob"),
        ];
        let relationships = vec![relationship(10, "ALICE", "BOB", "works", 3)];
        let communities = vec![
            community(1, 1, 0, Vec::new(), vec!["entity-a"]),
            community(0, 0, -1, vec![1], vec!["entity-a", "entity-b"]),
        ];

        let rows = create_community_reports(
            &model,
            &PromptRepository::new("."),
            &tokenizer,
            CommunityReportOperationInput {
                entities: &entities,
                relationships: &relationships,
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
        response_formats: Arc<Mutex<Vec<Option<serde_json::Value>>>>,
        fail_marker: Option<&'static str>,
    }

    #[derive(Debug)]
    struct InvalidReportModel;

    #[async_trait]
    impl CompletionModel for InvalidReportModel {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> graphloom_llm::Result<CompletionResponse> {
            Ok(CompletionResponse::text_for_test(
                "test",
                r#"{"title":"Title","summary":"Summary","rating":5,"rating_explanation":"Reason","findings":[{"summary":"Missing explanation"}]}"#,
            ))
        }
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
                .and_then(|message| message.content.as_text())
                .unwrap_or_default();
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            self.prompts
                .lock()
                .expect("prompts lock")
                .push(prompt.to_owned());
            self.response_formats
                .lock()
                .expect("response formats lock")
                .push(request.response_format.clone());
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
            Ok(CompletionResponse::text_for_test(
                "test",
                json_report(&format!("Captured {call}")),
            ))
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
            Ok(CompletionResponse::text_for_test(
                "test",
                format!(
                    "{{\"title\":\"{title}\",\"summary\":\"Summary\",\"rating\":5,\"\
                     rating_explanation\":\"Reason\",\"findings\":[{{\"summary\":\"Finding\",\"\
                     explanation\":\"Explanation\"}}]}}"
                ),
            ))
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

    fn test_relationships() -> Vec<RelationshipContextRow> {
        vec![
            relationship(0, "ALICE", "ALICE", "Alice relationship", 2),
            relationship(1, "BOB", "BOB", "Bob relationship", 2),
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
