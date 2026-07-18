//! DRIFT report hydration, HyDE expansion, and in-memory cosine ranking.

use std::sync::Arc;

use graphloom_llm::{
    ChatMessage, CompletionModel, CompletionRequest, EmbeddingModel, EmbeddingRequest, ModelConfig,
    Tokenizer,
};
use graphloom_vectors::{VectorError, VectorIndexSchema, VectorStore};
use rand::{Rng, seq::SliceRandom};

use crate::{
    DriftSearchConfig,
    query::{
        CommunityReport, QueryError, QueryUsageCategory, Result, SearchMethod,
        local::LocalContextBuilder,
    },
};

/// Random choices required by GraphRAG DRIFT.
pub(super) trait DriftRandom: Send {
    fn choose_report(&mut self, len: usize) -> usize;
    fn shuffle_actions(&mut self, actions: &mut [usize]);
}

#[derive(Debug, Default)]
pub(super) struct SystemDriftRandom;

impl DriftRandom for SystemDriftRandom {
    fn choose_report(&mut self, len: usize) -> usize {
        rand::thread_rng().gen_range(0..len)
    }

    fn shuffle_actions(&mut self, actions: &mut [usize]) {
        actions.shuffle(&mut rand::thread_rng());
    }
}

#[derive(Debug, Clone)]
pub(super) struct RankedReport {
    pub(super) short_id: String,
    pub(super) community_id: String,
    pub(super) full_content: String,
    pub(super) similarity: f64,
}

/// Shared context resources for all DRIFT stages.
#[derive(Debug)]
pub(crate) struct DriftContextBuilder {
    pub(crate) config: DriftSearchConfig,
    pub(crate) reports: Vec<CommunityReport>,
    pub(crate) local: LocalContextBuilder,
    pub(crate) completion_model: Arc<dyn CompletionModel>,
    pub(crate) embedding_model: Arc<dyn EmbeddingModel>,
    pub(crate) completion_model_id: String,
    pub(crate) embedding_model_id: String,
    pub(crate) completion_config: ModelConfig,
    pub(crate) vector_store: Arc<dyn VectorStore>,
    pub(crate) community_schema: VectorIndexSchema,
    pub(crate) tokenizer: Arc<dyn Tokenizer>,
}

impl DriftContextBuilder {
    pub(crate) async fn hydrate_reports(&mut self) -> Result<()> {
        hydrate_reports(
            self.vector_store.as_ref(),
            &self.community_schema,
            &mut self.reports,
        )
        .await
    }
}

async fn hydrate_reports(
    store: &dyn VectorStore,
    schema: &VectorIndexSchema,
    reports: &mut [CommunityReport],
) -> Result<()> {
    let mut missing = Vec::new();
    for report in &mut *reports {
        match store.get_by_id(schema, &report.id).await {
            Ok(Some(document)) => {
                validate_document_vector(&document.vector, &report.id, &schema.index_name)?;
                report.full_content_embedding = Some(document.vector);
            }
            Ok(None) => missing.push(report.id.clone()),
            Err(source @ VectorError::MissingIndex { .. }) => {
                return Err(QueryError::MissingVectorIndex {
                    method: SearchMethod::Drift,
                    operation: "hydrate DRIFT community report embeddings",
                    index: schema.index_name.clone(),
                    source: Box::new(source),
                });
            }
            Err(source) => {
                return Err(QueryError::InvalidVectorIndex {
                    method: SearchMethod::Drift,
                    operation: "hydrate DRIFT community report embeddings",
                    index: schema.index_name.clone(),
                    source: Box::new(source),
                });
            }
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    let sample = missing
        .iter()
        .take(10)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    Err(QueryError::InvalidVectorIndex {
        method: SearchMethod::Drift,
        operation: "hydrate DRIFT community report embeddings",
        index: schema.index_name.clone(),
        source: Box::new(VectorError::InvalidDocument {
            index_name: schema.index_name.clone(),
            message: format!(
                "missing {} of {} report vectors; report ids: {sample}",
                missing.len(),
                reports.len()
            ),
        }),
    })
}

impl DriftContextBuilder {
    pub(super) async fn build_ranked_context(
        &self,
        query: &str,
        random: &mut dyn DriftRandom,
    ) -> Result<(Vec<RankedReport>, QueryUsageCategory)> {
        if self.reports.is_empty() {
            return Err(QueryError::QueryContext {
                method: SearchMethod::Drift,
                operation: "select DRIFT HyDE template",
                message: "no community reports are available".to_owned(),
            });
        }
        let chosen = random.choose_report(self.reports.len());
        let template = self
            .reports
            .get(chosen)
            .ok_or_else(|| QueryError::QueryContext {
                method: SearchMethod::Drift,
                operation: "select DRIFT HyDE template",
                message: format!(
                    "random report index {chosen} is outside {}",
                    self.reports.len()
                ),
            })?;
        let prompt = hyde_prompt(query, &template.full_content);
        let prompt_tokens = count(&*self.tokenizer, &prompt, "count DRIFT HyDE prompt")?;
        let mut request = CompletionRequest::new(vec![ChatMessage::user(prompt)]);
        request
            .apply_call_args(&self.completion_config.call_args)
            .and_then(|()| {
                request.stream = Some(false);
                request.response_format = None;
                request.validate()
            })
            .map_err(|source| QueryError::InvalidQueryConfig {
                method: SearchMethod::Drift,
                operation: "build DRIFT HyDE completion request",
                message: source.to_string(),
            })?;
        let response = self
            .completion_model
            .complete(request)
            .await
            .map_err(|source| QueryError::QueryCompletion {
                method: SearchMethod::Drift,
                operation: "complete DRIFT HyDE expansion",
                model: self.completion_model_id.clone(),
                source: Box::new(source),
            })?;
        let expanded = response
            .first_choice()
            .and_then(|choice| choice.message.content.as_deref())
            .ok_or_else(|| QueryError::QueryCompletion {
                method: SearchMethod::Drift,
                operation: "read DRIFT HyDE expansion",
                model: self.completion_model_id.clone(),
                source: Box::new(graphloom_llm::LlmError::InvalidResponse {
                    model_instance: self.completion_model_id.clone(),
                    operation: "completion",
                    message: "missing choices[0].message.content".to_owned(),
                }),
            })?;
        let output_tokens = count(
            &*self.tokenizer,
            expanded,
            "count DRIFT HyDE expansion output",
        )?;
        let expanded_query = if expanded.is_empty() {
            tracing::warn!(method = %SearchMethod::Drift, "DRIFT HyDE expansion was empty");
            query
        } else {
            expanded
        };
        let embedding = self
            .embedding_model
            .embed(EmbeddingRequest::new(vec![expanded_query.to_owned()]))
            .await
            .map_err(|source| QueryError::QueryEmbedding {
                method: SearchMethod::Drift,
                operation: "embed DRIFT expanded query",
                model: self.embedding_model_id.clone(),
                source: Box::new(source),
            })?
            .into_embeddings()
            .into_iter()
            .next()
            .ok_or_else(|| QueryError::QueryEmbedding {
                method: SearchMethod::Drift,
                operation: "read DRIFT expanded query embedding",
                model: self.embedding_model_id.clone(),
                source: Box::new(graphloom_llm::LlmError::InvalidResponse {
                    model_instance: self.embedding_model_id.clone(),
                    operation: "embedding conversion",
                    message: "provider returned no embedding".to_owned(),
                }),
            })?;
        let ranked = rank_reports(
            &embedding,
            &self.reports,
            self.config.drift_k_followups,
            &self.community_schema.index_name,
        )?;
        Ok((
            ranked,
            QueryUsageCategory {
                llm_calls: 1,
                prompt_tokens,
                output_tokens,
            },
        ))
    }
}

/// Exact GraphRAG 3.1.0 `PrimerQueryProcessor.expand_query` prompt.
pub(super) fn hyde_prompt(query: &str, template: &str) -> String {
    format!(
        "Create a hypothetical answer to the following query: {query}\n\n\n                  \
         Format it to follow the structure of the template below:\n\n\n                  \
         {template}\n\"\n                  Ensure that the hypothetical answer does not reference \
         new named entities that are not present in the original query."
    )
}

fn rank_reports(
    query: &[f32],
    reports: &[CommunityReport],
    top_k: usize,
    index: &str,
) -> Result<Vec<RankedReport>> {
    validate_query_vector(query, index)?;
    let query_norm = norm(query);
    let mut ranked = reports
        .iter()
        .enumerate()
        .map(|(order, report)| {
            let vector = report.full_content_embedding.as_deref().ok_or_else(|| {
                invalid_vector(
                    index,
                    format!("report {} has no hydrated vector", report.id),
                )
            })?;
            validate_document_vector(vector, &report.id, index)?;
            if vector.len() != query.len() {
                return Err(invalid_vector(
                    index,
                    format!(
                        "query dimension {} does not match report {} dimension {}",
                        query.len(),
                        report.id,
                        vector.len()
                    ),
                ));
            }
            let similarity = dot(query, vector) / (query_norm * norm(vector));
            Ok((
                order,
                RankedReport {
                    short_id: report.short_id.clone(),
                    community_id: report.community_id.clone(),
                    full_content: report.full_content.clone(),
                    similarity,
                },
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    ranked.sort_by(|left, right| {
        right
            .1
            .similarity
            .total_cmp(&left.1.similarity)
            .then_with(|| left.0.cmp(&right.0))
    });
    ranked.truncate(top_k);
    Ok(ranked.into_iter().map(|(_, report)| report).collect())
}

fn validate_query_vector(vector: &[f32], index: &str) -> Result<()> {
    if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) || norm(vector) == 0.0 {
        return Err(invalid_vector(
            index,
            format!(
                "expanded query vector has invalid dimension {} or zero/non-finite norm",
                vector.len()
            ),
        ));
    }
    Ok(())
}

fn validate_document_vector(vector: &[f32], report_id: &str, index: &str) -> Result<()> {
    if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) || norm(vector) == 0.0 {
        return Err(invalid_vector(
            index,
            format!(
                "report {report_id} vector has invalid dimension {} or zero/non-finite norm",
                vector.len()
            ),
        ));
    }
    Ok(())
}

fn invalid_vector(index: &str, message: String) -> QueryError {
    QueryError::InvalidVectorIndex {
        method: SearchMethod::Drift,
        operation: "rank DRIFT community reports",
        index: index.to_owned(),
        source: Box::new(VectorError::InvalidDocument {
            index_name: index.to_owned(),
            message,
        }),
    }
}

fn norm(vector: &[f32]) -> f64 {
    vector
        .iter()
        .map(|value| f64::from(*value).powi(2))
        .sum::<f64>()
        .sqrt()
}

fn dot(left: &[f32], right: &[f32]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(left, right)| f64::from(*left) * f64::from(*right))
        .sum()
}

pub(super) fn count(
    tokenizer: &dyn Tokenizer,
    text: &str,
    operation: &'static str,
) -> Result<usize> {
    tokenizer
        .count(text)
        .map_err(|source| QueryError::QueryContext {
            method: SearchMethod::Drift,
            operation,
            message: source.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use async_trait::async_trait;
    use graphloom_vectors::{VectorDocument, VectorSearchResult};

    use super::*;

    #[derive(Debug)]
    struct ReadOnlyStore {
        documents: BTreeMap<String, Vec<f32>>,
        fail: bool,
    }

    #[async_trait]
    impl VectorStore for ReadOnlyStore {
        async fn ensure_index(&self, _: &VectorIndexSchema) -> graphloom_vectors::Result<()> {
            panic!("hydration must be read-only")
        }

        async fn reset_index(&self, _: &VectorIndexSchema) -> graphloom_vectors::Result<()> {
            panic!("hydration must be read-only")
        }

        async fn upsert_documents(
            &self,
            _: &VectorIndexSchema,
            _: &[VectorDocument],
        ) -> graphloom_vectors::Result<()> {
            panic!("hydration must be read-only")
        }

        async fn count(&self, _: &VectorIndexSchema) -> graphloom_vectors::Result<usize> {
            Ok(self.documents.len())
        }

        async fn ids(&self, _: &VectorIndexSchema) -> graphloom_vectors::Result<Vec<String>> {
            Ok(self.documents.keys().cloned().collect())
        }

        async fn get_by_id(
            &self,
            schema: &VectorIndexSchema,
            id: &str,
        ) -> graphloom_vectors::Result<Option<VectorDocument>> {
            if self.fail {
                return Err(VectorError::InvalidDocument {
                    index_name: schema.index_name.clone(),
                    message: "broken table".to_owned(),
                });
            }
            Ok(self.documents.get(id).map(|vector| VectorDocument {
                id: id.to_owned(),
                vector: vector.clone(),
            }))
        }

        async fn similarity_search_by_vector(
            &self,
            _: &VectorIndexSchema,
            _: &[f32],
            _: usize,
            _: bool,
        ) -> graphloom_vectors::Result<Vec<VectorSearchResult>> {
            panic!("report ranking must not use ANN")
        }
    }

    fn report(id: &str, vector: Vec<f32>) -> CommunityReport {
        CommunityReport {
            id: id.to_owned(),
            short_id: id.to_owned(),
            community_id: id.to_owned(),
            title: id.to_owned(),
            summary: String::new(),
            full_content: format!("report {id}"),
            rank: Some(1.0),
            full_content_embedding: Some(vector),
        }
    }

    #[test]
    fn test_should_preserve_exact_hyde_prompt_bytes() {
        assert_eq!(
            hyde_prompt("What changed?", "# Template\nFacts."),
            include_str!("../../../../../tests/compat/fixtures/query/drift_hyde_prompt.txt")
                .strip_suffix('\n')
                .expect("fixture has one text-file terminator")
        );
    }

    #[test]
    fn test_should_rank_cosine_descending_and_keep_first_on_ties() {
        let ranked = rank_reports(
            &[1.0, 0.0],
            &[
                report("first", vec![1.0, 0.0]),
                report("second", vec![1.0, 0.0]),
                report("third", vec![0.0, 1.0]),
            ],
            2,
            "community_full_content",
        )
        .expect("ranking");

        assert_eq!(
            ranked
                .iter()
                .map(|report| report.short_id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
    }

    #[test]
    fn test_should_reject_zero_nonfinite_and_dimension_mismatched_vectors() {
        for (query, reports) in [
            (vec![0.0, 0.0], vec![report("report", vec![1.0, 0.0])]),
            (vec![1.0, 0.0], vec![report("report", vec![f32::NAN, 0.0])]),
            (vec![1.0, 0.0], vec![report("report", vec![1.0])]),
        ] {
            assert!(matches!(
                rank_reports(&query, &reports, 1, "community_full_content"),
                Err(QueryError::InvalidVectorIndex {
                    method: SearchMethod::Drift,
                    ..
                })
            ));
        }
    }

    #[tokio::test]
    async fn test_should_hydrate_by_report_id_without_writes() {
        let store = ReadOnlyStore {
            documents: BTreeMap::from([("report-id".to_owned(), vec![1.0, 2.0])]),
            fail: false,
        };
        let schema = VectorIndexSchema::for_embedding_name("community_full_content", 2);
        let mut reports = vec![report("report-id", vec![9.0, 9.0])];
        reports[0].full_content_embedding = None;

        hydrate_reports(&store, &schema, &mut reports)
            .await
            .expect("hydrate");

        assert_eq!(reports[0].full_content_embedding, Some(vec![1.0, 2.0]));
    }

    #[tokio::test]
    async fn test_should_aggregate_missing_report_vectors() {
        let store = ReadOnlyStore {
            documents: BTreeMap::new(),
            fail: false,
        };
        let schema = VectorIndexSchema::for_embedding_name("community_full_content", 2);
        let mut reports = vec![
            report("missing-a", vec![1.0, 0.0]),
            report("missing-b", vec![0.0, 1.0]),
        ];

        let error = hydrate_reports(&store, &schema, &mut reports)
            .await
            .expect_err("missing vectors");
        let message = error.to_string();

        assert!(message.contains("missing 2 of 2"));
        assert!(message.contains("missing-a"));
        assert!(message.contains("missing-b"));
    }

    #[tokio::test]
    async fn test_should_propagate_vector_store_errors_as_invalid_index() {
        let store = ReadOnlyStore {
            documents: BTreeMap::new(),
            fail: true,
        };
        let schema = VectorIndexSchema::for_embedding_name("community_full_content", 2);
        let mut reports = vec![report("report-id", vec![1.0, 0.0])];

        assert!(matches!(
            hydrate_reports(&store, &schema, &mut reports).await,
            Err(QueryError::InvalidVectorIndex {
                method: SearchMethod::Drift,
                ..
            })
        ));
    }
}
