//! Local Search entity mapping and mixed-context construction.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::Arc,
};

use graphloom_llm::{EmbeddingModel, EmbeddingRequest, Tokenizer};
use graphloom_vectors::{VectorError, VectorIndexSchema, VectorSearchResult, VectorStore};
use polars_core::prelude::{DataFrame, NamedFrom, Series};
use tracing::Instrument;

use super::super::{
    CommunityReport, ConversationHistory, ConversationRole, Covariate, Entity, QueryContext,
    QueryContextRecords, QueryContextText, QueryDataIndex, QueryError, QueryUsageCategory,
    Relationship, Result, SearchMethod, TextUnit,
    context::ContextTable,
    explainability::LocalQueryExplainability as QueryExplainabilitySession,
    observability::{
        QueryTraceSession, query_error_kind, record_stage_error, record_u64, usize_to_u64,
    },
    result::resolve_embedding_prompt_tokens,
};
use crate::{
    LocalSearchConfig,
    explainability::{
        CandidatesFiltered, CandidatesRetrieved, CommunityReportsSelected, ContextBudgetAllocated,
        ContextCompleted, ContextSectionBudget, ContextSectionBuilt, ContextSectionKind,
        CovariatesSelected, EmbeddingCompleted, EmbeddingStarted, EntitiesSelected,
        ExplainabilityCandidate, ExplainabilityContextSection, ExplainabilityEvent,
        ExplainabilityRecordType, ExplainabilityScore, GraphExpansionStarted, MappingQueryBuilt,
        RelationshipsSelected, SelectionReason, TextUnitsSelected,
    },
    observability::{error_kind, event_name, field_name, operation, span_name, status},
};

/// Local Search context resources, independent of completion orchestration.
#[derive(Debug)]
pub(crate) struct LocalContextBuilder {
    pub(crate) method: SearchMethod,
    pub(crate) config: LocalSearchConfig,
    pub(crate) entities: Vec<Entity>,
    pub(crate) reports: Vec<CommunityReport>,
    pub(crate) text_units: Vec<TextUnit>,
    pub(crate) relationships: Vec<Relationship>,
    pub(crate) covariates: Vec<Covariate>,
    pub(crate) index: Arc<QueryDataIndex>,
    pub(crate) embedding_model: Arc<dyn EmbeddingModel>,
    pub(crate) embedding_model_id: String,
    pub(crate) embedding_provider: String,
    pub(crate) vector_store: Arc<dyn VectorStore>,
    pub(crate) vector_schema: VectorIndexSchema,
    pub(crate) tokenizer: Arc<dyn Tokenizer>,
}

/// Completed Local context and its embedding usage.
#[derive(Debug)]
pub(crate) struct LocalContextBuild {
    pub(crate) context: QueryContext,
    pub(crate) usage: QueryUsageCategory,
    pub(crate) context_tokens: usize,
}

#[derive(Debug)]
struct Section {
    text: String,
    table: ContextTable,
    tokens_used: usize,
    explainability: Option<SectionExplainability>,
}

#[derive(Debug)]
struct LocalSections {
    text: String,
    tables: BTreeMap<String, ContextTable>,
    tokens_used: usize,
    explainability: Vec<SectionExplainability>,
}

#[derive(Debug)]
struct ContextAssembly {
    parts: Vec<String>,
    tables: BTreeMap<String, ContextTable>,
    tokens_used: usize,
    explainability: Vec<SectionExplainability>,
}

#[derive(Debug)]
struct ConversationHistorySection {
    text: String,
    table: ContextTable,
    tokens_used: usize,
    capture: Option<SectionExplainability>,
}

#[derive(Debug, Clone)]
struct SectionExplainability {
    kind: ContextSectionKind,
    name: Option<String>,
    token_budget: usize,
    tokens_used: usize,
    candidate_count: usize,
    selected_count: usize,
    selected_record_ids: Vec<String>,
    truncated: bool,
    candidates: Vec<ExplainabilityCandidate>,
}

#[derive(Debug)]
struct FittedTable {
    table: ContextTable,
    tokens_used: usize,
    candidate_count: usize,
    selected_count: usize,
}

#[derive(Debug)]
struct SectionExplainabilitySpec {
    kind: ContextSectionKind,
    name: Option<String>,
    token_budget: usize,
    selected_reason: SelectionReason,
}

#[derive(Debug, Clone, Copy)]
struct RankedRelationship<'a> {
    relationship: &'a Relationship,
    links: Option<usize>,
}

#[derive(Debug)]
struct RelationshipSelection<'a> {
    selected: Vec<RankedRelationship<'a>>,
    rank_filtered: Vec<RankedRelationship<'a>>,
}

#[derive(Debug)]
struct SourceSelection<'a> {
    ranked: Vec<(&'a TextUnit, usize, usize)>,
    missing: Option<Vec<ExplainabilityCandidate>>,
}

#[derive(Debug)]
struct CommunitySelection<'a> {
    selected: Vec<(&'a CommunityReport, usize)>,
    non_token_candidates: Option<Vec<ExplainabilityCandidate>>,
}

#[derive(Debug)]
struct EntitySection {
    text: String,
    table: ContextTable,
    tokens_used: usize,
    explainability: Option<SectionExplainability>,
}

#[derive(Debug)]
struct LocalExpansionAttempt {
    text: Vec<String>,
    tables: BTreeMap<String, ContextTable>,
    tokens_used: usize,
    explainability: Vec<SectionExplainability>,
}

impl LocalContextBuilder {
    pub(crate) async fn build(
        &self,
        query: &str,
        conversation_history: Option<&ConversationHistory>,
    ) -> Result<LocalContextBuild> {
        self.build_with_entity_filters(query, conversation_history, &[], &[])
            .await
    }

    pub(crate) async fn build_with_entity_filters(
        &self,
        query: &str,
        conversation_history: Option<&ConversationHistory>,
        include_entity_names: &[String],
        exclude_entity_names: &[String],
    ) -> Result<LocalContextBuild> {
        self.build_with_entity_filters_and_explainability(
            query,
            conversation_history,
            include_entity_names,
            exclude_entity_names,
            None,
            None,
        )
        .await
    }

    pub(crate) async fn build_explainable(
        &self,
        query: &str,
        conversation_history: Option<&ConversationHistory>,
        explainability: Option<&QueryExplainabilitySession>,
        trace: Option<&QueryTraceSession>,
    ) -> Result<LocalContextBuild> {
        self.build_with_entity_filters_and_explainability(
            query,
            conversation_history,
            &[],
            &[],
            explainability,
            trace,
        )
        .await
    }

    async fn build_with_entity_filters_and_explainability(
        &self,
        query: &str,
        conversation_history: Option<&ConversationHistory>,
        include_entity_names: &[String],
        exclude_entity_names: &[String],
        explainability: Option<&QueryExplainabilitySession>,
        trace: Option<&QueryTraceSession>,
    ) -> Result<LocalContextBuild> {
        let context_span = trace.and_then(|trace| {
            trace.clone_root_span().map(|root_span| {
                tracing::info_span!(
                    parent: &root_span,
                    span_name::QUERY_CONTEXT,
                    "graphloom.operation" = operation::CONTEXT_BUILD,
                    "graphloom.context.tokens" = tracing::field::Empty,
                    "graphloom.candidate.count" = tracing::field::Empty,
                    "graphloom.selected.count" = tracing::field::Empty,
                    "graphloom.status" = tracing::field::Empty,
                    "graphloom.error.kind" = tracing::field::Empty,
                )
            })
        });
        let record_context_span = context_span.clone();
        let context_future = async {
            let outcome: Result<LocalContextBuild> = async {
                let mapping_query = conversation_history.map_or_else(
                    || query.to_owned(),
                    |history| {
                        history.mapping_query(query, self.config.conversation_history_max_turns)
                    },
                );
                self.emit_mapping_query(explainability, conversation_history, &mapping_query)
                    .await;
                let (selected_entities, usage) = self
                    .map_entities(
                        &mapping_query,
                        include_entity_names,
                        exclude_entity_names,
                        explainability,
                        trace,
                    )
                    .await?;
                self.emit_graph_expansion(explainability, &selected_entities)
                    .await;
                let assembly = self
                    .build_context_sections(
                        &selected_entities,
                        conversation_history,
                        explainability,
                        trace,
                    )
                    .await?;
                let context_tokens = assembly.tokens_used;
                let context_text = assembly.parts.join("\n\n");
                let records = self.context_records(assembly.tables)?;
                if let Some(session) = explainability {
                    self.emit_context_decisions(session, &assembly.explainability, &context_text)
                        .await;
                }
                Ok(LocalContextBuild {
                    context: QueryContext {
                        text: QueryContextText::Text(context_text),
                        records: QueryContextRecords::Tables(records),
                    },
                    usage,
                    context_tokens,
                })
            }
            .await;
            if let Some(span) = &record_context_span {
                match &outcome {
                    Ok(built) => {
                        record_u64(
                            span,
                            field_name::CONTEXT_TOKENS,
                            usize_to_u64(built.context_tokens),
                        );
                        span.record(field_name::STATUS, status::OK);
                    }
                    Err(error) => record_stage_error(span, query_error_kind(error)),
                }
            }
            outcome
        };
        match context_span {
            Some(span) => context_future.instrument(span).await,
            None => context_future.await,
        }
    }

    async fn map_entities<'a>(
        &'a self,
        query: &str,
        include_entity_names: &[String],
        exclude_entity_names: &[String],
        explainability: Option<&QueryExplainabilitySession>,
        trace: Option<&QueryTraceSession>,
    ) -> Result<(Vec<&'a Entity>, QueryUsageCategory)> {
        let mapping_span = trace.map(|_| {
            tracing::info_span!(
                span_name::QUERY_ENTITY_MAPPING,
                "graphloom.operation" = operation::ENTITY_MAPPING,
                "graphloom.candidate.count" = tracing::field::Empty,
                "graphloom.selected.count" = tracing::field::Empty,
                "graphloom.status" = tracing::field::Empty,
                "graphloom.error.kind" = tracing::field::Empty,
            )
        });
        let record_mapping_span = mapping_span.clone();
        let mapping_future = async {
            let outcome: Result<(Vec<&'a Entity>, QueryUsageCategory)> = async {
                if query.is_empty() {
                    Ok((
                        self.map_entities_by_rank(
                            include_entity_names,
                            exclude_entity_names,
                            explainability,
                            record_mapping_span.as_ref(),
                        )
                        .await,
                        QueryUsageCategory::default(),
                    ))
                } else {
                    self.map_entities_by_embedding(
                        query,
                        include_entity_names,
                        exclude_entity_names,
                        explainability,
                        record_mapping_span.as_ref(),
                        trace,
                    )
                    .await
                }
            }
            .await;
            if let Some(span) = &record_mapping_span {
                match &outcome {
                    Ok(_) => {
                        span.record(field_name::STATUS, status::OK);
                    }
                    Err(error) => record_stage_error(span, query_error_kind(error)),
                }
            }
            outcome
        };
        match mapping_span {
            Some(span) => mapping_future.instrument(span).await,
            None => mapping_future.await,
        }
    }

    async fn emit_mapping_query(
        &self,
        explainability: Option<&QueryExplainabilitySession>,
        conversation_history: Option<&ConversationHistory>,
        mapping_query: &str,
    ) {
        let Some(session) = explainability else {
            return;
        };
        let turn_count = conversation_history.map_or(0, |history| {
            history
                .recent_user_questions(self.config.conversation_history_max_turns)
                .len()
        });
        let Some(turn_count) = session.usize_to_u64(turn_count).and_then(|value| {
            u32::try_from(value).map_or_else(
                |_| {
                    session.mark_sidecar_failure("conversation_turn_conversion");
                    None
                },
                Some,
            )
        }) else {
            return;
        };
        let mut event = MappingQueryBuilt::new(turn_count);
        event.mapping_query = session.content(mapping_query);
        session
            .emit(
                session.spans().mapping(),
                Some(session.spans().root()),
                ExplainabilityEvent::MappingQueryBuilt(event),
            )
            .await;
    }

    async fn emit_graph_expansion(
        &self,
        explainability: Option<&QueryExplainabilitySession>,
        selected_entities: &[&Entity],
    ) {
        let Some(session) = explainability else {
            return;
        };
        session
            .emit(
                session.spans().graph_expansion(),
                Some(session.spans().root()),
                ExplainabilityEvent::GraphExpansionStarted(GraphExpansionStarted::new(
                    selected_entities
                        .iter()
                        .map(|entity| entity.id.clone())
                        .collect(),
                )),
            )
            .await;
    }

    async fn build_context_sections(
        &self,
        selected_entities: &[&Entity],
        conversation_history: Option<&ConversationHistory>,
        explainability: Option<&QueryExplainabilitySession>,
        trace: Option<&QueryTraceSession>,
    ) -> Result<ContextAssembly> {
        let mut remaining = self.config.max_context_tokens;
        let mut assembly = ContextAssembly {
            parts: Vec::new(),
            tables: BTreeMap::new(),
            tokens_used: 0,
            explainability: Vec::new(),
        };
        if let Some(history) = conversation_history
            && let Some(section) =
                self.build_conversation_history_capture(history, remaining, explainability)?
        {
            assembly.tokens_used = assembly.tokens_used.saturating_add(section.tokens_used);
            remaining = remaining.saturating_sub(section.tokens_used);
            if let Some(capture) = section.capture {
                assembly.explainability.push(capture);
            }
            assembly.parts.push(section.text);
            assembly
                .tables
                .insert("conversation history".to_owned(), section.table);
        }

        let community_tokens = proportion(remaining, self.config.community_prop);
        let local_proportion =
            (1.0 - self.config.community_prop - self.config.text_unit_prop).max(0.0);
        let local_tokens = proportion(remaining, local_proportion);
        let source_tokens = proportion(remaining, self.config.text_unit_prop);
        if let Some(session) = explainability {
            self.emit_context_budget(
                session,
                conversation_history.is_some(),
                community_tokens,
                local_tokens,
                source_tokens,
            )
            .await;
        }

        if let Some(section) = self.build_community_context_capture(
            selected_entities,
            community_tokens,
            explainability,
        )? {
            assembly.tokens_used = assembly.tokens_used.saturating_add(section.tokens_used);
            if let Some(capture) = section.explainability {
                assembly.explainability.push(capture);
            }
            assembly.parts.push(section.text);
            assembly.tables.insert("reports".to_owned(), section.table);
        }

        let local = self.build_local_context_capture(
            selected_entities,
            local_tokens,
            explainability,
            trace,
        )?;
        assembly.tokens_used = assembly.tokens_used.saturating_add(local.tokens_used);
        assembly.explainability.extend(local.explainability);
        if !local.text.trim().is_empty() {
            assembly.parts.push(local.text);
            assembly.tables.extend(local.tables);
        }

        if let Some(section) =
            self.build_source_context_capture(selected_entities, source_tokens, explainability)?
        {
            assembly.tokens_used = assembly.tokens_used.saturating_add(section.tokens_used);
            if let Some(capture) = section.explainability {
                assembly.explainability.push(capture);
            }
            if !section.text.is_empty() {
                assembly.parts.push(section.text);
                assembly.tables.insert("sources".to_owned(), section.table);
            }
        }
        Ok(assembly)
    }

    fn build_conversation_history_capture(
        &self,
        history: &ConversationHistory,
        remaining: usize,
        explainability: Option<&QueryExplainabilitySession>,
    ) -> Result<Option<ConversationHistorySection>> {
        let built = history.build_user_context(
            &self.tokenizer,
            self.config.conversation_history_max_turns,
            remaining,
        )?;
        if built.text.trim().is_empty() {
            return Ok(None);
        }
        let tokens_used = self.count(&built.text, "count conversation history context")?;
        let capture = explainability.map(|_| {
            let candidate_count = history
                .turns
                .iter()
                .filter(|turn| turn.role == ConversationRole::User)
                .take(if self.config.conversation_history_max_turns == 0 {
                    usize::MAX
                } else {
                    self.config.conversation_history_max_turns
                })
                .count();
            let selected_count = built.table.len();
            SectionExplainability {
                kind: ContextSectionKind::ConversationHistory,
                name: None,
                token_budget: self.config.max_context_tokens,
                tokens_used,
                candidate_count,
                selected_count,
                selected_record_ids: Vec::new(),
                truncated: selected_count < candidate_count,
                candidates: Vec::new(),
            }
        });
        Ok(Some(ConversationHistorySection {
            text: built.text,
            table: built.table,
            tokens_used,
            capture,
        }))
    }

    fn context_records(
        &self,
        context_tables: BTreeMap<String, ContextTable>,
    ) -> Result<BTreeMap<String, DataFrame>> {
        context_tables
            .into_iter()
            .map(|(name, table)| {
                table
                    .to_dataframe(self.method, "build Local context records")
                    .and_then(|mut dataframe| {
                        if local_table_requires_in_context(&name) {
                            dataframe
                                .with_column(
                                    Series::new(
                                        "in_context".into(),
                                        vec![true; dataframe.height()],
                                    )
                                    .into(),
                                )
                                .map_err(|source| QueryError::QueryContext {
                                    method: self.method,
                                    operation: "mark standard Local context records",
                                    message: source.to_string(),
                                })?;
                        }
                        Ok((name, dataframe))
                    })
            })
            .collect()
    }

    async fn map_entities_by_rank<'a>(
        &'a self,
        include_entity_names: &[String],
        exclude_entity_names: &[String],
        explainability: Option<&QueryExplainabilitySession>,
        mapping_span: Option<&tracing::Span>,
    ) -> Vec<&'a Entity> {
        let mut candidates = self.entities.iter().collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .rank
                .unwrap_or_default()
                .cmp(&left.rank.unwrap_or_default())
        });
        candidates.truncate(self.config.top_k_entities);
        if let Some(session) = explainability {
            let retrieved = candidates
                .iter()
                .enumerate()
                .filter_map(|(index, entity)| {
                    entity_candidate_with_rank(session, entity, index, None, None, false)
                })
                .collect();
            self.emit_retrieved(session, retrieved).await;
        }
        let filter_candidates = explainability.map(|_| {
            candidates
                .iter()
                .map(|entity| {
                    let mut candidate = entity_candidate(entity);
                    let excluded = exclude_entity_names
                        .iter()
                        .any(|name| name == &entity.title);
                    candidate.selected = !excluded;
                    candidate.reason = excluded.then_some(SelectionReason::ExplicitlyExcluded);
                    candidate
                })
                .collect::<Vec<_>>()
        });
        let candidate_count = candidates.len();
        let selected = add_entity_filters(
            &self.entities,
            &self.index,
            candidates,
            include_entity_names,
            exclude_entity_names,
        );
        if let Some(span) = mapping_span {
            record_u64(
                span,
                field_name::CANDIDATE_COUNT,
                usize_to_u64(candidate_count),
            );
            record_u64(
                span,
                field_name::SELECTED_COUNT,
                usize_to_u64(selected.len()),
            );
        }
        if let Some(session) = explainability {
            self.emit_entity_filter_events(
                session,
                &selected,
                include_entity_names,
                filter_candidates.unwrap_or_default(),
                None,
            )
            .await;
        }
        selected
    }

    async fn map_entities_by_embedding<'a>(
        &'a self,
        query: &str,
        include_entity_names: &[String],
        exclude_entity_names: &[String],
        explainability: Option<&QueryExplainabilitySession>,
        mapping_span: Option<&tracing::Span>,
        trace: Option<&QueryTraceSession>,
    ) -> Result<(Vec<&'a Entity>, QueryUsageCategory)> {
        let (vector, prompt_tokens) = self
            .embed_mapping_query(query, explainability, trace)
            .await?;
        let results = self
            .retrieve_mapping_candidates(&vector, explainability, trace)
            .await?;
        let candidate_count = results.len();
        let (entities, filter_candidates) =
            self.resolve_ann_entities(results, exclude_entity_names, explainability);
        let selected = add_entity_filters(
            &self.entities,
            &self.index,
            entities,
            include_entity_names,
            exclude_entity_names,
        );
        if let Some(span) = mapping_span {
            record_u64(
                span,
                field_name::CANDIDATE_COUNT,
                usize_to_u64(candidate_count),
            );
            record_u64(
                span,
                field_name::SELECTED_COUNT,
                usize_to_u64(selected.len()),
            );
        }
        if let Some(session) = explainability {
            self.emit_entity_filter_events(
                session,
                &selected,
                include_entity_names,
                filter_candidates.unwrap_or_default(),
                Some(SelectionReason::AnnResult),
            )
            .await;
        }
        Ok((
            selected,
            QueryUsageCategory {
                llm_calls: 1,
                prompt_tokens,
                output_tokens: 0,
            },
        ))
    }

    async fn embed_mapping_query(
        &self,
        query: &str,
        explainability: Option<&QueryExplainabilitySession>,
        trace: Option<&QueryTraceSession>,
    ) -> Result<(Vec<f32>, usize)> {
        if let Some(session) = explainability {
            let mut event = EmbeddingStarted::new(self.embedding_model_id.clone());
            event.input = session.content(query);
            session
                .emit(
                    session.spans().embedding(),
                    Some(session.spans().mapping()),
                    ExplainabilityEvent::EmbeddingStarted(event),
                )
                .await;
        }
        let embedding_span = trace.map(|_| {
            tracing::info_span!(
                span_name::EMBEDDING_REQUEST,
                "graphloom.operation" = operation::EMBEDDING,
                "graphloom.model.instance" = self.embedding_model_id.as_str(),
                "graphloom.model.provider" = self.embedding_provider.as_str(),
                "graphloom.input.count" = 1_u64,
                "graphloom.input.tokens" = tracing::field::Empty,
                "graphloom.embedding.dimensions" = tracing::field::Empty,
                "graphloom.status" = tracing::field::Empty,
                "graphloom.error.kind" = tracing::field::Empty,
            )
        });
        let record_embedding_span = embedding_span.clone();
        let embed_future = async {
            let outcome = self.embed_mapping_query_core(query).await;
            if let Some(span) = &record_embedding_span {
                match &outcome {
                    Ok((vector, prompt_tokens)) => {
                        record_u64(span, field_name::INPUT_TOKENS, usize_to_u64(*prompt_tokens));
                        record_u64(
                            span,
                            field_name::EMBEDDING_DIMENSIONS,
                            usize_to_u64(vector.len()),
                        );
                        span.record(field_name::STATUS, status::OK);
                    }
                    Err(error) => record_stage_error(span, query_error_kind(error)),
                }
            }
            outcome
        };
        let (vector, prompt_tokens) = match embedding_span {
            Some(span) => embed_future.instrument(span).await?,
            None => embed_future.await?,
        };
        if let Some(session) = explainability {
            match (
                session.usize_to_u64(prompt_tokens),
                u32::try_from(vector.len()),
            ) {
                (Some(prompt_tokens), Ok(dimensions)) => {
                    session
                        .emit(
                            session.spans().embedding(),
                            Some(session.spans().mapping()),
                            ExplainabilityEvent::EmbeddingCompleted(EmbeddingCompleted::new(
                                self.embedding_model_id.clone(),
                                prompt_tokens,
                                dimensions,
                            )),
                        )
                        .await;
                }
                (_, Err(_)) => session.mark_sidecar_failure("embedding_dimension_conversion"),
                (None, Ok(_)) => {}
            }
        }
        Ok((vector, prompt_tokens))
    }

    async fn embed_mapping_query_core(&self, query: &str) -> Result<(Vec<f32>, usize)> {
        let response = self
            .embedding_model
            .embed(EmbeddingRequest::new(vec![query.to_owned()]))
            .await
            .map_err(|source| QueryError::QueryEmbedding {
                method: self.method,
                operation: "embed Local Search entity mapping query",
                model: self.embedding_model_id.clone(),
                source: Box::new(source),
            })?;
        let prompt_tokens = resolve_embedding_prompt_tokens(
            response.usage.prompt_tokens,
            query,
            self.tokenizer.as_ref(),
            self.method,
            "count Local Search entity mapping embedding input tokens",
            &self.embedding_model_id,
        )?;
        let vector = response
            .into_embeddings()
            .into_iter()
            .next()
            .ok_or_else(|| QueryError::QueryEmbedding {
                method: self.method,
                operation: "read Local Search query embedding",
                model: self.embedding_model_id.clone(),
                source: Box::new(graphloom_llm::LlmError::InvalidResponse {
                    model_instance: self.embedding_model_id.clone(),
                    operation: "embedding conversion",
                    message: "provider returned no query embedding".to_owned(),
                }),
            })?;
        if vector.iter().any(|value| !value.is_finite()) {
            return Err(QueryError::QueryEmbedding {
                method: self.method,
                operation: "validate Local Search query embedding",
                model: self.embedding_model_id.clone(),
                source: Box::new(graphloom_llm::LlmError::InvalidResponse {
                    model_instance: self.embedding_model_id.clone(),
                    operation: "embedding conversion",
                    message: "provider returned a non-finite query embedding".to_owned(),
                }),
            });
        }
        Ok((vector, prompt_tokens))
    }

    async fn retrieve_mapping_candidates(
        &self,
        vector: &[f32],
        explainability: Option<&QueryExplainabilitySession>,
        trace: Option<&QueryTraceSession>,
    ) -> Result<Vec<VectorSearchResult>> {
        let ann_k = self.config.top_k_entities.checked_mul(2).ok_or_else(|| {
            QueryError::InvalidQueryConfig {
                method: self.method,
                operation: "compute Local Search ANN oversampling",
                message: "top_k_entities * 2 exceeds usize".to_owned(),
            }
        })?;
        let vector_span = trace.map(|_| {
            tracing::info_span!(
                span_name::VECTOR_SEARCH,
                "graphloom.operation" = operation::VECTOR_SEARCH,
                "graphloom.vector.index" = self.vector_schema.index_name.as_str(),
                "graphloom.retrieval.top_k" = tracing::field::Empty,
                "graphloom.candidate.count" = tracing::field::Empty,
                "graphloom.status" = tracing::field::Empty,
                "graphloom.error.kind" = tracing::field::Empty,
            )
        });
        let record_vector_span = vector_span.clone();
        let search_future = async {
            let outcome: Result<Vec<VectorSearchResult>> = async {
                let results = self
                    .vector_store
                    .similarity_search_by_vector(&self.vector_schema, vector, ann_k, false)
                    .await
                    .map_err(|source| match source {
                        source @ VectorError::MissingIndex { .. } => {
                            QueryError::MissingVectorIndex {
                                method: self.method,
                                operation: "search entity_description",
                                index: self.vector_schema.index_name.clone(),
                                source: Box::new(source),
                            }
                        }
                        source => QueryError::InvalidVectorIndex {
                            method: self.method,
                            operation: "search entity_description",
                            index: self.vector_schema.index_name.clone(),
                            source: Box::new(source),
                        },
                    })?;
                if let Some(session) = explainability {
                    let candidates = results
                        .iter()
                        .enumerate()
                        .filter_map(|(index, result)| {
                            raw_ann_candidate(session, &result.document.id, result.score, index)
                        })
                        .collect();
                    self.emit_retrieved(session, candidates).await;
                }
                Ok(results)
            }
            .await;
            if let Some(span) = &record_vector_span {
                match &outcome {
                    Ok(results) => {
                        record_u64(span, field_name::RETRIEVAL_TOP_K, usize_to_u64(ann_k));
                        record_u64(
                            span,
                            field_name::CANDIDATE_COUNT,
                            usize_to_u64(results.len()),
                        );
                        span.record(field_name::STATUS, status::OK);
                    }
                    Err(error) => record_stage_error(span, query_error_kind(error)),
                }
            }
            outcome
        };
        match vector_span {
            Some(span) => search_future.instrument(span).await,
            None => search_future.await,
        }
    }

    fn resolve_ann_entities<'a>(
        &'a self,
        results: Vec<VectorSearchResult>,
        exclude_entity_names: &[String],
        explainability: Option<&QueryExplainabilitySession>,
    ) -> (Vec<&'a Entity>, Option<Vec<ExplainabilityCandidate>>) {
        let mut entities = Vec::with_capacity(results.len());
        let mut filter_candidates = explainability.map(|_| Vec::with_capacity(results.len()));
        for (index, result) in results.into_iter().enumerate() {
            let normalized = uuid::Uuid::parse_str(&result.document.id)
                .ok()
                .map(|value| value.simple().to_string());
            let position = self
                .index
                .entity_by_id
                .get(result.document.id.as_str())
                .copied()
                .or_else(|| {
                    normalized
                        .as_deref()
                        .and_then(|normalized_id| self.index.entity_by_id.get(normalized_id))
                        .copied()
                });
            if let Some(entity) = position.and_then(|position| self.entities.get(position)) {
                entities.push(entity);
                if let (Some(session), Some(candidates)) =
                    (explainability, filter_candidates.as_mut())
                {
                    let excluded = exclude_entity_names
                        .iter()
                        .any(|name| name == &entity.title);
                    if let Some(mut candidate) = entity_candidate_with_rank(
                        session,
                        entity,
                        index,
                        Some(result.score),
                        Some(if excluded {
                            SelectionReason::ExplicitlyExcluded
                        } else {
                            SelectionReason::AnnResult
                        }),
                        !excluded,
                    ) {
                        candidate.selected = !excluded;
                        candidates.push(candidate);
                    }
                }
            } else {
                if let (Some(session), Some(candidates)) =
                    (explainability, filter_candidates.as_mut())
                    && let Some(mut candidate) =
                        raw_ann_candidate(session, &result.document.id, result.score, index)
                {
                    candidate.selected = false;
                    candidate.reason = Some(SelectionReason::StaleReference);
                    candidates.push(candidate);
                }
                tracing::warn!(
                    name: event_name::QUERY_ENTITY_MAPPING_STALE_REFERENCE,
                    {
                        "graphloom.query.method" = "local",
                        "graphloom.error.kind" = error_kind::STALE_REFERENCE,
                    },
                    "entity mapping ignored a stale vector reference"
                );
            }
        }
        (entities, filter_candidates)
    }

    async fn emit_retrieved(
        &self,
        session: &QueryExplainabilitySession,
        candidates: Vec<ExplainabilityCandidate>,
    ) {
        session
            .emit_contract(
                session.spans().retrieval(),
                Some(session.spans().mapping()),
                CandidatesRetrieved::try_new(ExplainabilityRecordType::Entity, candidates)
                    .map(ExplainabilityEvent::CandidatesRetrieved),
            )
            .await;
    }

    async fn emit_entity_filter_events(
        &self,
        session: &QueryExplainabilitySession,
        selected_entities: &[&Entity],
        include_entity_names: &[String],
        filter_candidates: Vec<ExplainabilityCandidate>,
        selected_reason: Option<SelectionReason>,
    ) {
        let included = included_entities(&self.entities, &self.index, include_entity_names);
        let mut filtered =
            Vec::with_capacity(included.len().saturating_add(filter_candidates.len()));
        filtered.extend(included.iter().map(|entity| {
            let mut candidate = entity_candidate(entity);
            candidate.selected = true;
            candidate.reason = Some(SelectionReason::ExplicitlyIncluded);
            candidate
        }));
        filtered.extend(filter_candidates);
        session
            .emit_contract(
                session.spans().mapping(),
                Some(session.spans().root()),
                CandidatesFiltered::try_new(ExplainabilityRecordType::Entity, filtered)
                    .map(ExplainabilityEvent::CandidatesFiltered),
            )
            .await;

        let selected = selected_entities
            .iter()
            .enumerate()
            .map(|(index, entity)| {
                let mut candidate = entity_candidate(entity);
                candidate.selected = true;
                candidate.reason = if index < included.len() {
                    Some(SelectionReason::ExplicitlyIncluded)
                } else {
                    selected_reason
                };
                candidate
            })
            .collect();
        session
            .emit_contract(
                session.spans().mapping(),
                Some(session.spans().root()),
                EntitiesSelected::try_new(selected).map(ExplainabilityEvent::EntitiesSelected),
            )
            .await;
    }

    async fn emit_context_budget(
        &self,
        session: &QueryExplainabilitySession,
        has_conversation_history: bool,
        community_tokens: usize,
        local_tokens: usize,
        source_tokens: usize,
    ) {
        let mut raw_sections = Vec::with_capacity(4);
        if has_conversation_history {
            raw_sections.push((
                ContextSectionKind::ConversationHistory,
                self.config.max_context_tokens,
            ));
        }
        raw_sections.extend([
            (ContextSectionKind::CommunityReports, community_tokens),
            (ContextSectionKind::LocalGraph, local_tokens),
            (ContextSectionKind::Sources, source_tokens),
        ]);
        let Some(total_token_budget) = session.usize_to_u64(self.config.max_context_tokens) else {
            return;
        };
        let mut sections = Vec::with_capacity(raw_sections.len());
        for (kind, budget) in raw_sections {
            let Some(budget) = session.usize_to_u64(budget) else {
                return;
            };
            sections.push(ContextSectionBudget::new(kind, budget));
        }
        session
            .emit(
                session.spans().context(),
                Some(session.spans().root()),
                ExplainabilityEvent::ContextBudgetAllocated(ContextBudgetAllocated::new(
                    total_token_budget,
                    sections,
                )),
            )
            .await;
    }

    async fn emit_context_decisions(
        &self,
        session: &QueryExplainabilitySession,
        sections: &[SectionExplainability],
        context_text: &str,
    ) {
        let candidates = |kind| {
            sections
                .iter()
                .filter(|section| section.kind == kind)
                .flat_map(|section| section.candidates.iter().cloned())
                .collect::<Vec<_>>()
        };
        session
            .emit_contract(
                session.spans().graph_expansion(),
                Some(session.spans().root()),
                CommunityReportsSelected::try_new(candidates(ContextSectionKind::CommunityReports))
                    .map(ExplainabilityEvent::CommunityReportsSelected),
            )
            .await;
        session
            .emit_contract(
                session.spans().graph_expansion(),
                Some(session.spans().root()),
                RelationshipsSelected::try_new(candidates(ContextSectionKind::Relationships))
                    .map(ExplainabilityEvent::RelationshipsSelected),
            )
            .await;
        session
            .emit_contract(
                session.spans().graph_expansion(),
                Some(session.spans().root()),
                CovariatesSelected::try_new(candidates(ContextSectionKind::Covariates))
                    .map(ExplainabilityEvent::CovariatesSelected),
            )
            .await;
        session
            .emit_contract(
                session.spans().graph_expansion(),
                Some(session.spans().root()),
                TextUnitsSelected::try_new(candidates(ContextSectionKind::Sources))
                    .map(ExplainabilityEvent::TextUnitsSelected),
            )
            .await;

        for captured in sections {
            let Some(token_budget) = session.usize_to_u64(captured.token_budget) else {
                continue;
            };
            let Some(tokens_used) = session.usize_to_u64(captured.tokens_used) else {
                continue;
            };
            let Some(candidate_count) = session.usize_to_u64(captured.candidate_count) else {
                continue;
            };
            let Some(selected_count) = session.usize_to_u64(captured.selected_count) else {
                continue;
            };
            let mut section = ExplainabilityContextSection::new(captured.kind, token_budget);
            section.name = captured.name.clone();
            section.tokens_used = tokens_used;
            section.candidate_count = candidate_count;
            section.selected_count = selected_count;
            section.truncated = captured.truncated;
            section.selected_record_ids = captured.selected_record_ids.clone();
            session
                .emit(
                    session.spans().context(),
                    Some(session.spans().root()),
                    ExplainabilityEvent::ContextSectionBuilt(ContextSectionBuilt::new(section)),
                )
                .await;
        }

        match self.tokenizer.count(context_text) {
            Ok(tokens) => {
                if let Some(tokens) = session.usize_to_u64(tokens) {
                    let mut event = ContextCompleted::new(tokens);
                    event.context = session.content(context_text);
                    session
                        .emit(
                            session.spans().context(),
                            Some(session.spans().root()),
                            ExplainabilityEvent::ContextCompleted(event),
                        )
                        .await;
                }
            }
            Err(_) => session.mark_sidecar_failure("context_token_count"),
        }
    }

    #[cfg(test)]
    fn build_community_context(
        &self,
        selected_entities: &[&Entity],
        max_tokens: usize,
    ) -> Result<Option<Section>> {
        self.build_community_context_capture(selected_entities, max_tokens, None)
    }

    fn build_community_context_capture(
        &self,
        selected_entities: &[&Entity],
        max_tokens: usize,
        explainability: Option<&QueryExplainabilitySession>,
    ) -> Result<Option<Section>> {
        if selected_entities.is_empty() || self.reports.is_empty() {
            return Ok(None);
        }
        let matches = community_matches(selected_entities);
        if matches.is_empty() {
            let candidates = explainability.map(|_| Vec::new());
            let tokens_used = explainability
                .and_then(|session| self.count_empty_community_for_explainability(session));
            return Ok(Some(empty_community_section(
                max_tokens,
                candidates,
                tokens_used,
            )));
        }
        let mut selection = self.select_community_reports(matches, explainability.is_some());
        let non_token_candidates = selection.non_token_candidates.take();
        let selected = &mut selection.selected;
        if selected.is_empty() {
            let tokens_used = explainability
                .and_then(|session| self.count_empty_community_for_explainability(session));
            return Ok(Some(empty_community_section(
                max_tokens,
                non_token_candidates,
                tokens_used,
            )));
        }
        selected.sort_by(|(left, left_matches), (right, right_matches)| {
            right_matches.cmp(left_matches).then_with(|| {
                right
                    .rank
                    .unwrap_or_default()
                    .total_cmp(&left.rank.unwrap_or_default())
            })
        });
        let candidate_reports = explainability.map(|_| {
            selected
                .iter()
                .map(|(report, _)| *report)
                .collect::<Vec<_>>()
        });
        let candidates = selected
            .iter()
            .map(|(report, _)| {
                vec![
                    report.short_id.clone(),
                    report.title.clone(),
                    report.full_content.clone(),
                ]
            })
            .collect::<Vec<_>>();
        let mut fitted = self.fit_report_rows(
            ContextTable::new(["id", "title", "content"], Vec::new()),
            candidates,
            "Reports",
            max_tokens,
            "build Local Reports context",
        )?;
        if fitted.table.is_empty() {
            let empty_tokens = explainability
                .and_then(|session| self.count_empty_community_for_explainability(session));
            if let Some(tokens) = empty_tokens {
                fitted.tokens_used = tokens;
            }
            let capture = empty_tokens.map(|_| {
                build_record_section_explainability(
                    SectionExplainabilitySpec {
                        kind: ContextSectionKind::CommunityReports,
                        name: None,
                        token_budget: max_tokens,
                        selected_reason: SelectionReason::CommunityMembership,
                    },
                    &fitted,
                    candidate_reports.unwrap_or_default(),
                    non_token_candidates.unwrap_or_default(),
                    community_report_candidate,
                )
            });
            return Ok(Some(Section {
                text: "[]".to_owned(),
                table: fitted.table,
                tokens_used: fitted.tokens_used,
                explainability: capture,
            }));
        }
        let text = fitted.table.render_csv_section(
            "Reports",
            self.method,
            "render Local Reports context",
        )?;
        if let Some(session) = explainability {
            fitted.tokens_used = self.count_for_explainability(session, &text, fitted.tokens_used);
        }
        let capture = explainability.map(|_| {
            build_record_section_explainability(
                SectionExplainabilitySpec {
                    kind: ContextSectionKind::CommunityReports,
                    name: None,
                    token_budget: max_tokens,
                    selected_reason: SelectionReason::CommunityMembership,
                },
                &fitted,
                candidate_reports.unwrap_or_default(),
                non_token_candidates.unwrap_or_default(),
                community_report_candidate,
            )
        });
        Ok(Some(Section {
            text,
            table: fitted.table,
            tokens_used: fitted.tokens_used,
            explainability: capture,
        }))
    }

    fn select_community_reports(
        &self,
        matches: Vec<(String, usize)>,
        capture_exclusions: bool,
    ) -> CommunitySelection<'_> {
        let mut selected = Vec::new();
        let mut non_token_candidates = capture_exclusions.then(Vec::new);
        for (community_id, count) in matches {
            let Some(report) = self
                .index
                .report_by_community_id
                .get(community_id.as_str())
                .and_then(|index| self.reports.get(*index))
            else {
                if let Some(candidates) = non_token_candidates.as_mut() {
                    let mut candidate = ExplainabilityCandidate::new(
                        community_id,
                        ExplainabilityRecordType::CommunityReport,
                    );
                    candidate.reason = Some(SelectionReason::MissingRecord);
                    candidates.push(candidate);
                }
                continue;
            };
            if report.rank.is_some_and(|rank| rank >= 0.0) {
                selected.push((report, count));
            } else if let Some(filtered) = non_token_candidates.as_mut() {
                let mut candidate = community_report_candidate(report);
                candidate.reason = Some(SelectionReason::RankThreshold);
                filtered.push(candidate);
            }
        }
        CommunitySelection {
            selected,
            non_token_candidates,
        }
    }

    #[cfg(test)]
    fn build_local_context(
        &self,
        selected_entities: &[&Entity],
        max_tokens: usize,
    ) -> Result<LocalSections> {
        self.build_local_context_capture(selected_entities, max_tokens, None, None)
    }

    fn build_local_context_capture(
        &self,
        selected_entities: &[&Entity],
        max_tokens: usize,
        explainability: Option<&QueryExplainabilitySession>,
        trace: Option<&QueryTraceSession>,
    ) -> Result<LocalSections> {
        if selected_entities.is_empty() {
            return Ok(LocalSections {
                text: String::new(),
                tables: BTreeMap::new(),
                tokens_used: 0,
                explainability: Vec::new(),
            });
        }
        let entity = self.build_entity_section(selected_entities, max_tokens, explainability)?;
        let mut expansion = self.expand_local_graph(
            selected_entities,
            max_tokens,
            entity.tokens_used,
            explainability,
            trace,
        )?;
        let mut text = vec![entity.text];
        text.append(&mut expansion.text);
        expansion.tables.insert("entities".to_owned(), entity.table);
        let mut section_explainability = entity.explainability.into_iter().collect::<Vec<_>>();
        section_explainability.append(&mut expansion.explainability);
        Ok(LocalSections {
            text: text
                .into_iter()
                .filter(|section| !section.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n\n"),
            tables: expansion.tables,
            tokens_used: expansion.tokens_used,
            explainability: section_explainability,
        })
    }

    fn build_entity_section(
        &self,
        selected_entities: &[&Entity],
        max_tokens: usize,
        explainability: Option<&QueryExplainabilitySession>,
    ) -> Result<EntitySection> {
        let entity_candidates = selected_entities
            .iter()
            .map(|entity| {
                vec![
                    entity.short_id.clone().unwrap_or_default(),
                    entity.title.clone(),
                    entity.description.clone().unwrap_or_default(),
                    python_optional_i64(entity.rank),
                ]
            })
            .collect::<Vec<_>>();
        let fitted_entities = self.fit_delimited_rows(
            ContextTable::new(
                ["id", "entity", "description", "number of relationships"],
                Vec::new(),
            ),
            entity_candidates,
            "Entities",
            max_tokens,
            "build Local Entities context",
        )?;
        let entity_text = fitted_entities.table.render_delimited_section(
            "Entities",
            self.method,
            "render Local Entities context",
        )?;
        let entity_tokens = self.count(&entity_text, "count Local Entities context")?;
        let capture = explainability.map(|_| SectionExplainability {
            kind: ContextSectionKind::Entities,
            name: None,
            token_budget: max_tokens,
            tokens_used: entity_tokens,
            candidate_count: fitted_entities.candidate_count,
            selected_count: fitted_entities.selected_count,
            selected_record_ids: selected_entities
                .iter()
                .take(fitted_entities.selected_count)
                .map(|entity| entity.id.clone())
                .collect(),
            truncated: fitted_entities.selected_count < fitted_entities.candidate_count,
            candidates: Vec::new(),
        });
        Ok(EntitySection {
            text: entity_text,
            table: fitted_entities.table,
            tokens_used: entity_tokens,
            explainability: capture,
        })
    }

    fn expand_local_graph(
        &self,
        selected_entities: &[&Entity],
        max_tokens: usize,
        entity_tokens: usize,
        explainability: Option<&QueryExplainabilitySession>,
        trace: Option<&QueryTraceSession>,
    ) -> Result<LocalExpansionAttempt> {
        let expansion_span = trace.map(|_| {
            tracing::info_span!(
                span_name::QUERY_GRAPH_EXPANSION,
                "graphloom.operation" = operation::GRAPH_EXPANSION,
                "graphloom.candidate.count" = tracing::field::Empty,
                "graphloom.selected.count" = tracing::field::Empty,
                "graphloom.status" = tracing::field::Empty,
                "graphloom.error.kind" = tracing::field::Empty,
            )
        });
        let outcome: Result<(LocalExpansionAttempt, usize, usize)> = match &expansion_span {
            Some(span) => span.in_scope(|| {
                self.expand_local_graph_inner(
                    selected_entities,
                    max_tokens,
                    entity_tokens,
                    explainability,
                )
            }),
            None => self.expand_local_graph_inner(
                selected_entities,
                max_tokens,
                entity_tokens,
                explainability,
            ),
        };
        if let Some(span) = &expansion_span {
            match &outcome {
                Ok((_, candidate_count, selected_count)) => {
                    record_u64(
                        span,
                        field_name::CANDIDATE_COUNT,
                        usize_to_u64(*candidate_count),
                    );
                    record_u64(
                        span,
                        field_name::SELECTED_COUNT,
                        usize_to_u64(*selected_count),
                    );
                    span.record(field_name::STATUS, status::OK);
                }
                Err(error) => record_stage_error(span, query_error_kind(error)),
            }
        }
        outcome.map(|(attempt, _, _)| attempt)
    }

    fn expand_local_graph_inner(
        &self,
        selected_entities: &[&Entity],
        max_tokens: usize,
        entity_tokens: usize,
        explainability: Option<&QueryExplainabilitySession>,
    ) -> Result<(LocalExpansionAttempt, usize, usize)> {
        let mut accepted = LocalExpansionAttempt {
            text: Vec::new(),
            tables: BTreeMap::new(),
            tokens_used: entity_tokens,
            explainability: Vec::new(),
        };
        let mut learned_links = BTreeMap::new();
        let mut relationship_positions = BTreeSet::new();
        let mut covariate_positions = Vec::new();
        for (index, added_entity) in selected_entities.iter().copied().enumerate() {
            let Some(current_entities) = selected_entities.get(..=index) else {
                break;
            };
            relationship_positions.extend(
                self.index
                    .relationships_by_entity
                    .get(added_entity.title.as_str())
                    .into_iter()
                    .flatten()
                    .copied(),
            );
            covariate_positions.extend(
                self.index
                    .covariates_by_subject
                    .get(added_entity.title.as_str())
                    .into_iter()
                    .flatten()
                    .copied(),
            );
            let attempt = self.build_local_expansion_attempt(
                current_entities,
                &relationship_positions,
                &covariate_positions,
                max_tokens,
                entity_tokens,
                &mut learned_links,
                explainability,
            )?;
            if attempt.tokens_used > max_tokens {
                if explainability.is_some() {
                    accepted.explainability = rollback_section_explainability(
                        attempt.explainability,
                        &accepted.explainability,
                    );
                }
                tracing::warn!(
                    method = %self.method,
                    "Local entity expansion reached the token limit; reverting the current entity"
                );
                break;
            }
            accepted = attempt;
        }
        let candidate_count = relationship_positions
            .len()
            .saturating_add(covariate_positions.len());
        let selected_count = accepted
            .tables
            .values()
            .map(ContextTable::len)
            .sum::<usize>();
        Ok((accepted, candidate_count, selected_count))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the arguments are distinct immutable views of one progressive graph-expansion \
                  step"
    )]
    fn build_local_expansion_attempt(
        &self,
        current_entities: &[&Entity],
        relationship_positions: &BTreeSet<usize>,
        covariate_positions: &[usize],
        max_tokens: usize,
        entity_tokens: usize,
        learned_links: &mut BTreeMap<String, usize>,
        explainability: Option<&QueryExplainabilitySession>,
    ) -> Result<LocalExpansionAttempt> {
        let relationship = self.build_relationship_context_from_positions_capture(
            current_entities,
            relationship_positions,
            max_tokens,
            learned_links,
            explainability,
        )?;
        let mut attempt = LocalExpansionAttempt {
            text: Vec::new(),
            tables: BTreeMap::new(),
            tokens_used: entity_tokens,
            explainability: Vec::new(),
        };
        if let Some(section) = relationship {
            let section_tokens = if section.text.is_empty() {
                0
            } else {
                self.count(&section.text, "count Local Relationships context")?
            };
            attempt.tokens_used = attempt.tokens_used.saturating_add(section_tokens);
            if let Some(mut capture) = section.explainability {
                capture.tokens_used = section_tokens;
                attempt.explainability.push(capture);
            }
            if !section.text.is_empty() {
                attempt.text.push(section.text);
            }
            attempt
                .tables
                .insert("relationships".to_owned(), section.table);
        } else {
            if explainability.is_some() {
                attempt.explainability.push(empty_section_explainability(
                    ContextSectionKind::Relationships,
                    None,
                    max_tokens,
                ));
            }
            attempt.tables.insert(
                "relationships".to_owned(),
                ContextTable::new(
                    ["id", "source", "target", "description", "weight"],
                    Vec::new(),
                ),
            );
        }
        for (name, group_positions) in &self.index.covariate_groups {
            let section = self.build_covariate_context_from_positions_capture(
                name,
                group_positions,
                covariate_positions,
                max_tokens,
                explainability,
            )?;
            if let Some(section) = section {
                let section_tokens = self.count(&section.text, "count Local covariate context")?;
                attempt.tokens_used = attempt.tokens_used.saturating_add(section_tokens);
                if let Some(mut capture) = section.explainability {
                    capture.tokens_used = section_tokens;
                    attempt.explainability.push(capture);
                }
                attempt.text.push(section.text);
                attempt.tables.insert(name.to_lowercase(), section.table);
            } else {
                if explainability.is_some() {
                    attempt.explainability.push(empty_section_explainability(
                        ContextSectionKind::Covariates,
                        Some(name.clone()),
                        max_tokens,
                    ));
                }
                attempt.tables.insert(
                    name.to_lowercase(),
                    ContextTable::new(covariate_columns(), Vec::new()),
                );
            }
        }
        Ok(attempt)
    }

    #[cfg(test)]
    fn build_relationship_context_from_positions(
        &self,
        selected_entities: &[&Entity],
        relationship_positions: &BTreeSet<usize>,
        max_tokens: usize,
        learned_links: &mut BTreeMap<String, usize>,
    ) -> Result<Option<Section>> {
        self.build_relationship_context_from_positions_capture(
            selected_entities,
            relationship_positions,
            max_tokens,
            learned_links,
            None,
        )
    }

    fn build_relationship_context_from_positions_capture(
        &self,
        selected_entities: &[&Entity],
        relationship_positions: &BTreeSet<usize>,
        max_tokens: usize,
        learned_links: &mut BTreeMap<String, usize>,
        explainability: Option<&QueryExplainabilitySession>,
    ) -> Result<Option<Section>> {
        let selection = filter_relationships_capture(
            selected_entities,
            &self.relationships,
            relationship_positions,
            self.config.top_k_relationships,
            learned_links,
            explainability.is_some(),
        );
        if selection.selected.is_empty() && selection.rank_filtered.is_empty() {
            return Ok(None);
        }
        if selection.selected.is_empty() {
            return Ok(
                explainability.map(|_| rank_filtered_relationship_section(&selection, max_tokens))
            );
        }
        let include_links = selection
            .selected
            .first()
            .is_some_and(|value| value.links.is_some());
        let mut columns = vec!["id", "source", "target", "description", "weight"];
        if include_links {
            columns.push("links");
        }
        let selected_relationships = explainability.map(|_| {
            selection
                .selected
                .iter()
                .map(|ranked| ranked.relationship)
                .collect::<Vec<_>>()
        });
        let candidates = selection
            .selected
            .iter()
            .map(|ranked| {
                let relationship = ranked.relationship;
                let mut row = vec![
                    relationship.short_id.clone().unwrap_or_default(),
                    relationship.source.clone(),
                    relationship.target.clone(),
                    relationship.description.clone().unwrap_or_default(),
                    python_optional_f64_truthy(relationship.weight),
                ];
                if include_links {
                    row.push(
                        ranked
                            .links
                            .map_or_else(String::new, |value| value.to_string()),
                    );
                }
                row
            })
            .collect::<Vec<_>>();
        let fitted = self.fit_delimited_rows(
            ContextTable::new(columns, Vec::new()),
            candidates,
            "Relationships",
            max_tokens,
            "build Local Relationships context",
        )?;
        let text = fitted.table.render_delimited_section(
            "Relationships",
            self.method,
            "render Local Relationships context",
        )?;
        let capture = explainability.map(|_| {
            let rank_filtered = selection
                .rank_filtered
                .iter()
                .map(|ranked| {
                    let mut candidate = relationship_candidate(ranked.relationship);
                    candidate.selected = false;
                    candidate.reason = Some(SelectionReason::RankThreshold);
                    candidate
                })
                .collect();
            build_record_section_explainability(
                SectionExplainabilitySpec {
                    kind: ContextSectionKind::Relationships,
                    name: None,
                    token_budget: max_tokens,
                    selected_reason: SelectionReason::GraphExpansion,
                },
                &fitted,
                selected_relationships.unwrap_or_default(),
                rank_filtered,
                relationship_candidate,
            )
        });
        Ok(Some(Section {
            text,
            table: fitted.table,
            tokens_used: fitted.tokens_used,
            explainability: capture,
        }))
    }

    #[cfg(test)]
    fn build_covariate_context_from_positions(
        &self,
        name: &str,
        group_positions: &HashSet<usize>,
        positions: &[usize],
        max_tokens: usize,
    ) -> Result<Option<Section>> {
        self.build_covariate_context_from_positions_capture(
            name,
            group_positions,
            positions,
            max_tokens,
            None,
        )
    }

    fn build_covariate_context_from_positions_capture(
        &self,
        name: &str,
        group_positions: &HashSet<usize>,
        positions: &[usize],
        max_tokens: usize,
        explainability: Option<&QueryExplainabilitySession>,
    ) -> Result<Option<Section>> {
        let selected = positions
            .iter()
            .filter(|index| group_positions.contains(index))
            .filter_map(|index| self.covariates.get(*index))
            .collect::<Vec<_>>();
        let selected_covariates = explainability.map(|_| selected.clone());
        let candidates = selected
            .iter()
            .map(|covariate| {
                vec![
                    covariate.short_id.clone().unwrap_or_default(),
                    covariate.subject_id.clone(),
                    covariate.object_id.clone().unwrap_or_default(),
                    covariate.status.clone().unwrap_or_default(),
                    covariate.start_date.clone().unwrap_or_default(),
                    covariate.end_date.clone().unwrap_or_default(),
                    covariate.description.clone().unwrap_or_default(),
                ]
            })
            .collect::<Vec<_>>();
        if group_positions.is_empty() {
            return Ok(None);
        }
        let fitted = self.fit_delimited_rows(
            ContextTable::new(covariate_columns(), Vec::new()),
            candidates,
            name,
            max_tokens,
            "build Local covariate context",
        )?;
        let text = fitted.table.render_delimited_section(
            name,
            self.method,
            "render Local covariate context",
        )?;
        let capture = explainability.map(|_| {
            build_record_section_explainability(
                SectionExplainabilitySpec {
                    kind: ContextSectionKind::Covariates,
                    name: Some(name.to_owned()),
                    token_budget: max_tokens,
                    selected_reason: SelectionReason::GraphExpansion,
                },
                &fitted,
                selected_covariates.unwrap_or_default(),
                Vec::new(),
                covariate_candidate,
            )
        });
        Ok(Some(Section {
            text,
            table: fitted.table,
            tokens_used: fitted.tokens_used,
            explainability: capture,
        }))
    }

    #[cfg(test)]
    fn build_source_context(
        &self,
        selected_entities: &[&Entity],
        max_tokens: usize,
    ) -> Result<Option<Section>> {
        self.build_source_context_capture(selected_entities, max_tokens, None)
    }

    fn build_source_context_capture(
        &self,
        selected_entities: &[&Entity],
        max_tokens: usize,
        explainability: Option<&QueryExplainabilitySession>,
    ) -> Result<Option<Section>> {
        if selected_entities.is_empty() {
            return Ok(None);
        }
        let selection = self.select_source_text_units(selected_entities, explainability.is_some());
        if selection.ranked.is_empty() {
            return Ok(selection
                .missing
                .filter(|candidates| !candidates.is_empty())
                .map(|candidates| missing_source_section(candidates, max_tokens)));
        }
        let selected_text_units = explainability.map(|_| {
            selection
                .ranked
                .iter()
                .map(|(unit, _, _)| *unit)
                .collect::<Vec<_>>()
        });
        let candidates = selection
            .ranked
            .iter()
            .map(|(unit, _, _)| vec![unit.short_id.clone(), unit.text.clone()])
            .collect::<Vec<_>>();
        let mut fitted = self.fit_delimited_rows(
            ContextTable::new(["id", "text"], Vec::new()),
            candidates,
            "Sources",
            max_tokens,
            "build Local Sources context",
        )?;
        let text = fitted.table.render_delimited_section(
            "Sources",
            self.method,
            "render Local Sources context",
        )?;
        if let Some(session) = explainability {
            fitted.tokens_used = self.count_for_explainability(session, &text, fitted.tokens_used);
        }
        let capture = explainability.map(|_| {
            build_record_section_explainability(
                SectionExplainabilitySpec {
                    kind: ContextSectionKind::Sources,
                    name: None,
                    token_budget: max_tokens,
                    selected_reason: SelectionReason::SourceReference,
                },
                &fitted,
                selected_text_units.unwrap_or_default(),
                selection.missing.unwrap_or_default(),
                text_unit_candidate,
            )
        });
        Ok(Some(Section {
            text,
            table: fitted.table,
            tokens_used: fitted.tokens_used,
            explainability: capture,
        }))
    }

    fn select_source_text_units<'a>(
        &'a self,
        selected_entities: &[&Entity],
        capture_missing: bool,
    ) -> SourceSelection<'a> {
        let mut seen = BTreeSet::new();
        let mut ranked = Vec::<(&TextUnit, usize, usize)>::new();
        let mut missing = capture_missing.then(Vec::new);
        for (entity_order, entity) in selected_entities.iter().enumerate() {
            let entity_relationships = self
                .index
                .relationships_by_entity
                .get(entity.title.as_str())
                .into_iter()
                .flatten()
                .filter_map(|index| self.relationships.get(*index))
                .collect::<Vec<_>>();
            for text_unit_id in &entity.text_unit_ids {
                if !seen.insert(text_unit_id.as_str()) {
                    continue;
                }
                let Some(unit) = self
                    .index
                    .text_unit_by_id
                    .get(text_unit_id.as_str())
                    .and_then(|index| self.text_units.get(*index))
                else {
                    if let Some(candidates) = missing.as_mut() {
                        let mut candidate = ExplainabilityCandidate::new(
                            text_unit_id.clone(),
                            ExplainabilityRecordType::TextUnit,
                        );
                        candidate.selected = false;
                        candidate.reason = Some(SelectionReason::MissingRecord);
                        candidates.push(candidate);
                    }
                    tracing::warn!(
                        method = %self.method,
                        text_unit_id,
                        "entity references a missing text unit"
                    );
                    continue;
                };
                ranked.push((
                    unit,
                    entity_order,
                    count_relationships(&entity_relationships, unit),
                ));
            }
        }
        ranked.sort_by(
            |(_, left_order, left_count), (_, right_order, right_count)| {
                left_order
                    .cmp(right_order)
                    .then_with(|| right_count.cmp(left_count))
            },
        );
        SourceSelection { ranked, missing }
    }

    fn fit_delimited_rows(
        &self,
        mut table: ContextTable,
        candidates: Vec<Vec<String>>,
        context_name: &str,
        max_tokens: usize,
        operation: &'static str,
    ) -> Result<FittedTable> {
        let header = table.render_delimited_header(context_name, self.method, operation)?;
        let mut tokens = self.count(&header, operation)?;
        let candidate_count = candidates.len();
        for row in candidates {
            let row_text = table.render_delimited_row(&row, self.method, operation)?;
            let row_tokens = self.count(&row_text, operation)?;
            if tokens.saturating_add(row_tokens) > max_tokens {
                break;
            }
            tokens = tokens.saturating_add(row_tokens);
            table.push(row);
        }
        let selected_count = table.len();
        Ok(FittedTable {
            table,
            tokens_used: tokens,
            candidate_count,
            selected_count,
        })
    }

    fn fit_report_rows(
        &self,
        mut table: ContextTable,
        candidates: Vec<Vec<String>>,
        context_name: &str,
        max_tokens: usize,
        operation: &'static str,
    ) -> Result<FittedTable> {
        let header = table.render_delimited_header(context_name, self.method, operation)?;
        let mut tokens = self.count(&header, operation)?;
        let candidate_count = candidates.len();
        for row in candidates {
            let row_text = table.render_delimited_row(&row, self.method, operation)?;
            let row_tokens = self.count(&row_text, operation)?;
            if tokens.saturating_add(row_tokens) > max_tokens {
                break;
            }
            tokens = tokens.saturating_add(row_tokens);
            table.push(row);
        }
        let selected_count = table.len();
        Ok(FittedTable {
            table,
            tokens_used: tokens,
            candidate_count,
            selected_count,
        })
    }

    fn count(&self, text: &str, operation: &'static str) -> Result<usize> {
        self.tokenizer
            .count(text)
            .map_err(|source| QueryError::QueryContext {
                method: self.method,
                operation,
                message: source.to_string(),
            })
    }

    fn count_for_explainability(
        &self,
        session: &QueryExplainabilitySession,
        text: &str,
        reliable_fallback: usize,
    ) -> usize {
        if let Ok(tokens) = self.tokenizer.count(text) {
            tokens
        } else {
            session.mark_sidecar_failure("section_token_count");
            reliable_fallback
        }
    }

    fn count_empty_community_for_explainability(
        &self,
        session: &QueryExplainabilitySession,
    ) -> Option<usize> {
        self.tokenizer.count("[]").map_or_else(
            |_| {
                session.mark_sidecar_failure("section_token_count");
                None
            },
            Some,
        )
    }
}

fn local_table_requires_in_context(name: &str) -> bool {
    !matches!(name, "conversation history" | "reports" | "sources")
}

fn add_entity_filters<'a>(
    all_entities: &'a [Entity],
    index: &QueryDataIndex,
    matched: Vec<&'a Entity>,
    include_entity_names: &[String],
    exclude_entity_names: &[String],
) -> Vec<&'a Entity> {
    let mut result = Vec::new();
    for name in include_entity_names {
        result.extend(
            index
                .entity_by_title
                .get(name)
                .into_iter()
                .flatten()
                .filter_map(|position| all_entities.get(*position)),
        );
    }
    result.extend(matched.into_iter().filter(|entity| {
        !exclude_entity_names
            .iter()
            .any(|name| name == &entity.title)
    }));
    result
}

fn included_entities<'a>(
    all_entities: &'a [Entity],
    index: &QueryDataIndex,
    include_entity_names: &[String],
) -> Vec<&'a Entity> {
    include_entity_names
        .iter()
        .flat_map(|name| index.entity_by_title.get(name).into_iter().flatten())
        .filter_map(|position| all_entities.get(*position))
        .collect()
}

fn entity_candidate(entity: &Entity) -> ExplainabilityCandidate {
    let mut candidate =
        ExplainabilityCandidate::new(entity.id.clone(), ExplainabilityRecordType::Entity);
    candidate.short_id.clone_from(&entity.short_id);
    candidate.title = Some(entity.title.clone());
    candidate
}

fn raw_ann_candidate(
    session: &QueryExplainabilitySession,
    id: &str,
    score: f32,
    index: usize,
) -> Option<ExplainabilityCandidate> {
    let rank = u32::try_from(index.saturating_add(1)).map_or_else(
        |_| {
            session.mark_sidecar_failure("candidate_rank_conversion");
            None
        },
        Some,
    )?;
    let score = ExplainabilityScore::try_from(f64::from(score)).map_or_else(
        |_| {
            session.mark_sidecar_failure("candidate_score");
            None
        },
        Some,
    )?;
    let mut candidate =
        ExplainabilityCandidate::new(id.to_owned(), ExplainabilityRecordType::Entity);
    candidate.score = Some(score);
    candidate.rank = Some(rank);
    candidate.reason = Some(SelectionReason::AnnResult);
    Some(candidate)
}

fn entity_candidate_with_rank(
    session: &QueryExplainabilitySession,
    entity: &Entity,
    index: usize,
    score: Option<f32>,
    reason: Option<SelectionReason>,
    selected: bool,
) -> Option<ExplainabilityCandidate> {
    let rank = u32::try_from(index.saturating_add(1)).map_or_else(
        |_| {
            session.mark_sidecar_failure("candidate_rank_conversion");
            None
        },
        Some,
    )?;
    let score = match score {
        Some(score) => Some(ExplainabilityScore::try_from(f64::from(score)).map_or_else(
            |_| {
                session.mark_sidecar_failure("candidate_score");
                None
            },
            Some,
        )?),
        None => None,
    };
    let mut candidate = entity_candidate(entity);
    candidate.score = score;
    candidate.rank = Some(rank);
    candidate.selected = selected;
    candidate.reason = reason;
    Some(candidate)
}

fn community_report_candidate(report: &CommunityReport) -> ExplainabilityCandidate {
    let mut candidate =
        ExplainabilityCandidate::new(report.id.clone(), ExplainabilityRecordType::CommunityReport);
    candidate.short_id = Some(report.short_id.clone());
    candidate.title = Some(report.title.clone());
    candidate
}

fn relationship_candidate(relationship: &Relationship) -> ExplainabilityCandidate {
    let mut candidate = ExplainabilityCandidate::new(
        relationship.id.clone(),
        ExplainabilityRecordType::Relationship,
    );
    candidate.short_id.clone_from(&relationship.short_id);
    candidate
}

fn covariate_candidate(covariate: &Covariate) -> ExplainabilityCandidate {
    let mut candidate =
        ExplainabilityCandidate::new(covariate.id.clone(), ExplainabilityRecordType::Covariate);
    candidate.short_id.clone_from(&covariate.short_id);
    candidate
}

fn text_unit_candidate(text_unit: &TextUnit) -> ExplainabilityCandidate {
    let mut candidate =
        ExplainabilityCandidate::new(text_unit.id.clone(), ExplainabilityRecordType::TextUnit);
    candidate.short_id = Some(text_unit.short_id.clone());
    candidate
}

fn empty_section_explainability(
    kind: ContextSectionKind,
    name: Option<String>,
    token_budget: usize,
) -> SectionExplainability {
    SectionExplainability {
        kind,
        name,
        token_budget,
        tokens_used: 0,
        candidate_count: 0,
        selected_count: 0,
        selected_record_ids: Vec::new(),
        truncated: false,
        candidates: Vec::new(),
    }
}

fn empty_community_section(
    max_tokens: usize,
    candidates: Option<Vec<ExplainabilityCandidate>>,
    tokens_used: Option<usize>,
) -> Section {
    Section {
        text: "[]".to_owned(),
        table: ContextTable::new(["id", "title", "content"], Vec::new()),
        tokens_used: tokens_used.unwrap_or(0),
        explainability: candidates
            .zip(tokens_used)
            .map(|(candidates, tokens_used)| SectionExplainability {
                kind: ContextSectionKind::CommunityReports,
                name: None,
                token_budget: max_tokens,
                tokens_used,
                candidate_count: 0,
                selected_count: 0,
                selected_record_ids: Vec::new(),
                truncated: false,
                candidates,
            }),
    }
}

fn rank_filtered_relationship_section(
    selection: &RelationshipSelection<'_>,
    max_tokens: usize,
) -> Section {
    let candidates = selection
        .rank_filtered
        .iter()
        .map(|ranked| {
            let mut candidate = relationship_candidate(ranked.relationship);
            candidate.selected = false;
            candidate.reason = Some(SelectionReason::RankThreshold);
            candidate
        })
        .collect();
    Section {
        text: String::new(),
        table: ContextTable::new(
            ["id", "source", "target", "description", "weight"],
            Vec::new(),
        ),
        tokens_used: 0,
        explainability: Some(SectionExplainability {
            kind: ContextSectionKind::Relationships,
            name: None,
            token_budget: max_tokens,
            tokens_used: 0,
            candidate_count: 0,
            selected_count: 0,
            selected_record_ids: Vec::new(),
            truncated: false,
            candidates,
        }),
    }
}

fn missing_source_section(candidates: Vec<ExplainabilityCandidate>, max_tokens: usize) -> Section {
    Section {
        text: String::new(),
        table: ContextTable::new(["id", "text"], Vec::new()),
        tokens_used: 0,
        explainability: Some(SectionExplainability {
            kind: ContextSectionKind::Sources,
            name: None,
            token_budget: max_tokens,
            tokens_used: 0,
            candidate_count: 0,
            selected_count: 0,
            selected_record_ids: Vec::new(),
            truncated: false,
            candidates,
        }),
    }
}

fn build_record_section_explainability<T>(
    spec: SectionExplainabilitySpec,
    fitted: &FittedTable,
    records: Vec<&T>,
    mut non_token_candidates: Vec<ExplainabilityCandidate>,
    candidate_from_record: fn(&T) -> ExplainabilityCandidate,
) -> SectionExplainability {
    let mut candidates =
        Vec::with_capacity(records.len().saturating_add(non_token_candidates.len()));
    let mut selected_record_ids = Vec::with_capacity(fitted.selected_count);
    for (index, record) in records.into_iter().enumerate() {
        let mut candidate = candidate_from_record(record);
        if index < fitted.selected_count {
            candidate.selected = true;
            candidate.reason = Some(spec.selected_reason);
            selected_record_ids.push(candidate.id.clone());
        } else {
            candidate.selected = false;
            candidate.reason = Some(SelectionReason::TokenBudget);
        }
        candidates.push(candidate);
    }
    candidates.append(&mut non_token_candidates);
    SectionExplainability {
        kind: spec.kind,
        name: spec.name,
        token_budget: spec.token_budget,
        tokens_used: fitted.tokens_used,
        candidate_count: fitted.candidate_count,
        selected_count: fitted.selected_count,
        selected_record_ids,
        truncated: fitted.selected_count < fitted.candidate_count,
        candidates,
    }
}

fn rollback_section_explainability(
    attempted: Vec<SectionExplainability>,
    accepted: &[SectionExplainability],
) -> Vec<SectionExplainability> {
    let mut rolled_back = Vec::with_capacity(attempted.len().max(accepted.len()));
    for attempted_section in attempted {
        let accepted_section = accepted.iter().find(|section| {
            section.kind == attempted_section.kind && section.name == attempted_section.name
        });
        rolled_back.push(rollback_one_section(attempted_section, accepted_section));
    }
    for accepted_section in accepted {
        if !rolled_back.iter().any(|section| {
            section.kind == accepted_section.kind && section.name == accepted_section.name
        }) {
            rolled_back.push(accepted_section.clone());
        }
    }
    rolled_back
}

fn rollback_one_section(
    attempted: SectionExplainability,
    accepted: Option<&SectionExplainability>,
) -> SectionExplainability {
    let mut section = accepted.cloned().unwrap_or(SectionExplainability {
        kind: attempted.kind,
        name: attempted.name.clone(),
        token_budget: attempted.token_budget,
        tokens_used: 0,
        candidate_count: 0,
        selected_count: 0,
        selected_record_ids: Vec::new(),
        truncated: false,
        candidates: Vec::new(),
    });
    let accepted_candidate_count = section.candidates.len();
    let mut matched_accepted = vec![false; accepted_candidate_count];
    let mut rollback_candidates = 0_usize;
    for mut candidate in attempted.candidates {
        let accepted_occurrence = section
            .candidates
            .iter()
            .take(accepted_candidate_count)
            .enumerate()
            .position(|(index, accepted_candidate)| {
                !matched_accepted[index] && accepted_candidate.id == candidate.id
            });
        if let Some(index) = accepted_occurrence {
            matched_accepted[index] = true;
            continue;
        }
        if !matches!(
            candidate.reason,
            Some(SelectionReason::RankThreshold | SelectionReason::MissingRecord)
        ) {
            candidate.selected = false;
            candidate.reason = Some(SelectionReason::TokenBudget);
            rollback_candidates = rollback_candidates.saturating_add(1);
        }
        section.candidates.push(candidate);
    }
    section.candidate_count = section
        .candidates
        .iter()
        .filter(|candidate| {
            !matches!(
                candidate.reason,
                Some(SelectionReason::RankThreshold | SelectionReason::MissingRecord)
            )
        })
        .count();
    section.selected_record_ids = section
        .candidates
        .iter()
        .filter(|candidate| candidate.selected)
        .map(|candidate| candidate.id.clone())
        .collect();
    section.selected_count = section.selected_record_ids.len();
    section.truncated = section.truncated || rollback_candidates > 0;
    section
}

#[cfg(test)]
fn filter_relationships<'a>(
    selected_entities: &[&Entity],
    relationships: &'a [Relationship],
    relationship_positions: &BTreeSet<usize>,
    top_k_relationships: usize,
    learned_links: &mut BTreeMap<String, usize>,
) -> Vec<RankedRelationship<'a>> {
    filter_relationships_capture(
        selected_entities,
        relationships,
        relationship_positions,
        top_k_relationships,
        learned_links,
        false,
    )
    .selected
}

fn filter_relationships_capture<'a>(
    selected_entities: &[&Entity],
    relationships: &'a [Relationship],
    relationship_positions: &BTreeSet<usize>,
    top_k_relationships: usize,
    learned_links: &mut BTreeMap<String, usize>,
    capture_rank_filtered: bool,
) -> RelationshipSelection<'a> {
    let selected_names = selected_entities
        .iter()
        .map(|entity| entity.title.as_str())
        .collect::<BTreeSet<_>>();
    let candidates = relationship_positions
        .iter()
        .filter_map(|position| relationships.get(*position))
        .collect::<Vec<_>>();
    let mut in_network = candidates
        .iter()
        .copied()
        .filter(|relationship| {
            selected_names.contains(relationship.source.as_str())
                && selected_names.contains(relationship.target.as_str())
        })
        .map(|relationship| RankedRelationship {
            relationship,
            links: learned_links.get(&relationship.id).copied(),
        })
        .collect::<Vec<_>>();
    in_network.sort_by(rank_relationships);

    let mut out_network = candidates
        .iter()
        .copied()
        .filter(|relationship| {
            selected_names.contains(relationship.source.as_str())
                && !selected_names.contains(relationship.target.as_str())
        })
        .chain(candidates.iter().copied().filter(|relationship| {
            selected_names.contains(relationship.target.as_str())
                && !selected_names.contains(relationship.source.as_str())
        }))
        .map(|relationship| RankedRelationship {
            relationship,
            links: None,
        })
        .collect::<Vec<_>>();
    out_network.sort_by(rank_relationships);
    let mut rank_filtered = Vec::new();
    if out_network.len() > 1 {
        let mut neighbors_by_outside = HashMap::<&str, BTreeSet<&str>>::new();
        for ranked in &out_network {
            let relationship = ranked.relationship;
            let (outside, neighbor) = if selected_names.contains(relationship.source.as_str()) {
                (relationship.target.as_str(), relationship.source.as_str())
            } else {
                (relationship.source.as_str(), relationship.target.as_str())
            };
            neighbors_by_outside
                .entry(outside)
                .or_default()
                .insert(neighbor);
        }
        for ranked in &mut out_network {
            let outside = if selected_names.contains(ranked.relationship.source.as_str()) {
                ranked.relationship.target.as_str()
            } else {
                ranked.relationship.source.as_str()
            };
            ranked.links = neighbors_by_outside.get(outside).map(BTreeSet::len);
            if let Some(link_count) = ranked.links {
                learned_links.insert(ranked.relationship.id.clone(), link_count);
            }
        }
        out_network.sort_by(|left, right| {
            right
                .links
                .unwrap_or_default()
                .cmp(&left.links.unwrap_or_default())
                .then_with(|| rank_relationships(left, right))
        });
        let budget = top_k_relationships.saturating_mul(selected_entities.len());
        if capture_rank_filtered && budget < out_network.len() {
            rank_filtered.extend(out_network.get(budget..).into_iter().flatten().copied());
        }
        out_network.truncate(budget);
    }
    in_network.extend(out_network);
    RelationshipSelection {
        selected: in_network,
        rank_filtered,
    }
}

fn community_matches(selected_entities: &[&Entity]) -> Vec<(String, usize)> {
    let mut matches = Vec::<(String, usize)>::new();
    for entity in selected_entities {
        for community_id in &entity.community_ids {
            if let Some((_, count)) = matches
                .iter_mut()
                .find(|(candidate, _)| candidate == community_id)
            {
                *count = count.saturating_add(1);
            } else {
                matches.push((community_id.clone(), 1));
            }
        }
    }
    matches
}

fn rank_relationships(left: &RankedRelationship<'_>, right: &RankedRelationship<'_>) -> Ordering {
    right
        .relationship
        .rank
        .unwrap_or_default()
        .cmp(&left.relationship.rank.unwrap_or_default())
}

fn count_relationships(entity_relationships: &[&Relationship], text_unit: &TextUnit) -> usize {
    if text_unit.relationship_ids.is_empty() {
        entity_relationships
            .iter()
            .filter(|relationship| {
                relationship
                    .text_unit_ids
                    .iter()
                    .any(|id| id == &text_unit.id)
            })
            .count()
    } else {
        let relationship_ids = entity_relationships
            .iter()
            .map(|relationship| relationship.id.as_str())
            .collect::<BTreeSet<_>>();
        text_unit
            .relationship_ids
            .iter()
            .filter(|id| relationship_ids.contains(id.as_str()))
            .count()
    }
}

fn covariate_columns() -> [&'static str; 7] {
    [
        "id",
        "entity",
        "object_id",
        "status",
        "start_date",
        "end_date",
        "description",
    ]
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "GraphRAG uses Python int(positive_float), whose observable behavior is truncation"
)]
fn proportion(total: usize, value: f64) -> usize {
    (total as f64 * value) as usize
}

fn python_optional_i64(value: Option<i64>) -> String {
    value.map_or_else(|| "None".to_owned(), |number| number.to_string())
}

fn python_optional_f64_truthy(value: Option<f64>) -> String {
    match value {
        Some(number) if number != 0.0 => python_f64(number),
        _ => String::new(),
    }
}

fn python_f64(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering as AtomicOrdering},
        },
        time::Instant,
    };

    use async_trait::async_trait;
    use graphloom_llm::{EmbeddingResponse, EmbeddingUsage, LlmError};
    use graphloom_vectors::{VectorDocument, VectorSearchResult};
    use polars_core::prelude::DataType;

    use super::*;
    use crate::{
        explainability::{
            ExplainabilityContentMode, ExplainabilityRecord, ExplainabilityRunId,
            ExplainabilitySink, ExplainabilitySinkChain, ExplainabilitySinkError,
            NoopExplainabilitySink,
        },
        query::{ConversationRole, ConversationTurn, QueryExplainabilityOptions},
    };

    type RecordedSearches = Arc<Mutex<Vec<(Vec<f32>, usize, bool)>>>;
    const LOCAL_CONTEXT_GOLDEN: &str =
        include_str!("../../../../../tests/compat/fixtures/query/local_context.txt");
    const LOCAL_SPECIAL_CHARACTERS_GOLDEN: &str =
        include_str!("../../../../../tests/compat/fixtures/query/local_special_characters.json");
    const REPORT_CSV_GOLDEN: &str = include_str!(
        "../../../../../tests/compat/fixtures/query/report_csv_special_characters.json"
    );

    #[derive(Debug, Default)]
    struct ByteTokenizer;

    impl Tokenizer for ByteTokenizer {
        fn encode(&self, text: &str) -> graphloom_llm::Result<Vec<u32>> {
            Ok(text.bytes().map(u32::from).collect())
        }

        fn decode(&self, tokens: &[u32]) -> graphloom_llm::Result<String> {
            let bytes = tokens
                .iter()
                .map(|token| {
                    u8::try_from(*token).map_err(|source| LlmError::Tokenizer {
                        encoding_model: "bytes".to_owned(),
                        message: source.to_string(),
                    })
                })
                .collect::<graphloom_llm::Result<Vec<_>>>()?;
            String::from_utf8(bytes).map_err(|source| LlmError::Tokenizer {
                encoding_model: "bytes".to_owned(),
                message: source.to_string(),
            })
        }
    }

    #[derive(Debug)]
    struct CountingTokenizer {
        calls: Arc<AtomicUsize>,
    }

    impl Tokenizer for CountingTokenizer {
        fn encode(&self, text: &str) -> graphloom_llm::Result<Vec<u32>> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            ByteTokenizer.encode(text)
        }

        fn decode(&self, tokens: &[u32]) -> graphloom_llm::Result<String> {
            ByteTokenizer.decode(tokens)
        }
    }

    #[derive(Debug)]
    struct RecordingEmbedding {
        inputs: Arc<Mutex<Vec<Vec<String>>>>,
        prompt_tokens: u64,
    }

    #[async_trait]
    impl EmbeddingModel for RecordingEmbedding {
        async fn embed(
            &self,
            request: EmbeddingRequest,
        ) -> graphloom_llm::Result<EmbeddingResponse> {
            self.inputs
                .lock()
                .expect("recording embedding mutex")
                .push(request.input);
            let mut response =
                EmbeddingResponse::vectors_for_test("embedding", vec![vec![0.2, 0.8]]);
            response.usage = EmbeddingUsage {
                prompt_tokens: self.prompt_tokens,
                total_tokens: self.prompt_tokens,
                extra: BTreeMap::new(),
            };
            Ok(response)
        }
    }

    #[derive(Debug)]
    struct RecordingStore {
        results: Vec<VectorSearchResult>,
        searches: RecordedSearches,
        missing: bool,
        invalid: bool,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl VectorStore for RecordingStore {
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
            schema: &VectorIndexSchema,
            query_vector: &[f32],
            k: usize,
            include_vectors: bool,
        ) -> graphloom_vectors::Result<Vec<VectorSearchResult>> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            self.searches.lock().expect("recording vector mutex").push((
                query_vector.to_vec(),
                k,
                include_vectors,
            ));
            if self.missing {
                return Err(VectorError::MissingIndex {
                    index_name: schema.index_name.clone(),
                });
            }
            if self.invalid {
                return Err(VectorError::InvalidQuery {
                    index_name: schema.index_name.clone(),
                    message: "query vector dimension 2 does not match index dimension 3".to_owned(),
                });
            }
            Ok(self.results.clone())
        }
    }

    struct Fixture {
        builder: LocalContextBuilder,
        embedding_inputs: Arc<Mutex<Vec<Vec<String>>>>,
        searches: RecordedSearches,
    }

    #[derive(Debug, Default)]
    struct RecordingExplainabilitySink {
        records: Mutex<Vec<Arc<ExplainabilityRecord>>>,
        fail_emit: bool,
    }

    #[async_trait]
    impl ExplainabilitySink for RecordingExplainabilitySink {
        async fn emit(
            &self,
            record: Arc<ExplainabilityRecord>,
        ) -> std::result::Result<(), ExplainabilitySinkError> {
            self.records
                .lock()
                .expect("Explainability records")
                .push(record);
            if self.fail_emit {
                Err(ExplainabilitySinkError::RecordNotAccepted)
            } else {
                Ok(())
            }
        }

        async fn finish_run(
            &self,
            _run_id: &ExplainabilityRunId,
        ) -> std::result::Result<(), ExplainabilitySinkError> {
            Ok(())
        }
    }

    fn fixture(max_context_tokens: usize, ann_ids: &[&str]) -> Fixture {
        let embedding_inputs = Arc::new(Mutex::new(Vec::new()));
        let searches = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let entities = vec![
            entity(
                "entity-a",
                "0",
                "Alice",
                5,
                &["1", "2"],
                &["tu-a", "missing"],
            ),
            entity("entity-b", "1", "Bob", 4, &["2"], &["tu-b", "tu-shared"]),
            entity("entity-c", "2", "Carol", 3, &["3"], &["tu-c", "tu-shared"]),
        ];
        let results = ann_ids
            .iter()
            .map(|id| VectorSearchResult {
                document: VectorDocument {
                    id: (*id).to_owned(),
                    vector: Vec::new(),
                },
                score: 1.0,
            })
            .collect();
        let reports = vec![
            report("1", 8.0, "Alpha report"),
            report("2", 5.0, "Shared report"),
            report("3", 9.0, "Carol report"),
        ];
        let text_units = vec![
            text_unit("tu-a", "0", "Alice source", &["rel-ab", "rel-ax"]),
            text_unit("tu-b", "1", "Bob source", &["rel-ab"]),
            text_unit("tu-c", "2", "Carol source", &[]),
            text_unit("tu-shared", "3", "Shared source", &["rel-ab"]),
        ];
        let relationships = vec![
            relationship("rel-ab", "0", "Alice", "Bob", 9, 1.5, &["tu-a", "tu-b"]),
            relationship("rel-ax", "1", "Alice", "External", 7, 0.0, &["tu-a"]),
            relationship("rel-bx", "2", "Bob", "External", 6, 2.0, &[]),
            relationship("rel-ay", "3", "Alice", "Other", 8, 3.0, &[]),
        ];
        let covariates = vec![
            covariate("claim-1", "10", "Alice", "claims", "Alice claim"),
            covariate("fact-1", "11", "Bob", "facts", "Bob fact"),
        ];
        let index = Arc::new(QueryDataIndex::new(
            &entities,
            &reports,
            &text_units,
            &relationships,
            &covariates,
        ));
        Fixture {
            builder: LocalContextBuilder {
                method: SearchMethod::Local,
                config: LocalSearchConfig {
                    max_context_tokens,
                    top_k_entities: 2,
                    top_k_relationships: 1,
                    community_prop: 0.2,
                    text_unit_prop: 0.3,
                    ..LocalSearchConfig::default()
                },
                entities,
                reports,
                text_units,
                relationships,
                covariates,
                index,
                embedding_model: Arc::new(RecordingEmbedding {
                    inputs: Arc::clone(&embedding_inputs),
                    prompt_tokens: 7,
                }),
                embedding_model_id: "embedding".to_owned(),
                embedding_provider: "openai".to_owned(),
                vector_store: Arc::new(RecordingStore {
                    results,
                    searches: Arc::clone(&searches),
                    missing: false,
                    invalid: false,
                    calls,
                }),
                vector_schema: VectorIndexSchema::for_embedding_name(
                    crate::ENTITY_DESCRIPTION_EMBEDDING,
                    2,
                ),
                tokenizer: Arc::new(ByteTokenizer),
            },
            embedding_inputs,
            searches,
        }
    }

    fn entity(
        id: &str,
        short_id: &str,
        title: &str,
        rank: i64,
        community_ids: &[&str],
        text_unit_ids: &[&str],
    ) -> Entity {
        Entity {
            id: id.to_owned(),
            short_id: Some(short_id.to_owned()),
            title: title.to_owned(),
            entity_type: Some("PERSON".to_owned()),
            description: Some(format!("{title} description")),
            community_ids: community_ids
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            text_unit_ids: text_unit_ids
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            rank: Some(rank),
        }
    }

    fn report(community_id: &str, rank: f64, content: &str) -> CommunityReport {
        CommunityReport {
            id: format!("report-{community_id}"),
            short_id: community_id.to_owned(),
            community_id: community_id.to_owned(),
            title: format!("Report {community_id}"),
            summary: format!("Summary {community_id}"),
            full_content: content.to_owned(),
            rank: Some(rank),
            full_content_embedding: None,
        }
    }

    fn relationship(
        id: &str,
        short_id: &str,
        source: &str,
        target: &str,
        rank: i64,
        weight: f64,
        text_unit_ids: &[&str],
    ) -> Relationship {
        Relationship {
            id: id.to_owned(),
            short_id: Some(short_id.to_owned()),
            source: source.to_owned(),
            target: target.to_owned(),
            description: Some(format!("{source} to {target}")),
            weight: Some(weight),
            rank: Some(rank),
            text_unit_ids: text_unit_ids
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    }

    fn text_unit(id: &str, short_id: &str, text: &str, relationship_ids: &[&str]) -> TextUnit {
        TextUnit {
            id: id.to_owned(),
            short_id: short_id.to_owned(),
            text: text.to_owned(),
            entity_ids: Vec::new(),
            relationship_ids: relationship_ids
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            covariate_ids: Vec::new(),
            n_tokens: None,
            document_id: None,
        }
    }

    fn covariate(
        id: &str,
        short_id: &str,
        subject: &str,
        covariate_type: &str,
        description: &str,
    ) -> Covariate {
        Covariate {
            id: id.to_owned(),
            short_id: Some(short_id.to_owned()),
            subject_id: subject.to_owned(),
            covariate_type: covariate_type.to_owned(),
            object_id: None,
            status: Some("TRUE".to_owned()),
            start_date: None,
            end_date: None,
            description: Some(description.to_owned()),
        }
    }

    fn history() -> ConversationHistory {
        ConversationHistory {
            turns: vec![
                ConversationTurn {
                    role: ConversationRole::User,
                    content: "old question".to_owned(),
                },
                ConversationTurn {
                    role: ConversationRole::Assistant,
                    content: "old answer".to_owned(),
                },
                ConversationTurn {
                    role: ConversationRole::User,
                    content: "new question".to_owned(),
                },
                ConversationTurn {
                    role: ConversationRole::Assistant,
                    content: "new answer".to_owned(),
                },
            ],
        }
    }

    #[tokio::test]
    async fn test_should_map_by_entity_id_preserve_ann_order_and_oversample() {
        let fixture = fixture(20_000, &["entity-b", "stale", "entity-a"]);

        let (selected, usage) = fixture
            .builder
            .map_entities("question", &[], &[], None, None)
            .await
            .expect("entity mapping");

        assert_eq!(
            selected
                .iter()
                .map(|entity| entity.id.as_str())
                .collect::<Vec<_>>(),
            ["entity-b", "entity-a"]
        );
        assert_eq!(usage.llm_calls, 1);
        assert_eq!(usage.prompt_tokens, 7);
        assert_eq!(
            *fixture.searches.lock().expect("searches"),
            vec![(vec![0.2, 0.8], 4, false)]
        );
    }

    #[tokio::test]
    async fn test_should_fallback_to_tokenizer_for_zero_local_embedding_usage() {
        let mut fixture = fixture(20_000, &["entity-a"]);
        fixture.builder.embedding_model = Arc::new(RecordingEmbedding {
            inputs: Arc::clone(&fixture.embedding_inputs),
            prompt_tokens: 0,
        });

        let (_, usage) = fixture
            .builder
            .map_entities("zero usage", &[], &[], None, None)
            .await
            .expect("entity mapping");

        assert_eq!(
            usage,
            QueryUsageCategory {
                llm_calls: 1,
                prompt_tokens: "zero usage".len(),
                output_tokens: 0,
            }
        );
    }

    #[tokio::test]
    async fn test_should_match_dashed_ann_uuid_to_undashed_entity_id() {
        let dashed = "550e8400-e29b-41d4-a716-446655440000";
        let mut fixture = fixture(20_000, &[dashed]);
        fixture.builder.entities[0].id = dashed.replace('-', "");
        fixture.builder.index = Arc::new(QueryDataIndex::new(
            &fixture.builder.entities,
            &fixture.builder.reports,
            &fixture.builder.text_units,
            &fixture.builder.relationships,
            &fixture.builder.covariates,
        ));

        let (selected, _) = fixture
            .builder
            .map_entities("question", &[], &[], None, None)
            .await
            .expect("canonical UUID mapping");

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].title, "Alice");
    }

    #[tokio::test]
    async fn test_should_prepend_includes_and_filter_excludes_like_graphrag() {
        let fixture = fixture(20_000, &["entity-b", "entity-a", "entity-c"]);
        let include = vec!["Carol".to_owned()];
        let exclude = vec!["Bob".to_owned()];

        let (selected, _) = fixture
            .builder
            .map_entities("question", &include, &exclude, None, None)
            .await
            .expect("entity filters");

        assert_eq!(
            selected
                .iter()
                .map(|entity| entity.title.as_str())
                .collect::<Vec<_>>(),
            ["Carol", "Alice", "Carol"]
        );

        let recording = Arc::new(RecordingExplainabilitySink::default());
        let failing = Arc::new(RecordingExplainabilitySink {
            fail_emit: true,
            ..RecordingExplainabilitySink::default()
        });
        let chain_success = Arc::new(RecordingExplainabilitySink::default());
        let chain_failure = Arc::new(RecordingExplainabilitySink {
            fail_emit: true,
            ..RecordingExplainabilitySink::default()
        });
        let sinks: Vec<Arc<dyn ExplainabilitySink>> = vec![
            Arc::new(NoopExplainabilitySink::new()),
            recording.clone(),
            failing,
            Arc::new(ExplainabilitySinkChain::new(vec![
                chain_success,
                chain_failure,
            ])),
        ];
        for sink in sinks {
            let options =
                QueryExplainabilityOptions::generated(ExplainabilityContentMode::Metadata, sink);
            let session = QueryExplainabilitySession::new(&options);
            let (explained, _) = fixture
                .builder
                .map_entities("question", &include, &exclude, Some(&session), None)
                .await
                .expect("explained entity filters");
            assert_eq!(
                explained
                    .iter()
                    .map(|entity| entity.title.as_str())
                    .collect::<Vec<_>>(),
                ["Carol", "Alice", "Carol"]
            );
        }
        let records = recording.records.lock().expect("Explainability records");
        let filtered = records.iter().find_map(|record| match &record.event {
            ExplainabilityEvent::CandidatesFiltered(event) => Some(event.candidates()),
            _ => None,
        });
        assert!(filtered.is_some_and(|candidates| {
            candidates
                .iter()
                .map(|candidate| {
                    (
                        candidate.title.as_deref(),
                        candidate.selected,
                        candidate.reason,
                    )
                })
                .collect::<Vec<_>>()
                == [
                    (
                        Some("Carol"),
                        true,
                        Some(SelectionReason::ExplicitlyIncluded),
                    ),
                    (
                        Some("Bob"),
                        false,
                        Some(SelectionReason::ExplicitlyExcluded),
                    ),
                    (Some("Alice"), true, Some(SelectionReason::AnnResult)),
                    (Some("Carol"), true, Some(SelectionReason::AnnResult)),
                ]
        }));
        assert_eq!(
            *fixture.embedding_inputs.lock().expect("embedding inputs"),
            vec![vec!["question".to_owned()]; 5]
        );
        let searches = fixture.searches.lock().expect("recorded searches");
        assert_eq!(searches.len(), 5);
        assert!(
            searches
                .first()
                .is_some_and(|expected| searches.iter().all(|actual| actual == expected))
        );
    }

    #[test]
    fn test_should_retain_all_rank_filtered_relationship_decisions() {
        let mut fixture = fixture(20_000, &["entity-a"]);
        fixture.builder.config.top_k_relationships = 0;
        let selected_entities = vec![&fixture.builder.entities[0]];
        let relationship_positions = fixture
            .builder
            .index
            .relationships_by_entity
            .get("Alice")
            .into_iter()
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>();
        let options = QueryExplainabilityOptions::generated(
            ExplainabilityContentMode::Metadata,
            Arc::new(NoopExplainabilitySink::new()),
        );
        let session = QueryExplainabilitySession::new(&options);
        let mut learned_links = BTreeMap::new();

        let section = fixture
            .builder
            .build_relationship_context_from_positions_capture(
                &selected_entities,
                &relationship_positions,
                20_000,
                &mut learned_links,
                Some(&session),
            )
            .expect("rank-filtered relationship capture");
        let Some(section) = section else {
            panic!("rank-filtered relationships must retain sidecar metadata");
        };
        assert!(section.text.is_empty());
        assert!(section.table.is_empty());
        let Some(capture) = section.explainability else {
            panic!("rank-filtered relationship capture must exist");
        };
        assert!(!capture.candidates.is_empty());
        assert!(capture.candidates.iter().all(|candidate| {
            !candidate.selected && candidate.reason == Some(SelectionReason::RankThreshold)
        }));
        assert!(!capture.truncated);
    }

    #[test]
    fn test_should_retain_missing_text_unit_decisions_without_context_changes() {
        let mut fixture = fixture(20_000, &["entity-a"]);
        fixture.builder.entities[0].text_unit_ids = vec!["missing-only".to_owned()];
        fixture.builder.text_units.clear();
        let selected_entities = vec![&fixture.builder.entities[0]];
        let options = QueryExplainabilityOptions::generated(
            ExplainabilityContentMode::Metadata,
            Arc::new(NoopExplainabilitySink::new()),
        );
        let session = QueryExplainabilitySession::new(&options);

        let section = fixture
            .builder
            .build_source_context_capture(&selected_entities, 20_000, Some(&session))
            .expect("missing source capture");
        let Some(section) = section else {
            panic!("missing text units must retain sidecar metadata");
        };
        assert!(section.text.is_empty());
        assert!(section.table.is_empty());
        let Some(capture) = section.explainability else {
            panic!("missing text-unit capture must exist");
        };
        assert_eq!(capture.candidates.len(), 1);
        assert!(capture.candidates.first().is_some_and(|candidate| {
            candidate.id == "missing-only"
                && !candidate.selected
                && candidate.reason == Some(SelectionReason::MissingRecord)
        }));
        assert!(!capture.truncated);
    }

    #[tokio::test]
    async fn test_should_build_mapping_query_and_user_only_history_in_upstream_orders() {
        let fixture = fixture(20_000, &["entity-a", "entity-b"]);
        let history = history();

        let built = fixture
            .builder
            .build("current", Some(&history))
            .await
            .expect("Local context");

        assert_eq!(
            *fixture.embedding_inputs.lock().expect("embedding inputs"),
            vec![vec!["current\nnew question\nold question".to_owned()]]
        );
        let QueryContextText::Text(text) = built.context.text else {
            panic!("expected text context");
        };
        assert_eq!(text, LOCAL_CONTEXT_GOLDEN);
        assert!(text.starts_with(
            "-----Conversation History-----\nturn|content\nuser|old question\nuser|new \
             question\n\n"
        ));
        assert!(!text.contains("old answer"));
        assert!(!text.contains("new answer"));
    }

    #[tokio::test]
    async fn test_should_use_recent_history_for_mapping_but_oldest_history_for_context_limit() {
        let mut fixture = fixture(20_000, &["entity-a"]);
        fixture.builder.config.conversation_history_max_turns = 1;

        let built = fixture
            .builder
            .build("current", Some(&history()))
            .await
            .expect("limited history");

        assert_eq!(
            *fixture.embedding_inputs.lock().expect("embedding inputs"),
            vec![vec!["current\nnew question".to_owned()]]
        );
        let QueryContextText::Text(text) = built.context.text else {
            panic!("expected text context");
        };
        assert!(text.contains("user|old question\n"));
        assert!(!text.contains("user|new question\n"));
    }

    #[test]
    fn test_should_render_empty_community_list_and_stop_at_record_boundary() {
        let mut fixture = fixture(20_000, &[]);
        fixture
            .builder
            .reports
            .retain(|report| report.community_id != "3");
        let carol = vec![&fixture.builder.entities[2]];
        let missing = fixture
            .builder
            .build_community_context(&carol, 20_000)
            .expect("missing report context")
            .expect("upstream empty community list");
        assert_eq!(missing.text, "[]");
        assert!(missing.table.is_empty());

        let alice = vec![&fixture.builder.entities[0]];
        let header = "-----Reports-----\nid|title|content\n";
        let first = "1|Report 1|Alpha report\n";
        let section = fixture
            .builder
            .build_community_context(&alice, header.len() + first.len())
            .expect("bounded reports")
            .expect("one report");
        assert_eq!(section.text, format!("{header}{first}"));
    }

    #[test]
    fn test_should_count_rendered_empty_community_tokens_for_explainability() {
        let mut fixture = fixture(20_000, &[]);
        fixture
            .builder
            .reports
            .retain(|report| report.community_id != "3");
        let options = QueryExplainabilityOptions::generated(
            ExplainabilityContentMode::Metadata,
            Arc::new(NoopExplainabilitySink::new()),
        );
        let session = QueryExplainabilitySession::new(&options);
        let carol = vec![&fixture.builder.entities[2]];

        let section = fixture
            .builder
            .build_community_context_capture(&carol, 20_000, Some(&session))
            .expect("empty community context")
            .expect("rendered empty community section");
        let expected_tokens = fixture
            .builder
            .tokenizer
            .count("[]")
            .expect("deterministic empty-list token count");

        assert_eq!(section.text, "[]");
        assert_eq!(
            section.explainability.map(|capture| capture.tokens_used),
            Some(expected_tokens)
        );
    }

    #[test]
    fn test_should_only_count_empty_community_sidecar_when_explainability_is_enabled() {
        let mut fixture = fixture(20_000, &[]);
        fixture
            .builder
            .reports
            .retain(|report| report.community_id != "3");
        let calls = Arc::new(AtomicUsize::new(0));
        fixture.builder.tokenizer = Arc::new(CountingTokenizer {
            calls: Arc::clone(&calls),
        });
        let carol = vec![&fixture.builder.entities[2]];

        let baseline = fixture
            .builder
            .build_community_context_capture(&carol, 20_000, None)
            .expect("baseline empty community context")
            .expect("baseline empty community section");
        assert_eq!(baseline.text, "[]");
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 0);

        let options = QueryExplainabilityOptions::generated(
            ExplainabilityContentMode::Metadata,
            Arc::new(NoopExplainabilitySink::new()),
        );
        let session = QueryExplainabilitySession::new(&options);
        let expected_tokens = ByteTokenizer
            .count("[]")
            .expect("deterministic empty-list token count");
        let explained = fixture
            .builder
            .build_community_context_capture(&carol, 20_000, Some(&session))
            .expect("explained empty community context")
            .expect("explained empty community section");

        assert_eq!(explained.text, baseline.text);
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(
            explained.explainability.map(|capture| capture.tokens_used),
            Some(expected_tokens)
        );
    }

    #[test]
    fn test_should_fit_local_reports_with_raw_rows_and_render_final_csv() {
        let mut fixture = fixture(20_000, &[]);
        let mut first = report("1", 4.0, "alpha|beta \"quoted\" \\path\nsecond line");
        first.short_id = "0".to_owned();
        first.title = "Report 0".to_owned();
        let mut second = report("2", 3.0, "plain second");
        second.short_id = "1".to_owned();
        second.title = "Report 1".to_owned();
        fixture.builder.reports = vec![first, second];
        let selected = vec![&fixture.builder.entities[0]];
        let golden = serde_json::from_str::<serde_json::Value>(REPORT_CSV_GOLDEN)
            .expect("report CSV golden");
        let budget = golden["local_report_budget"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .expect("Local report budget");

        let section = fixture
            .builder
            .build_community_context(&selected, budget)
            .expect("Local Reports")
            .expect("one fitted report");
        assert_eq!(
            section.text,
            golden["local_reports_context"]
                .as_str()
                .expect("Local Reports golden")
        );
        assert!(section.text.contains("\\path"));
        assert!(!section.text.contains("\\\\path"));
        let records = section
            .table
            .to_dataframe(SearchMethod::Local, "build Local Reports golden records")
            .expect("Local Reports records");
        assert_eq!(records.height(), 1);
        assert_eq!(
            records
                .column("id")
                .expect("Local report id")
                .str()
                .expect("Local report id strings")
                .get(0),
            Some("0")
        );
        let under_budget = fixture
            .builder
            .build_community_context(&selected, budget - 1)
            .expect("under-budget Local Reports")
            .expect("upstream empty community list");
        assert_eq!(under_budget.text, "[]");
        assert!(under_budget.table.is_empty());
    }

    #[tokio::test]
    async fn test_should_render_exact_mixed_context_order_headers_and_records() {
        let fixture = fixture(20_000, &["entity-a", "entity-b"]);

        let built = fixture
            .builder
            .build("question", None)
            .await
            .expect("Local context");

        let QueryContextText::Text(text) = &built.context.text else {
            panic!("expected text context");
        };
        let report = text.find("-----Reports-----").expect("reports");
        let entities = text.find("-----Entities-----").expect("entities");
        let relationships = text.find("-----Relationships-----").expect("relationships");
        let claims = text.find("-----claims-----").expect("claims");
        let facts = text.find("-----facts-----").expect("facts");
        let sources = text.find("-----Sources-----").expect("sources");
        assert!(report < entities);
        assert!(entities < relationships);
        assert!(relationships < claims);
        assert!(claims < facts);
        assert!(facts < sources);
        assert!(text.contains("id|title|content\n2|Report 2|Shared report\n"));
        assert!(text.contains(
            "id|entity|description|number of relationships\n0|Alice|Alice description|5\n"
        ));
        assert!(text.contains("id|source|target|description|weight|links\n"));
        assert!(text.contains(
            "id|entity|object_id|status|start_date|end_date|description\n10|Alice||TRUE|||Alice \
             claim\n"
        ));
        assert!(text.contains(
            "-----Sources-----\nid|text\n0|Alice source\n1|Bob source\n3|Shared source\n"
        ));
        let QueryContextRecords::Tables(records) = &built.context.records else {
            panic!("expected Local tables");
        };
        assert_eq!(
            records.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "claims",
                "entities",
                "facts",
                "relationships",
                "reports",
                "sources"
            ]
        );
    }

    #[test]
    fn test_should_group_all_runtime_covariates_under_claims() {
        let fixture = fixture(20_000, &[]);
        let index = QueryDataIndex::new_with_single_covariate_group(
            &fixture.builder.entities,
            &fixture.builder.reports,
            &fixture.builder.text_units,
            &fixture.builder.relationships,
            &fixture.builder.covariates,
            "claims",
        );

        let [(name, positions)] = index.covariate_groups.as_slice() else {
            panic!("expected one runtime covariate group");
        };
        assert_eq!(name, "claims");
        assert_eq!(positions.len(), fixture.builder.covariates.len());
        assert!((0..fixture.builder.covariates.len()).all(|index| positions.contains(&index)));
    }

    #[tokio::test]
    async fn test_should_preserve_upstream_empty_community_list_text() {
        let fixture = fixture(1, &["entity-a"]);

        let built = fixture
            .builder
            .build("question", None)
            .await
            .expect("Local context whose community reports exceed the budget");

        let QueryContextText::Text(text) = &built.context.text else {
            panic!("expected text context");
        };
        assert!(text.starts_with("[]\n\n-----Entities-----"));
        let QueryContextRecords::Tables(records) = &built.context.records else {
            panic!("expected Local tables");
        };
        assert_eq!(records["reports"].height(), 0);
    }

    #[tokio::test]
    async fn test_should_render_raw_local_special_characters_from_shared_golden() {
        let mut fixture = fixture(20_000, &["entity-a"]);
        fixture.builder.reports.clear();
        fixture.builder.entities[0].description =
            Some("Alice|Bob \"quoted\" \\path\nsecond line".to_owned());
        fixture.builder.entities[0].community_ids.clear();
        fixture.builder.entities[0].text_unit_ids = vec!["tu-a".to_owned()];
        fixture.builder.relationships.truncate(1);
        fixture.builder.relationships[0].description =
            Some("A|B \"rel\" \\edge\r\nnext".to_owned());
        fixture.builder.relationships[0].text_unit_ids = vec!["tu-a".to_owned()];
        fixture.builder.covariates.truncate(1);
        fixture.builder.covariates[0].description =
            Some("claim|text \"quoted\" \\claim\nnext".to_owned());
        fixture.builder.text_units.truncate(1);
        fixture.builder.text_units[0].text = "source|text \"quoted\" \\source\r\nnext".to_owned();
        fixture.builder.text_units[0].relationship_ids = vec!["rel-ab".to_owned()];
        fixture.builder.index = Arc::new(QueryDataIndex::new(
            &fixture.builder.entities,
            &fixture.builder.reports,
            &fixture.builder.text_units,
            &fixture.builder.relationships,
            &fixture.builder.covariates,
        ));

        let built = fixture
            .builder
            .build("question", None)
            .await
            .expect("special-character Local context");
        let QueryContextText::Text(text) = &built.context.text else {
            panic!("expected special-character text");
        };
        let golden = serde_json::from_str::<serde_json::Value>(LOCAL_SPECIAL_CHARACTERS_GOLDEN)
            .expect("special-character golden JSON");
        assert_eq!(text, golden["context"].as_str().expect("golden context"));
        let QueryContextRecords::Tables(records) = &built.context.records else {
            panic!("expected special-character records");
        };
        let golden_records = golden
            .get("records")
            .and_then(serde_json::Value::as_object)
            .expect("golden records");
        assert_eq!(records.len(), golden_records.len());
        for (name, snapshot) in golden_records {
            let frame = records.get(name).expect("golden record table");
            let columns = snapshot
                .get("columns")
                .and_then(serde_json::Value::as_array)
                .expect("golden columns")
                .iter()
                .map(|column| column.as_str().expect("golden column"))
                .collect::<Vec<_>>();
            let mut expected_columns = columns.clone();
            if local_table_requires_in_context(name) {
                expected_columns.push("in_context");
            }
            assert_eq!(
                frame
                    .get_column_names()
                    .iter()
                    .map(|column| column.as_str())
                    .collect::<Vec<_>>(),
                expected_columns
            );
            let rows = snapshot
                .get("rows")
                .and_then(serde_json::Value::as_array)
                .expect("golden rows");
            assert_eq!(frame.height(), rows.len());
            for (row_index, row) in rows.iter().enumerate() {
                let fields = row.as_array().expect("golden row");
                for (column, expected) in columns.iter().zip(fields) {
                    assert_eq!(
                        frame
                            .column(column)
                            .expect("record column")
                            .str()
                            .expect("record string column")
                            .get(row_index),
                        expected.as_str(),
                    );
                }
            }
            if local_table_requires_in_context(name) {
                let in_context = frame
                    .column("in_context")
                    .expect("in_context metadata")
                    .bool()
                    .expect("Boolean in_context");
                assert_eq!(in_context.len(), frame.height());
                assert!((0..in_context.len()).all(|index| in_context.get(index) == Some(true)));
            }
        }
    }

    #[tokio::test]
    async fn test_should_add_true_in_context_to_standard_local_metadata() {
        let fixture = fixture(20_000, &["entity-a", "entity-b"]);
        let built = fixture
            .builder
            .build("question", Some(&history()))
            .await
            .expect("standard Local context");
        let QueryContextText::Text(text) = &built.context.text else {
            panic!("expected standard Local text");
        };
        assert!(!text.contains("in_context"));
        let QueryContextRecords::Tables(records) = built.context.records else {
            panic!("expected standard Local records");
        };

        for (name, frame) in &records {
            let has_in_context = frame
                .get_column_names()
                .iter()
                .any(|column| column.as_str() == "in_context");
            if local_table_requires_in_context(name) {
                assert!(has_in_context, "{name} is missing in_context");
                let in_context = frame.column("in_context").expect("in_context column");
                assert_eq!(in_context.dtype(), &DataType::Boolean);
                let values = in_context.bool().expect("Boolean in_context");
                assert_eq!(values.len(), frame.height());
                assert!((0..values.len()).all(|index| values.get(index) == Some(true)));
            } else {
                assert!(!has_in_context, "{name} unexpectedly has in_context");
            }
        }
    }

    #[test]
    fn test_should_rank_in_network_before_mutual_out_network_and_keep_stable_ties() {
        let fixture = fixture(20_000, &[]);
        let selected = vec![&fixture.builder.entities[0], &fixture.builder.entities[1]];
        let positions = selected
            .iter()
            .filter_map(|entity| {
                fixture
                    .builder
                    .index
                    .relationships_by_entity
                    .get(entity.title.as_str())
            })
            .flatten()
            .copied()
            .collect();

        let ranked = filter_relationships(
            &selected,
            &fixture.builder.relationships,
            &positions,
            fixture.builder.config.top_k_relationships,
            &mut BTreeMap::new(),
        );

        assert_eq!(
            ranked
                .iter()
                .map(|value| value.relationship.id.as_str())
                .collect::<Vec<_>>(),
            ["rel-ab", "rel-ax", "rel-bx"]
        );
        assert_eq!(ranked[1].links, Some(2));
        assert_eq!(ranked[2].links, Some(2));
    }

    #[test]
    #[ignore = "repeatable large-context performance probe; run through make bench-query"]
    fn test_performance_should_scale_indexed_relationship_expansion() {
        for relationship_count in [10_000_usize, 100_000] {
            let mut fixture = fixture(usize::MAX, &[]);
            fixture.builder.relationships = (0..relationship_count)
                .map(|index| {
                    relationship(
                        &format!("large-{index}"),
                        &index.to_string(),
                        "Alice",
                        &format!("Outside-{index}"),
                        i64::try_from(index % 100).unwrap_or_default(),
                        1.0,
                        &[],
                    )
                })
                .collect();
            fixture.builder.index = Arc::new(QueryDataIndex::new(
                &fixture.builder.entities,
                &fixture.builder.reports,
                &fixture.builder.text_units,
                &fixture.builder.relationships,
                &fixture.builder.covariates,
            ));
            let selected = [&fixture.builder.entities[0], &fixture.builder.entities[1]];
            let started = Instant::now();
            let section = fixture
                .builder
                .build_local_context(&selected, usize::MAX)
                .expect("large relationship context");
            let elapsed = started.elapsed();
            assert!(section.tables.contains_key("relationships"));
            eprintln!(
                "local relationship performance: relationships={relationship_count}, \
                 elapsed={elapsed:?}"
            );
        }
    }

    #[test]
    fn test_should_restore_accepted_candidates_missing_from_failed_attempt() {
        let accepted = section_explainability_for_test(
            ContextSectionKind::Relationships,
            None,
            7,
            vec![candidate_for_test(
                "r1",
                true,
                SelectionReason::GraphExpansion,
            )],
        );
        let attempted = section_explainability_for_test(
            ContextSectionKind::Relationships,
            None,
            19,
            vec![
                candidate_for_test("r2", true, SelectionReason::GraphExpansion),
                candidate_for_test("r3", false, SelectionReason::RankThreshold),
            ],
        );

        let rolled_back = rollback_section_explainability(vec![attempted], &[accepted]);
        let [section] = rolled_back.as_slice() else {
            panic!("expected one relationship section");
        };

        assert_eq!(section.tokens_used, 7);
        assert_eq!(section.selected_count, 1);
        assert_eq!(section.selected_record_ids, ["r1"]);
        assert_eq!(
            section
                .candidates
                .iter()
                .map(|candidate| (candidate.id.as_str(), candidate.selected, candidate.reason))
                .collect::<Vec<_>>(),
            [
                ("r1", true, Some(SelectionReason::GraphExpansion)),
                ("r2", false, Some(SelectionReason::TokenBudget)),
                ("r3", false, Some(SelectionReason::RankThreshold)),
            ]
        );
    }

    #[test]
    fn test_should_preserve_duplicate_occurrences_and_covariate_groups_on_rollback() {
        let accepted = vec![
            section_explainability_for_test(
                ContextSectionKind::Relationships,
                None,
                5,
                vec![
                    candidate_for_test("r1", true, SelectionReason::GraphExpansion),
                    candidate_for_test("r1", true, SelectionReason::GraphExpansion),
                ],
            ),
            section_explainability_for_test(
                ContextSectionKind::Covariates,
                Some("claims".to_owned()),
                3,
                vec![candidate_for_test(
                    "claim-1",
                    true,
                    SelectionReason::GraphExpansion,
                )],
            ),
            section_explainability_for_test(
                ContextSectionKind::Covariates,
                Some("other".to_owned()),
                4,
                vec![candidate_for_test(
                    "other-1",
                    true,
                    SelectionReason::GraphExpansion,
                )],
            ),
        ];
        let attempted = vec![
            section_explainability_for_test(
                ContextSectionKind::Relationships,
                None,
                20,
                vec![
                    candidate_for_test("r1", true, SelectionReason::GraphExpansion),
                    candidate_for_test("r2", true, SelectionReason::GraphExpansion),
                ],
            ),
            section_explainability_for_test(
                ContextSectionKind::Covariates,
                Some("claims".to_owned()),
                20,
                vec![candidate_for_test(
                    "claim-2",
                    true,
                    SelectionReason::GraphExpansion,
                )],
            ),
            section_explainability_for_test(
                ContextSectionKind::Covariates,
                Some("other".to_owned()),
                20,
                vec![candidate_for_test(
                    "other-2",
                    true,
                    SelectionReason::GraphExpansion,
                )],
            ),
        ];

        let rolled_back = rollback_section_explainability(attempted, &accepted);

        assert_eq!(rolled_back[0].selected_record_ids, ["r1", "r1"]);
        assert_eq!(rolled_back[0].selected_count, 2);
        assert_eq!(rolled_back[0].candidates[2].id, "r2");
        assert!(!rolled_back[0].candidates[2].selected);
        assert_eq!(rolled_back[1].name.as_deref(), Some("claims"));
        assert_eq!(rolled_back[1].selected_record_ids, ["claim-1"]);
        assert_eq!(rolled_back[2].name.as_deref(), Some("other"));
        assert_eq!(rolled_back[2].selected_record_ids, ["other-1"]);
    }

    #[test]
    fn test_should_reject_all_selectable_candidates_when_first_attempt_rolls_back() {
        let attempted = section_explainability_for_test(
            ContextSectionKind::Relationships,
            None,
            11,
            vec![
                candidate_for_test("r1", true, SelectionReason::GraphExpansion),
                candidate_for_test("r2", false, SelectionReason::MissingRecord),
            ],
        );

        let rolled_back = rollback_section_explainability(vec![attempted], &[]);
        let [section] = rolled_back.as_slice() else {
            panic!("expected one relationship section");
        };

        assert_eq!(section.tokens_used, 0);
        assert_eq!(section.selected_count, 0);
        assert!(section.selected_record_ids.is_empty());
        assert!(section.truncated);
        assert_eq!(
            section.candidates[0].reason,
            Some(SelectionReason::TokenBudget)
        );
        assert_eq!(
            section.candidates[1].reason,
            Some(SelectionReason::MissingRecord)
        );
        assert!(
            section
                .candidates
                .iter()
                .all(|candidate| !candidate.selected)
        );
    }

    fn candidate_for_test(
        id: &str,
        selected: bool,
        reason: SelectionReason,
    ) -> ExplainabilityCandidate {
        let mut candidate =
            ExplainabilityCandidate::new(id.to_owned(), ExplainabilityRecordType::Relationship);
        candidate.selected = selected;
        candidate.reason = Some(reason);
        candidate
    }

    fn section_explainability_for_test(
        kind: ContextSectionKind,
        name: Option<String>,
        tokens_used: usize,
        candidates: Vec<ExplainabilityCandidate>,
    ) -> SectionExplainability {
        let selected_record_ids = candidates
            .iter()
            .filter(|candidate| candidate.selected)
            .map(|candidate| candidate.id.clone())
            .collect::<Vec<_>>();
        SectionExplainability {
            kind,
            name,
            token_budget: 20,
            tokens_used,
            candidate_count: candidates.len(),
            selected_count: selected_record_ids.len(),
            selected_record_ids,
            truncated: false,
            candidates,
        }
    }

    #[test]
    fn test_should_rollback_progressive_relationship_and_covariate_state() {
        let mut fixture = fixture(20_000, &[]);
        let selected = vec![&fixture.builder.entities[0], &fixture.builder.entities[1]];
        let entity_candidates = selected
            .iter()
            .map(|entity| {
                vec![
                    entity.short_id.clone().unwrap_or_default(),
                    entity.title.clone(),
                    entity.description.clone().unwrap_or_default(),
                    python_optional_i64(entity.rank),
                ]
            })
            .collect();
        let entity_table = fixture
            .builder
            .fit_delimited_rows(
                ContextTable::new(
                    ["id", "entity", "description", "number of relationships"],
                    Vec::new(),
                ),
                entity_candidates,
                "Entities",
                20_000,
                "test entities",
            )
            .expect("entity table");
        let entity_text = entity_table
            .table
            .render_delimited_section("Entities", SearchMethod::Local, "test entities")
            .expect("entity text");
        let relationship_positions = fixture.builder.index.relationships_by_entity["Alice"]
            .iter()
            .copied()
            .collect();
        let relationship = fixture
            .builder
            .build_relationship_context_from_positions(
                &selected[..1],
                &relationship_positions,
                20_000,
                &mut BTreeMap::new(),
            )
            .expect("relationship section")
            .expect("Alice relationships");
        let covariate_positions = fixture.builder.index.covariates_by_subject["Alice"].clone();
        let (claim_name, claim_group_positions) = fixture
            .builder
            .index
            .covariate_groups
            .first()
            .cloned()
            .expect("claims group");
        let claim = fixture
            .builder
            .build_covariate_context_from_positions(
                &claim_name,
                &claim_group_positions,
                &covariate_positions,
                20_000,
            )
            .expect("claim context")
            .expect("Alice claim");
        let (fact_name, fact_group_positions) = fixture
            .builder
            .index
            .covariate_groups
            .get(1)
            .cloned()
            .expect("facts group");
        let fact = fixture
            .builder
            .build_covariate_context_from_positions(
                &fact_name,
                &fact_group_positions,
                &covariate_positions,
                20_000,
            )
            .expect("fact context")
            .expect("header-only facts");
        let one_tokens = fixture
            .builder
            .count(&entity_text, "test entity count")
            .expect("entity tokens")
            .saturating_add(
                fixture
                    .builder
                    .count(&relationship.text, "test relationship count")
                    .expect("relationship tokens"),
            )
            .saturating_add(
                fixture
                    .builder
                    .count(&claim.text, "test claim count")
                    .expect("claim tokens"),
            )
            .saturating_add(
                fixture
                    .builder
                    .count(&fact.text, "test fact count")
                    .expect("fact tokens"),
            );
        drop(selected);
        fixture.builder.config.max_context_tokens = one_tokens;
        let selected = vec![&fixture.builder.entities[0], &fixture.builder.entities[1]];

        let rolled_back = fixture
            .builder
            .build_local_context(&selected, one_tokens)
            .expect("rolled back context");

        assert!(rolled_back.text.contains("Alice to Bob"));
        assert!(!rolled_back.text.contains("Bob to External"));
        assert!(!rolled_back.text.contains("Bob fact"));
    }

    #[test]
    fn test_should_stop_before_partial_source_record_at_exact_token_boundary() {
        let fixture = fixture(20_000, &[]);
        let selected = vec![&fixture.builder.entities[0]];
        let header = "-----Sources-----\nid|text\n";
        let first = "0|Alice source\n";
        let budget = header.len() + first.len();

        let section = fixture
            .builder
            .build_source_context(&selected, budget)
            .expect("sources")
            .expect("source section");

        assert_eq!(section.text, format!("{header}{first}"));
        assert_eq!(
            section
                .table
                .to_dataframe(SearchMethod::Local, "test Sources records")
                .expect("source dataframe")
                .height(),
            1
        );
    }

    #[test]
    fn test_should_fit_special_character_source_using_raw_rendered_tokens() {
        let mut fixture = fixture(20_000, &[]);
        fixture.builder.entities[0].text_unit_ids = vec!["tu-a".to_owned(), "tu-b".to_owned()];
        fixture.builder.text_units[0].text = "source|text \"quoted\" \\source\r\nnext".to_owned();
        let selected = vec![&fixture.builder.entities[0]];
        let expected = concat!(
            "-----Sources-----\n",
            "id|text\n",
            "0|source|text \"quoted\" \\source\r\nnext\n",
        );

        let section = fixture
            .builder
            .build_source_context(&selected, expected.len())
            .expect("special-character sources")
            .expect("one special-character source");

        assert_eq!(section.text, expected);
        assert_eq!(
            section
                .table
                .to_dataframe(SearchMethod::Local, "test special Sources records")
                .expect("special source dataframe")
                .height(),
            1
        );
    }

    #[tokio::test]
    async fn test_should_match_upstream_empty_frame_columns() {
        let mut fixture = fixture(20_000, &["entity-a"]);
        fixture.builder.relationships.clear();
        fixture.builder.covariates.clear();

        let built = fixture
            .builder
            .build("question", None)
            .await
            .expect("Local empty relationship context");
        let QueryContextRecords::Tables(records) = built.context.records else {
            panic!("expected records");
        };
        assert_eq!(records["relationships"].get_column_names(), ["in_context"]);
        assert_eq!(
            records["relationships"]
                .column("in_context")
                .expect("empty relationship metadata")
                .dtype(),
            &DataType::Boolean
        );

        let history = ConversationHistory {
            turns: vec![ConversationTurn {
                role: ConversationRole::User,
                content: "long question".to_owned(),
            }],
        };
        let history_context = history
            .build_user_context(
                &fixture.builder.tokenizer,
                5,
                "-----Conversation History-----\nturn|content\n".len(),
            )
            .expect("header-only history");
        assert_eq!(history_context.text, "-----Conversation History-----\n\n");
        assert!(history_context.table.is_empty());
        assert!(
            history_context
                .table
                .to_dataframe(SearchMethod::Local, "test empty history")
                .expect("empty history dataframe")
                .get_column_names()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn test_should_return_typed_missing_vector_error() {
        let mut fixture = fixture(20_000, &["entity-a"]);
        fixture.builder.vector_store = Arc::new(RecordingStore {
            results: Vec::new(),
            searches: Arc::new(Mutex::new(Vec::new())),
            missing: true,
            invalid: false,
            calls: Arc::new(AtomicUsize::new(0)),
        });

        let error = fixture
            .builder
            .map_entities("question", &[], &[], None, None)
            .await
            .expect_err("missing vector index");

        assert!(matches!(error, QueryError::MissingVectorIndex { .. }));
    }

    #[tokio::test]
    async fn test_should_propagate_embedding_dimension_error_as_invalid_vector_index() {
        let mut fixture = fixture(20_000, &["entity-a"]);
        fixture.builder.vector_store = Arc::new(RecordingStore {
            results: Vec::new(),
            searches: Arc::new(Mutex::new(Vec::new())),
            missing: false,
            invalid: true,
            calls: Arc::new(AtomicUsize::new(0)),
        });

        let error = fixture
            .builder
            .map_entities("question", &[], &[], None, None)
            .await
            .expect_err("dimension mismatch");

        assert!(matches!(error, QueryError::InvalidVectorIndex { .. }));
        assert!(error.to_string().contains("dimension 2"));
    }
}
