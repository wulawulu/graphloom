//! `GraphRAG` 3.1-compatible Basic Search Sources context.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use graphloom_llm::{EmbeddingModel, EmbeddingRequest, Tokenizer};
use graphloom_vectors::{VectorError, VectorIndexSchema, VectorSearchResult, VectorStore};
use polars_core::prelude::{DataFrame, NamedFrom, Series};

use super::super::{
    QueryContext, QueryContextRecords, QueryContextText, QueryError, QueryUsageCategory, Result,
    SearchMethod, TextUnit, result::resolve_embedding_prompt_tokens,
};
use crate::{
    BasicSearchConfig,
    explainability::{
        BasicRetrievalSkipReason, BasicRetrievalSkipped, CandidatesFiltered, CandidatesRetrieved,
        ContextBudgetAllocated, ContextCompleted, ContextSectionBudget, ContextSectionBuilt,
        ContextSectionKind, EmbeddingCompleted, EmbeddingStarted, ExplainabilityCandidate,
        ExplainabilityContextSection, ExplainabilityEvent, ExplainabilityRecordType,
        ExplainabilityScore, SelectionReason,
    },
    query::explainability::BasicQueryExplainability,
};

#[derive(Debug)]
pub(crate) struct BasicContextBuilder {
    pub(crate) config: BasicSearchConfig,
    pub(crate) text_units: Vec<TextUnit>,
    pub(crate) embedding_model: Arc<dyn EmbeddingModel>,
    pub(crate) embedding_model_id: String,
    pub(crate) vector_store: Arc<dyn VectorStore>,
    pub(crate) vector_schema: VectorIndexSchema,
    pub(crate) tokenizer: Arc<dyn Tokenizer>,
}

#[derive(Debug)]
pub(crate) struct BasicContextBuild {
    pub(crate) context: QueryContext,
    pub(crate) usage: QueryUsageCategory,
}

#[derive(Debug)]
struct AnnCandidateMetadata {
    score: Option<ExplainabilityScore>,
    rank: Option<u32>,
}

#[derive(Debug)]
struct BasicRetrievalBuild {
    matched_ids: BTreeSet<String>,
    usage: QueryUsageCategory,
    metadata_by_id: Option<BTreeMap<String, AnnCandidateMetadata>>,
}

#[derive(Debug)]
struct BasicBudgetBuild<'a> {
    selected: Vec<&'a TextUnit>,
    tokens_used: usize,
    truncated: bool,
}

impl BasicContextBuilder {
    #[cfg(test)]
    async fn build(&self, query: &str) -> Result<BasicContextBuild> {
        self.build_explainable(query, None).await
    }

    pub(crate) async fn build_explainable(
        &self,
        query: &str,
        explainability: Option<&BasicQueryExplainability>,
    ) -> Result<BasicContextBuild> {
        let retrieved = if query.is_empty() {
            if let Some(session) = explainability {
                session
                    .emit(
                        session.spans().retrieval(),
                        Some(session.root_span()),
                        ExplainabilityEvent::BasicRetrievalSkipped(BasicRetrievalSkipped::new(
                            BasicRetrievalSkipReason::EmptyQuery,
                        )),
                    )
                    .await;
            }
            BasicRetrievalBuild {
                matched_ids: BTreeSet::new(),
                usage: QueryUsageCategory::default(),
                metadata_by_id: explainability.is_some().then(BTreeMap::new),
            }
        } else {
            self.retrieve(query, explainability).await?
        };
        let candidates = self
            .text_units
            .iter()
            .filter(|text_unit| retrieved.matched_ids.contains(&text_unit.id))
            .collect::<Vec<_>>();
        self.emit_context_budget(explainability).await;
        let fitted = self.within_budget(&candidates)?;
        self.emit_filtered_candidates(
            explainability,
            &candidates,
            fitted.selected.len(),
            retrieved.metadata_by_id.as_ref(),
        )
        .await;
        let context_text = render_sources(&fitted.selected)?;
        self.emit_context_completed(explainability, &candidates, &fitted, &context_text)
            .await;
        let ids = fitted
            .selected
            .iter()
            .map(|text_unit| text_unit.short_id.as_str())
            .collect::<Vec<_>>();
        let texts = fitted
            .selected
            .iter()
            .map(|text_unit| text_unit.text.as_str())
            .collect::<Vec<_>>();
        let records = DataFrame::new(
            fitted.selected.len(),
            vec![
                Series::new("id".into(), ids).into(),
                Series::new("text".into(), texts).into(),
            ],
        )
        .map_err(|source| QueryError::QueryContext {
            method: SearchMethod::Basic,
            operation: "build Sources records",
            message: source.to_string(),
        })?;
        let context = QueryContext {
            text: QueryContextText::Text(context_text),
            records: QueryContextRecords::Tables(BTreeMap::from([("sources".to_owned(), records)])),
        };
        Ok(BasicContextBuild {
            context,
            usage: retrieved.usage,
        })
    }

    async fn retrieve(
        &self,
        query: &str,
        explainability: Option<&BasicQueryExplainability>,
    ) -> Result<BasicRetrievalBuild> {
        if let Some(session) = explainability {
            let mut event = EmbeddingStarted::new(self.embedding_model_id.clone());
            event.input = session.content(query);
            session
                .emit(
                    session.spans().embedding(),
                    Some(session.root_span()),
                    ExplainabilityEvent::EmbeddingStarted(event),
                )
                .await;
        }
        let response = self
            .embedding_model
            .embed(EmbeddingRequest::new(vec![query.to_owned()]))
            .await
            .map_err(|source| QueryError::QueryEmbedding {
                method: SearchMethod::Basic,
                operation: "embed Basic Search query",
                model: self.embedding_model_id.clone(),
                source: Box::new(source),
            })?;
        let prompt_tokens = resolve_embedding_prompt_tokens(
            response.usage.prompt_tokens,
            query,
            self.tokenizer.as_ref(),
            SearchMethod::Basic,
            "count Basic Search embedding input tokens",
            &self.embedding_model_id,
        )?;
        let vector = response
            .into_embeddings()
            .into_iter()
            .next()
            .ok_or_else(|| QueryError::QueryEmbedding {
                method: SearchMethod::Basic,
                operation: "read Basic Search query embedding",
                model: self.embedding_model_id.clone(),
                source: Box::new(graphloom_llm::LlmError::InvalidResponse {
                    model_instance: self.embedding_model_id.clone(),
                    operation: "embedding conversion",
                    message: "provider returned no query embedding".to_owned(),
                }),
            })?;
        if vector.iter().any(|value| !value.is_finite()) {
            return Err(QueryError::QueryEmbedding {
                method: SearchMethod::Basic,
                operation: "validate Basic Search query embedding",
                model: self.embedding_model_id.clone(),
                source: Box::new(graphloom_llm::LlmError::InvalidResponse {
                    model_instance: self.embedding_model_id.clone(),
                    operation: "embedding conversion",
                    message: "provider returned a non-finite query embedding".to_owned(),
                }),
            });
        }
        if let Some(session) = explainability
            && let (Some(prompt_tokens), Some(dimensions)) = (
                session.usize_to_u64(prompt_tokens),
                session.usize_to_u32(vector.len()),
            )
        {
            session
                .emit(
                    session.spans().embedding(),
                    Some(session.root_span()),
                    ExplainabilityEvent::EmbeddingCompleted(EmbeddingCompleted::new(
                        self.embedding_model_id.clone(),
                        prompt_tokens,
                        dimensions,
                    )),
                )
                .await;
        }
        let results = self
            .vector_store
            .similarity_search_by_vector(&self.vector_schema, &vector, self.config.k, false)
            .await
            .map_err(|source| match source {
                source @ VectorError::MissingIndex { .. } => QueryError::MissingVectorIndex {
                    method: SearchMethod::Basic,
                    operation: "search text_unit_text",
                    index: self.vector_schema.index_name.clone(),
                    source: Box::new(source),
                },
                source => QueryError::InvalidVectorIndex {
                    method: SearchMethod::Basic,
                    operation: "search text_unit_text",
                    index: self.vector_schema.index_name.clone(),
                    source: Box::new(source),
                },
            })?;
        let metadata_by_id = self
            .emit_retrieved_candidates(explainability, &results)
            .await;
        Ok(BasicRetrievalBuild {
            matched_ids: results
                .into_iter()
                .map(|result| result.document.id)
                .collect(),
            usage: QueryUsageCategory {
                llm_calls: 1,
                prompt_tokens,
                output_tokens: 0,
            },
            metadata_by_id,
        })
    }

    fn within_budget<'a>(&self, candidates: &[&'a TextUnit]) -> Result<BasicBudgetBuild<'a>> {
        let mut tokens = self
            .tokenizer
            .count("id|text\n")
            .map_err(|source| context_token_error(&source))?;
        let mut selected = Vec::new();
        for text_unit in candidates {
            let row = format!("{}|{}\n", text_unit.short_id, text_unit.text);
            let row_tokens = self
                .tokenizer
                .count(&row)
                .map_err(|source| context_token_error(&source))?;
            if tokens.saturating_add(row_tokens) > self.config.max_context_tokens {
                break;
            }
            tokens = tokens.saturating_add(row_tokens);
            selected.push(*text_unit);
        }
        Ok(BasicBudgetBuild {
            truncated: selected.len() < candidates.len(),
            selected,
            tokens_used: tokens,
        })
    }

    async fn emit_retrieved_candidates(
        &self,
        explainability: Option<&BasicQueryExplainability>,
        results: &[VectorSearchResult],
    ) -> Option<BTreeMap<String, AnnCandidateMetadata>> {
        let session = explainability?;
        let short_ids = self
            .text_units
            .iter()
            .map(|text_unit| (text_unit.id.as_str(), text_unit.short_id.as_str()))
            .collect::<BTreeMap<_, _>>();
        let mut candidates = Vec::with_capacity(results.len());
        let mut metadata_by_id = BTreeMap::new();
        for (index, result) in results.iter().enumerate() {
            let rank = session.usize_to_u32(index.saturating_add(1));
            let score = ExplainabilityScore::try_from(f64::from(result.score)).ok();
            if score.is_none() {
                session.mark_sidecar_failure("non_finite_ann_score");
            }
            let mut candidate = ExplainabilityCandidate::new(
                result.document.id.clone(),
                ExplainabilityRecordType::TextUnit,
            );
            candidate.short_id = short_ids
                .get(result.document.id.as_str())
                .map(|value| (*value).to_owned());
            candidate.score = score;
            candidate.rank = rank;
            candidate.reason = Some(SelectionReason::AnnResult);
            candidates.push(candidate);
            metadata_by_id
                .entry(result.document.id.clone())
                .or_insert(AnnCandidateMetadata { score, rank });
        }
        session
            .emit_contract(
                session.spans().retrieval(),
                Some(session.root_span()),
                CandidatesRetrieved::try_new(ExplainabilityRecordType::TextUnit, candidates)
                    .map(ExplainabilityEvent::CandidatesRetrieved),
            )
            .await;
        Some(metadata_by_id)
    }

    async fn emit_context_budget(&self, explainability: Option<&BasicQueryExplainability>) {
        let Some(session) = explainability else {
            return;
        };
        let Some(token_budget) = session.usize_to_u64(self.config.max_context_tokens) else {
            return;
        };
        session
            .emit(
                session.spans().context(),
                Some(session.root_span()),
                ExplainabilityEvent::ContextBudgetAllocated(ContextBudgetAllocated::new(
                    token_budget,
                    vec![ContextSectionBudget::new(
                        ContextSectionKind::Sources,
                        token_budget,
                    )],
                )),
            )
            .await;
    }

    async fn emit_filtered_candidates(
        &self,
        explainability: Option<&BasicQueryExplainability>,
        candidates: &[&TextUnit],
        selected_count: usize,
        metadata_by_id: Option<&BTreeMap<String, AnnCandidateMetadata>>,
    ) {
        let (Some(session), Some(metadata_by_id)) = (explainability, metadata_by_id) else {
            return;
        };
        let decisions = candidates
            .iter()
            .enumerate()
            .map(|(index, text_unit)| {
                let mut candidate = ExplainabilityCandidate::new(
                    text_unit.id.clone(),
                    ExplainabilityRecordType::TextUnit,
                );
                candidate.short_id = Some(text_unit.short_id.clone());
                if let Some(metadata) = metadata_by_id.get(&text_unit.id) {
                    candidate.score = metadata.score;
                    candidate.rank = metadata.rank;
                }
                candidate.selected = index < selected_count;
                candidate.reason = Some(if candidate.selected {
                    SelectionReason::AnnResult
                } else {
                    SelectionReason::TokenBudget
                });
                candidate
            })
            .collect();
        session
            .emit_contract(
                session.spans().retrieval(),
                Some(session.root_span()),
                CandidatesFiltered::try_new(ExplainabilityRecordType::TextUnit, decisions)
                    .map(ExplainabilityEvent::CandidatesFiltered),
            )
            .await;
    }

    async fn emit_context_completed(
        &self,
        explainability: Option<&BasicQueryExplainability>,
        candidates: &[&TextUnit],
        fitted: &BasicBudgetBuild<'_>,
        context_text: &str,
    ) {
        let Some(session) = explainability else {
            return;
        };
        let values = (
            session.usize_to_u64(self.config.max_context_tokens),
            session.usize_to_u64(fitted.tokens_used),
            session.usize_to_u64(candidates.len()),
            session.usize_to_u64(fitted.selected.len()),
        );
        if let (
            Some(token_budget),
            Some(tokens_used),
            Some(candidate_count),
            Some(selected_count),
        ) = values
        {
            let mut section =
                ExplainabilityContextSection::new(ContextSectionKind::Sources, token_budget);
            section.tokens_used = tokens_used;
            section.candidate_count = candidate_count;
            section.selected_count = selected_count;
            section.truncated = fitted.truncated;
            section.selected_record_ids = fitted
                .selected
                .iter()
                .map(|text_unit| text_unit.id.clone())
                .collect();
            session
                .emit(
                    session.spans().context(),
                    Some(session.root_span()),
                    ExplainabilityEvent::ContextSectionBuilt(ContextSectionBuilt::new(section)),
                )
                .await;
            let mut event = ContextCompleted::new(tokens_used);
            event.context = session.content(context_text);
            session
                .emit(
                    session.spans().context(),
                    Some(session.root_span()),
                    ExplainabilityEvent::ContextCompleted(event),
                )
                .await;
        }
    }
}

fn render_sources(text_units: &[&TextUnit]) -> Result<String> {
    let mut writer = csv::WriterBuilder::new()
        .delimiter(b'|')
        .escape(b'\\')
        .from_writer(Vec::new());
    writer
        .write_record(["id", "text"])
        .map_err(|source| csv_context_error(&source))?;
    for text_unit in text_units {
        let id = pandas_escape_field(&text_unit.short_id);
        let text = pandas_escape_field(&text_unit.text);
        writer
            .write_record([id.as_ref(), text.as_ref()])
            .map_err(|source| csv_context_error(&source))?;
    }
    let bytes = writer
        .into_inner()
        .map_err(|source| csv_context_error(&source.into_error().into()))?;
    String::from_utf8(bytes).map_err(|source| QueryError::QueryContext {
        method: SearchMethod::Basic,
        operation: "encode Sources CSV",
        message: source.to_string(),
    })
}

fn pandas_escape_field(value: &str) -> std::borrow::Cow<'_, str> {
    if value.contains('\\') {
        std::borrow::Cow::Owned(value.replace('\\', "\\\\"))
    } else {
        std::borrow::Cow::Borrowed(value)
    }
}

fn csv_context_error(source: &csv::Error) -> QueryError {
    QueryError::QueryContext {
        method: SearchMethod::Basic,
        operation: "render Sources CSV",
        message: source.to_string(),
    }
}

fn context_token_error(source: &graphloom_llm::LlmError) -> QueryError {
    QueryError::QueryContext {
        method: SearchMethod::Basic,
        operation: "count Sources tokens",
        message: source.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use graphloom_llm::{EmbeddingResponse, LlmError};
    use graphloom_vectors::{VectorDocument, VectorSearchResult};

    use super::*;
    use crate::{
        explainability::{
            ExplainabilityContentMode, ExplainabilityRecord, ExplainabilityRunId,
            ExplainabilitySink, ExplainabilitySinkError,
        },
        query::QueryExplainabilityOptions,
    };

    #[derive(Debug, Default)]
    struct RecordingSink {
        records: Mutex<Vec<Arc<ExplainabilityRecord>>>,
    }

    #[async_trait]
    impl ExplainabilitySink for RecordingSink {
        async fn emit(
            &self,
            record: Arc<ExplainabilityRecord>,
        ) -> std::result::Result<(), ExplainabilitySinkError> {
            self.records.lock().expect("records").push(record);
            Ok(())
        }

        async fn finish_run(
            &self,
            _run_id: &ExplainabilityRunId,
        ) -> std::result::Result<(), ExplainabilitySinkError> {
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct ByteTokenizer;

    impl Tokenizer for ByteTokenizer {
        fn encode(&self, text: &str) -> graphloom_llm::Result<Vec<u32>> {
            Ok(text
                .as_bytes()
                .iter()
                .map(|value| u32::from(*value))
                .collect())
        }

        fn decode(&self, tokens: &[u32]) -> graphloom_llm::Result<String> {
            let bytes = tokens
                .iter()
                .map(|value| u8::try_from(*value))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|source| LlmError::Tokenizer {
                    encoding_model: "bytes".to_owned(),
                    message: source.to_string(),
                })?;
            String::from_utf8(bytes).map_err(|source| LlmError::Tokenizer {
                encoding_model: "bytes".to_owned(),
                message: source.to_string(),
            })
        }
    }

    #[derive(Debug)]
    struct RecordingEmbedding {
        calls: Arc<AtomicUsize>,
        inputs: Arc<Mutex<Vec<Vec<String>>>>,
    }

    #[async_trait]
    impl EmbeddingModel for RecordingEmbedding {
        async fn embed(
            &self,
            request: EmbeddingRequest,
        ) -> graphloom_llm::Result<EmbeddingResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inputs
                .lock()
                .expect("recording mutex")
                .push(request.input);
            Ok(EmbeddingResponse::vectors_for_test(
                "embedding",
                vec![vec![0.25, 0.75]],
            ))
        }
    }

    #[derive(Debug)]
    struct RecordingVectorStore {
        results: Vec<VectorSearchResult>,
        calls: Arc<AtomicUsize>,
        queries: RecordedQueries,
    }

    type VectorQuery = (Vec<f32>, usize, bool);
    type RecordedQueries = Arc<Mutex<Vec<VectorQuery>>>;
    type BuilderFixture = (
        BasicContextBuilder,
        Arc<AtomicUsize>,
        Arc<Mutex<Vec<Vec<String>>>>,
        Arc<AtomicUsize>,
        RecordedQueries,
    );

    #[async_trait]
    impl VectorStore for RecordingVectorStore {
        async fn ensure_index(&self, _schema: &VectorIndexSchema) -> graphloom_vectors::Result<()> {
            Ok(())
        }

        async fn reset_index(&self, _schema: &VectorIndexSchema) -> graphloom_vectors::Result<()> {
            Ok(())
        }

        async fn append_documents(
            &self,
            _schema: &VectorIndexSchema,
            _documents: &[VectorDocument],
        ) -> graphloom_vectors::Result<()> {
            Ok(())
        }

        async fn upsert_documents(
            &self,
            _schema: &VectorIndexSchema,
            _documents: &[VectorDocument],
        ) -> graphloom_vectors::Result<()> {
            Ok(())
        }

        async fn count(&self, _schema: &VectorIndexSchema) -> graphloom_vectors::Result<usize> {
            Ok(self.results.len())
        }

        async fn ids(&self, _schema: &VectorIndexSchema) -> graphloom_vectors::Result<Vec<String>> {
            Ok(self
                .results
                .iter()
                .map(|result| result.document.id.clone())
                .collect())
        }

        async fn get_by_id(
            &self,
            _schema: &VectorIndexSchema,
            id: &str,
        ) -> graphloom_vectors::Result<Option<VectorDocument>> {
            Ok(self
                .results
                .iter()
                .find(|result| result.document.id == id)
                .map(|result| result.document.clone()))
        }

        async fn similarity_search_by_vector(
            &self,
            _schema: &VectorIndexSchema,
            query_vector: &[f32],
            k: usize,
            include_vectors: bool,
        ) -> graphloom_vectors::Result<Vec<VectorSearchResult>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.queries.lock().expect("recording mutex").push((
                query_vector.to_vec(),
                k,
                include_vectors,
            ));
            Ok(self.results.clone())
        }
    }

    fn text_unit(id: &str, short_id: &str, text: &str) -> TextUnit {
        TextUnit {
            id: id.to_owned(),
            short_id: short_id.to_owned(),
            text: text.to_owned(),
            entity_ids: Vec::new(),
            relationship_ids: Vec::new(),
            covariate_ids: Vec::new(),
            n_tokens: None,
            document_id: None,
        }
    }

    fn result(id: &str, score: f32) -> VectorSearchResult {
        VectorSearchResult {
            document: VectorDocument {
                id: id.to_owned(),
                vector: Vec::new(),
            },
            score,
        }
    }

    fn builder(
        text_units: Vec<TextUnit>,
        results: Vec<VectorSearchResult>,
        max_context_tokens: usize,
    ) -> BuilderFixture {
        let embedding_calls = Arc::new(AtomicUsize::new(0));
        let embedding_inputs = Arc::new(Mutex::new(Vec::new()));
        let vector_calls = Arc::new(AtomicUsize::new(0));
        let vector_queries = Arc::new(Mutex::new(Vec::new()));
        let config = BasicSearchConfig {
            k: 2,
            max_context_tokens,
            ..BasicSearchConfig::default()
        };
        (
            BasicContextBuilder {
                config,
                text_units,
                embedding_model: Arc::new(RecordingEmbedding {
                    calls: Arc::clone(&embedding_calls),
                    inputs: Arc::clone(&embedding_inputs),
                }),
                embedding_model_id: "embedding".to_owned(),
                vector_store: Arc::new(RecordingVectorStore {
                    results,
                    calls: Arc::clone(&vector_calls),
                    queries: Arc::clone(&vector_queries),
                }),
                vector_schema: VectorIndexSchema::for_embedding_name("text_unit_text", 2),
                tokenizer: Arc::new(ByteTokenizer),
            },
            embedding_calls,
            embedding_inputs,
            vector_calls,
            vector_queries,
        )
    }

    fn context_text(build: &BasicContextBuild) -> &str {
        match &build.context.text {
            QueryContextText::Text(value) => value,
            _ => panic!("expected Basic text context"),
        }
    }

    #[tokio::test]
    async fn test_should_use_ann_as_id_set_and_preserve_text_unit_table_order() {
        let (builder, _, embedding_inputs, _, vector_queries) = builder(
            vec![text_unit("A", "0", "first"), text_unit("B", "1", "second")],
            vec![result("B", 0.9), result("A", 0.8)],
            usize::MAX,
        );

        let built = builder.build("question").await.expect("context");

        assert_eq!(context_text(&built), "id|text\n0|first\n1|second\n");
        assert_eq!(
            built.usage,
            QueryUsageCategory {
                llm_calls: 1,
                prompt_tokens: "question".len(),
                output_tokens: 0,
            }
        );
        assert_eq!(
            embedding_inputs.lock().expect("inputs").as_slice(),
            &[vec!["question".to_owned()]]
        );
        assert_eq!(
            vector_queries.lock().expect("queries").as_slice(),
            &[(vec![0.25, 0.75], 2, false)]
        );
    }

    #[tokio::test]
    async fn test_should_fallback_to_tokenizer_for_zero_basic_embedding_usage() {
        let (builder, _, _, _, _) = builder(
            vec![text_unit("A", "0", "source")],
            vec![result("A", 1.0)],
            usize::MAX,
        );

        let built = builder.build("zero usage").await.expect("context");

        assert_eq!(
            built.usage,
            QueryUsageCategory {
                llm_calls: 1,
                prompt_tokens: "zero usage".len(),
                output_tokens: 0,
            }
        );
    }

    #[tokio::test]
    async fn test_should_match_pandas_sources_csv_escaping_golden() {
        let texts = [
            "pipe|value",
            "quote \"value\"",
            "back\\slash",
            "line1\nline2",
            "",
            "Unicode 世界",
        ];
        let units = texts
            .iter()
            .enumerate()
            .map(|(index, text)| text_unit(&format!("id-{index}"), &index.to_string(), text))
            .collect::<Vec<_>>();
        let results = (0..texts.len())
            .rev()
            .map(|index| result(&format!("id-{index}"), 1.0))
            .collect::<Vec<_>>();
        let (builder, _, _, _, _) = builder(units, results, usize::MAX);

        let built = builder.build("question").await.expect("context");

        assert_eq!(
            context_text(&built),
            "id|text\n0|\"pipe|value\"\n1|\"quote \
             \"\"value\"\"\"\n2|back\\\\slash\n3|\"line1\nline2\"\n4|\n5|Unicode 世界\n"
        );
    }

    #[tokio::test]
    async fn test_should_count_header_and_keep_only_whole_rows_within_budget() {
        let (builder, _, _, _, _) = builder(
            vec![text_unit("A", "0", "A"), text_unit("B", "1", "BBBB")],
            vec![result("A", 1.0), result("B", 0.9)],
            "id|text\n0|A\n".len(),
        );

        let built = builder.build("question").await.expect("context");

        assert_eq!(context_text(&built), "id|text\n0|A\n");
    }

    #[tokio::test]
    async fn test_should_skip_retrieval_only_for_exactly_empty_query() {
        let (builder, embedding_calls, _, vector_calls, _) = builder(
            vec![text_unit("A", "0", "A")],
            vec![result("A", 1.0)],
            usize::MAX,
        );

        let empty = builder.build("").await.expect("empty context");
        assert_eq!(context_text(&empty), "id|text\n");
        assert_eq!(empty.usage, QueryUsageCategory::default());
        assert_eq!(embedding_calls.load(Ordering::SeqCst), 0);
        assert_eq!(vector_calls.load(Ordering::SeqCst), 0);

        builder.build("   ").await.expect("whitespace query");
        assert_eq!(embedding_calls.load(Ordering::SeqCst), 1);
        assert_eq!(vector_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_should_capture_ann_order_and_real_budget_stop_without_replaying_selection() {
        let (builder, _, _, _, _) = builder(
            vec![
                text_unit("A", "0", "A"),
                text_unit("B", "1", "BBBBBBBBBBBB"),
                text_unit("C", "2", "C"),
            ],
            vec![result("C", 0.95), result("A", 0.90), result("B", 0.80)],
            "id|text\n0|A\n2|C\n".len(),
        );
        let sink = Arc::new(RecordingSink::default());
        let options = QueryExplainabilityOptions::new(
            "basic-context-test".parse().expect("run id"),
            ExplainabilityContentMode::Content,
            sink.clone(),
        );
        let explainability = BasicQueryExplainability::new(&options);

        let built = builder
            .build_explainable("question", Some(&explainability))
            .await
            .expect("context");

        assert_eq!(context_text(&built), "id|text\n0|A\n");
        let records = sink.records.lock().expect("records");
        let retrieved = records
            .iter()
            .find_map(|record| match &record.event {
                ExplainabilityEvent::CandidatesRetrieved(event) => Some(event),
                _ => None,
            })
            .expect("retrieved evidence");
        assert_eq!(
            retrieved
                .candidates()
                .iter()
                .map(|candidate| candidate.id.as_str())
                .collect::<Vec<_>>(),
            ["C", "A", "B"]
        );
        let filtered = records
            .iter()
            .find_map(|record| match &record.event {
                ExplainabilityEvent::CandidatesFiltered(event) => Some(event),
                _ => None,
            })
            .expect("filtered evidence");
        assert_eq!(
            filtered
                .candidates()
                .iter()
                .map(|candidate| (
                    candidate.id.as_str(),
                    candidate.selected,
                    candidate.reason,
                    candidate.rank,
                ))
                .collect::<Vec<_>>(),
            [
                ("A", true, Some(SelectionReason::AnnResult), Some(2)),
                ("B", false, Some(SelectionReason::TokenBudget), Some(3)),
                ("C", false, Some(SelectionReason::TokenBudget), Some(1)),
            ]
        );
        let section = records
            .iter()
            .find_map(|record| match &record.event {
                ExplainabilityEvent::ContextSectionBuilt(event) => Some(&event.section),
                _ => None,
            })
            .expect("Sources section");
        assert_eq!(section.selected_record_ids, ["A"]);
        assert_eq!(
            section.tokens_used,
            u64::try_from("id|text\n0|A\n".len()).expect("tokens")
        );
        assert!(section.truncated);
        assert!(records.iter().any(|record| matches!(
            &record.event,
            ExplainabilityEvent::ContextCompleted(event)
                if event.context.as_deref() == Some("id|text\n0|A\n")
                    && event.tokens_used == section.tokens_used
        )));
    }
}
