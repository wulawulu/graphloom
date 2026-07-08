//! Covariate extraction operations and table codecs.

use std::path::Path;

use futures_util::{StreamExt, stream};
use graphloom_llm::{
    ChatMessage, CompletionModel, CompletionRequest, DefaultPrompt, PromptLoader,
    parse_claim_tuples,
};
use polars_core::prelude::*;
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{GraphLoomError, Result, dataframe::string_value};

const EXTRACT_COVARIATES_CONTEXT: &str = "extract_covariates";
const DEFAULT_CLAIM_ENTITY_TYPES: &[&str] = &["organization", "person", "geo", "event"];

#[derive(Debug, Clone)]
pub(crate) struct TextUnitInput {
    pub(crate) id: String,
    pub(crate) text: String,
}

#[derive(Debug, Clone)]
pub(crate) struct CovariateRow {
    pub(crate) id: String,
    pub(crate) human_readable_id: i64,
    pub(crate) covariate_type: String,
    pub(crate) claim_type: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) subject_id: Option<String>,
    pub(crate) object_id: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) start_date: Option<String>,
    pub(crate) end_date: Option<String>,
    pub(crate) source_text: Option<String>,
    pub(crate) text_unit_id: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ClaimExtractionConfig<'a> {
    pub(crate) prompt_path: Option<&'a str>,
    pub(crate) claim_description: &'a str,
    pub(crate) entity_types: &'a [String],
    pub(crate) max_gleanings: usize,
    pub(crate) concurrency: usize,
}

#[derive(Debug, Serialize)]
struct ClaimPromptValues<'a> {
    input_text: &'a str,
    entity_specs: &'a [String],
    claim_description: &'a str,
}

pub(crate) fn default_claim_entity_types() -> Vec<String> {
    DEFAULT_CLAIM_ENTITY_TYPES
        .iter()
        .map(|entity_type| (*entity_type).to_owned())
        .collect()
}

pub(crate) fn read_text_unit_inputs(dataframe: &DataFrame) -> Result<Vec<TextUnitInput>> {
    let ids = dataframe.column("id")?.str()?;
    let texts = dataframe.column("text")?.str()?;
    let mut rows = Vec::with_capacity(dataframe.height());
    for index in 0..dataframe.height() {
        rows.push(TextUnitInput {
            id: string_value(ids.get(index), "id", EXTRACT_COVARIATES_CONTEXT)?,
            text: string_value(texts.get(index), "text", EXTRACT_COVARIATES_CONTEXT)?,
        });
    }
    Ok(rows)
}

pub(crate) async fn extract_covariates(
    model: &dyn CompletionModel,
    prompt_loader: &PromptLoader,
    text_units: &[TextUnitInput],
    config: ClaimExtractionConfig<'_>,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Vec<CovariateRow>> {
    let concurrency = config.concurrency.max(1);
    let mut results = stream::iter(text_units.iter().cloned().enumerate())
        .map(|(index, text_unit)| async move {
            extract_claims_for_text_unit(model, prompt_loader, &text_unit, &config)
                .await
                .map(|claims| (index, text_unit, claims))
        })
        .buffer_unordered(concurrency);

    let mut completed = 0usize;
    let mut extracted = Vec::with_capacity(text_units.len());
    while let Some(result) = results.next().await {
        let result = result?;
        completed = completed.saturating_add(1);
        progress(completed, text_units.len());
        extracted.push(result);
    }
    extracted.sort_by_key(|(index, _, _)| *index);

    let mut rows = Vec::new();
    for (_, text_unit, claims) in extracted {
        for claim in claims {
            rows.push(CovariateRow {
                id: Uuid::new_v4().to_string(),
                human_readable_id: rows.len() as i64,
                covariate_type: "claim".to_owned(),
                claim_type: claim.claim_type,
                description: claim.description,
                subject_id: claim.subject_id,
                object_id: claim.object_id,
                status: claim.status,
                start_date: claim.start_date,
                end_date: claim.end_date,
                source_text: claim.source_text,
                text_unit_id: text_unit.id.clone(),
            });
        }
    }
    Ok(rows)
}

async fn extract_claims_for_text_unit(
    model: &dyn CompletionModel,
    prompt_loader: &PromptLoader,
    text_unit: &TextUnitInput,
    config: &ClaimExtractionConfig<'_>,
) -> Result<Vec<graphloom_llm::ClaimRecord>> {
    let initial_prompt = render_claim_prompt(
        prompt_loader,
        config.prompt_path,
        &text_unit.text,
        config.entity_types,
        config.claim_description,
    )
    .await?;
    let mut messages = vec![ChatMessage::user(initial_prompt)];
    let initial = model
        .complete(CompletionRequest {
            messages: messages.clone(),
            temperature: None,
            top_p: None,
            max_tokens: None,
            response_format: None,
            cache_namespace: None,
        })
        .await?
        .content;
    messages.push(ChatMessage::assistant(initial.clone()));

    for glean_index in 0..config.max_gleanings {
        messages.push(ChatMessage::user(
            render_builtin_prompt(prompt_loader, DefaultPrompt::ExtractClaimsContinue).await?,
        ));
        let extension = model
            .complete(CompletionRequest {
                messages: messages.clone(),
                temperature: None,
                top_p: None,
                max_tokens: None,
                response_format: None,
                cache_namespace: None,
            })
            .await?
            .content;
        messages.push(ChatMessage::assistant(extension));

        if glean_index >= config.max_gleanings.saturating_sub(1) {
            break;
        }

        messages.push(ChatMessage::user(
            render_builtin_prompt(prompt_loader, DefaultPrompt::ExtractClaimsLoop).await?,
        ));
        let response = model
            .complete(CompletionRequest {
                messages: messages.clone(),
                temperature: None,
                top_p: None,
                max_tokens: None,
                response_format: None,
                cache_namespace: None,
            })
            .await?
            .content;
        if response != "Y" {
            break;
        }
    }

    // Keep this compatible with Microsoft GraphRAG's current claim extractor:
    // continuation requests are sent, but tuple parsing still uses the initial response.
    Ok(parse_claim_tuples(&initial))
}

async fn render_claim_prompt(
    prompt_loader: &PromptLoader,
    prompt_path: Option<&str>,
    input_text: &str,
    entity_specs: &[String],
    claim_description: &str,
) -> Result<String> {
    prompt_loader
        .render(
            DefaultPrompt::ExtractClaims,
            prompt_path.map(Path::new),
            &ClaimPromptValues {
                input_text,
                entity_specs,
                claim_description,
            },
        )
        .await
        .map_err(GraphLoomError::from)
}

async fn render_builtin_prompt(
    prompt_loader: &PromptLoader,
    prompt: DefaultPrompt,
) -> Result<String> {
    prompt_loader
        .render(prompt, None, &json!({}))
        .await
        .map_err(GraphLoomError::from)
}

pub(crate) fn covariates_dataframe(rows: &[CovariateRow]) -> Result<DataFrame> {
    Ok(df!(
        "id" => rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        "human_readable_id" => rows.iter().map(|row| row.human_readable_id).collect::<Vec<_>>(),
        "covariate_type" => rows.iter().map(|row| row.covariate_type.as_str()).collect::<Vec<_>>(),
        "type" => rows.iter().map(|row| row.claim_type.as_deref()).collect::<Vec<_>>(),
        "description" => rows.iter().map(|row| row.description.as_deref()).collect::<Vec<_>>(),
        "subject_id" => rows.iter().map(|row| row.subject_id.as_deref()).collect::<Vec<_>>(),
        "object_id" => rows.iter().map(|row| row.object_id.as_deref()).collect::<Vec<_>>(),
        "status" => rows.iter().map(|row| row.status.as_deref()).collect::<Vec<_>>(),
        "start_date" => rows.iter().map(|row| row.start_date.as_deref()).collect::<Vec<_>>(),
        "end_date" => rows.iter().map(|row| row.end_date.as_deref()).collect::<Vec<_>>(),
        "source_text" => rows.iter().map(|row| row.source_text.as_deref()).collect::<Vec<_>>(),
        "text_unit_id" => rows.iter().map(|row| row.text_unit_id.as_str()).collect::<Vec<_>>(),
    )?)
}

pub(crate) fn covariate_value(row: &CovariateRow) -> Value {
    json!({
        "id": row.id,
        "human_readable_id": row.human_readable_id,
        "covariate_type": row.covariate_type,
        "type": row.claim_type,
        "description": row.description,
        "subject_id": row.subject_id,
        "object_id": row.object_id,
        "status": row.status,
        "start_date": row.start_date,
        "end_date": row.end_date,
        "source_text": row.source_text,
        "text_unit_id": row.text_unit_id,
    })
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use async_trait::async_trait;
    use graphloom_llm::{CompletionResponse, LlmError, MockCompletionModel, PromptLoader};
    use polars_core::prelude::*;
    use tokio::time::sleep;

    use super::*;

    #[tokio::test]
    async fn test_should_keep_stable_order_with_concurrent_claim_extraction() {
        let model = DelayedContentModel::default();
        let text_units = vec![
            TextUnitInput {
                id: "tu-1".to_owned(),
                text: "tu-1 Alice reports Bob.".to_owned(),
            },
            TextUnitInput {
                id: "tu-2".to_owned(),
                text: "tu-2 Carol reports Dave.".to_owned(),
            },
        ];
        let completed = AtomicUsize::new(0);

        let rows = extract_covariates(
            &model,
            &PromptLoader::new("."),
            &text_units,
            ClaimExtractionConfig {
                prompt_path: None,
                claim_description: "claims",
                entity_types: &default_claim_entity_types(),
                max_gleanings: 0,
                concurrency: 2,
            },
            &|done, _total| {
                completed.store(done, Ordering::SeqCst);
            },
        )
        .await
        .expect("claims should extract");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].human_readable_id, 0);
        assert_eq!(rows[0].text_unit_id, "tu-1");
        assert_eq!(rows[0].subject_id.as_deref(), Some("ALICE"));
        assert_eq!(rows[1].human_readable_id, 1);
        assert_eq!(rows[1].text_unit_id, "tu-2");
        assert_eq!(rows[1].subject_id.as_deref(), Some("CAROL"));
        assert_eq!(completed.load(Ordering::SeqCst), 2);
        assert_eq!(model.max_in_flight(), 2);
        assert_eq!(
            model.completion_order(),
            vec!["tu-2".to_owned(), "tu-1".to_owned()]
        );
    }

    #[tokio::test]
    async fn test_should_parse_initial_claims_only_after_gleaning() {
        let model = MockCompletionModel::new(
            "claims",
            vec![claim("ALICE", "BOB"), claim("CAROL", "DAVE")],
        );
        let text_units = vec![TextUnitInput {
            id: "tu-1".to_owned(),
            text: "Alice reports Bob. Carol reports Dave.".to_owned(),
        }];

        let rows = extract_covariates(
            &model,
            &PromptLoader::new("."),
            &text_units,
            ClaimExtractionConfig {
                prompt_path: None,
                claim_description: "claims",
                entity_types: &default_claim_entity_types(),
                max_gleanings: 1,
                concurrency: 1,
            },
            &|_, _| {},
        )
        .await
        .expect("claims should extract");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].subject_id.as_deref(), Some("ALICE"));
    }

    #[tokio::test]
    async fn test_should_fail_fast_when_any_text_unit_claim_extraction_fails() {
        let model = FailingContentModel;
        let text_units = vec![
            TextUnitInput {
                id: "tu-1".to_owned(),
                text: "tu-1 Alice reports Bob.".to_owned(),
            },
            TextUnitInput {
                id: "tu-2".to_owned(),
                text: "tu-2 should fail.".to_owned(),
            },
        ];

        let error = extract_covariates(
            &model,
            &PromptLoader::new("."),
            &text_units,
            ClaimExtractionConfig {
                prompt_path: None,
                claim_description: "claims",
                entity_types: &default_claim_entity_types(),
                max_gleanings: 0,
                concurrency: 2,
            },
            &|_, _| {},
        )
        .await
        .expect_err("GraphLoom currently fails fast on a single text-unit LLM error");

        // Microsoft GraphRAG currently records per-document claim extraction errors and
        // continues. GraphLoom intentionally preserves its existing fail-fast behavior
        // until error-tolerance semantics are decided separately.
        assert!(error.to_string().contains("tu-2"));
    }

    #[test]
    fn test_should_write_covariate_schema() {
        let rows = vec![CovariateRow {
            id: "claim-1".to_owned(),
            human_readable_id: 0,
            covariate_type: "claim".to_owned(),
            claim_type: Some("REPORT".to_owned()),
            description: Some("Alice reports Bob".to_owned()),
            subject_id: Some("ALICE".to_owned()),
            object_id: Some("BOB".to_owned()),
            status: Some("TRUE".to_owned()),
            start_date: Some("2026-07-07".to_owned()),
            end_date: Some("2026-07-08".to_owned()),
            source_text: Some("Alice reports Bob.".to_owned()),
            text_unit_id: "tu-1".to_owned(),
        }];

        let dataframe = covariates_dataframe(&rows).expect("dataframe should build");

        assert_eq!(
            column_names(&dataframe),
            [
                "id",
                "human_readable_id",
                "covariate_type",
                "type",
                "description",
                "subject_id",
                "object_id",
                "status",
                "start_date",
                "end_date",
                "source_text",
                "text_unit_id",
            ]
        );
        assert_eq!(
            dataframe
                .column("human_readable_id")
                .expect("human_readable_id")
                .dtype(),
            &DataType::Int64
        );
        assert_eq!(
            dataframe
                .column("text_unit_id")
                .expect("text_unit_id")
                .dtype(),
            &DataType::String
        );
    }

    fn claim(subject: &str, object: &str) -> String {
        format!(
            "({subject}<|>{object}<|>REPORT<|>TRUE<|>2026-07-07<|>2026-07-08<|>{subject} reports \
             {object}<|>source)##<|COMPLETE|>"
        )
    }

    #[derive(Debug, Default)]
    struct DelayedContentModel {
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
        completion_order: Mutex<Vec<String>>,
    }

    impl DelayedContentModel {
        fn max_in_flight(&self) -> usize {
            self.max_in_flight.load(Ordering::SeqCst)
        }

        fn completion_order(&self) -> Vec<String> {
            self.completion_order
                .lock()
                .expect("completion order lock should not be poisoned")
                .clone()
        }

        fn observe_in_flight(&self) {
            let current = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            let mut max_seen = self.max_in_flight.load(Ordering::SeqCst);
            while current > max_seen {
                match self.max_in_flight.compare_exchange(
                    max_seen,
                    current,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(actual) => max_seen = actual,
                }
            }
        }
    }

    #[async_trait]
    impl CompletionModel for DelayedContentModel {
        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> graphloom_llm::Result<CompletionResponse> {
            self.observe_in_flight();
            let content = request
                .messages
                .last()
                .map(|message| message.content.as_str())
                .unwrap_or_default();
            let response = if content.contains("tu-1") {
                sleep(Duration::from_millis(80)).await;
                self.completion_order
                    .lock()
                    .map_err(|source| LlmError::InvalidResponse {
                        model_instance: "delayed".to_owned(),
                        operation: "completion",
                        message: source.to_string(),
                    })?
                    .push("tu-1".to_owned());
                claim("ALICE", "BOB")
            } else if content.contains("tu-2") {
                sleep(Duration::from_millis(5)).await;
                self.completion_order
                    .lock()
                    .map_err(|source| LlmError::InvalidResponse {
                        model_instance: "delayed".to_owned(),
                        operation: "completion",
                        message: source.to_string(),
                    })?
                    .push("tu-2".to_owned());
                claim("CAROL", "DAVE")
            } else {
                claim("UNKNOWN", "UNKNOWN")
            };
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(CompletionResponse {
                content: response,
                usage: None,
                request_id: None,
            })
        }
    }

    #[derive(Debug)]
    struct FailingContentModel;

    #[async_trait]
    impl CompletionModel for FailingContentModel {
        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> graphloom_llm::Result<CompletionResponse> {
            let content = request
                .messages
                .last()
                .map(|message| message.content.as_str())
                .unwrap_or_default();
            if content.contains("tu-2") {
                return Err(LlmError::InvalidResponse {
                    model_instance: "failing".to_owned(),
                    operation: "completion",
                    message: "tu-2 failed".to_owned(),
                });
            }
            Ok(CompletionResponse {
                content: claim("ALICE", "BOB"),
                usage: None,
                request_id: None,
            })
        }
    }

    fn column_names(dataframe: &DataFrame) -> Vec<&str> {
        dataframe
            .get_column_names()
            .into_iter()
            .map(|name| name.as_str())
            .collect()
    }
}
