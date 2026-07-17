//! Local Search entity mapping and mixed-context construction.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use graphloom_llm::{EmbeddingModel, EmbeddingRequest, Tokenizer};
use graphloom_vectors::{VectorError, VectorIndexSchema, VectorStore};
use polars_core::prelude::{NamedFrom, Series};

use super::super::{
    CommunityReport, ConversationHistory, Covariate, Entity, QueryContext, QueryContextRecords,
    QueryContextText, QueryError, QueryUsageCategory, Relationship, Result, SearchMethod, TextUnit,
    context::ContextTable,
};
use crate::LocalSearchConfig;

/// Local Search context resources, independent of completion orchestration.
#[derive(Debug)]
pub(crate) struct LocalContextBuilder {
    pub(crate) config: LocalSearchConfig,
    pub(crate) entities: Vec<Entity>,
    pub(crate) reports: Vec<CommunityReport>,
    pub(crate) text_units: Vec<TextUnit>,
    pub(crate) relationships: Vec<Relationship>,
    pub(crate) covariates: Vec<Covariate>,
    pub(crate) embedding_model: Arc<dyn EmbeddingModel>,
    pub(crate) embedding_model_id: String,
    pub(crate) vector_store: Arc<dyn VectorStore>,
    pub(crate) vector_schema: VectorIndexSchema,
    pub(crate) tokenizer: Arc<dyn Tokenizer>,
}

/// Completed Local context and its embedding usage.
#[derive(Debug)]
pub(crate) struct LocalContextBuild {
    pub(crate) context: QueryContext,
    pub(crate) usage: QueryUsageCategory,
}

#[derive(Debug)]
struct Section {
    text: String,
    table: ContextTable,
}

#[derive(Debug)]
struct LocalSections {
    text: String,
    tables: BTreeMap<String, ContextTable>,
}

#[derive(Debug, Clone, Copy)]
struct RankedRelationship<'a> {
    relationship: &'a Relationship,
    links: Option<usize>,
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
        let mapping_query = conversation_history.map_or_else(
            || query.to_owned(),
            |history| history.mapping_query(query, self.config.conversation_history_max_turns),
        );
        let (selected_entities, usage) = self
            .map_entities(&mapping_query, include_entity_names, exclude_entity_names)
            .await?;

        let mut remaining = self.config.max_context_tokens;
        let mut context_parts = Vec::new();
        let mut context_tables = BTreeMap::new();
        if let Some(history) = conversation_history {
            let built = history.build_user_context(
                &self.tokenizer,
                self.config.conversation_history_max_turns,
                remaining,
            )?;
            if !built.text.trim().is_empty() {
                remaining = remaining
                    .saturating_sub(self.count(&built.text, "count conversation history context")?);
                context_parts.push(built.text);
                context_tables.insert("conversation history".to_owned(), built.table);
            }
        }

        let community_tokens = proportion(remaining, self.config.community_prop);
        if let Some(section) = self.build_community_context(&selected_entities, community_tokens)? {
            context_parts.push(section.text);
            context_tables.insert("reports".to_owned(), section.table);
        }

        let local_proportion =
            (1.0 - self.config.community_prop - self.config.text_unit_prop).max(0.0);
        let local_tokens = proportion(remaining, local_proportion);
        let local = self.build_local_context(&selected_entities, local_tokens)?;
        if !local.text.trim().is_empty() {
            context_parts.push(local.text);
            context_tables.extend(local.tables);
        }

        let source_tokens = proportion(remaining, self.config.text_unit_prop);
        if let Some(section) = self.build_source_context(&selected_entities, source_tokens)? {
            context_parts.push(section.text);
            context_tables.insert("sources".to_owned(), section.table);
        }

        let records = context_tables
            .into_iter()
            .map(|(name, table)| {
                table
                    .to_dataframe(SearchMethod::Local, "build Local context records")
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
                                    method: SearchMethod::Local,
                                    operation: "mark standard Local context records",
                                    message: source.to_string(),
                                })?;
                        }
                        Ok((name, dataframe))
                    })
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        Ok(LocalContextBuild {
            context: QueryContext {
                text: QueryContextText::Text(context_parts.join("\n\n")),
                records: QueryContextRecords::Tables(records),
            },
            usage,
        })
    }

    async fn map_entities<'a>(
        &'a self,
        query: &str,
        include_entity_names: &[String],
        exclude_entity_names: &[String],
    ) -> Result<(Vec<&'a Entity>, QueryUsageCategory)> {
        let mut matched = if query.is_empty() {
            let mut all = self.entities.iter().collect::<Vec<_>>();
            all.sort_by(|left, right| {
                right
                    .rank
                    .unwrap_or_default()
                    .cmp(&left.rank.unwrap_or_default())
            });
            all.truncate(self.config.top_k_entities);
            all
        } else {
            let response = self
                .embedding_model
                .embed(EmbeddingRequest::new(vec![query.to_owned()]))
                .await
                .map_err(|source| QueryError::QueryEmbedding {
                    method: SearchMethod::Local,
                    operation: "embed Local Search entity mapping query",
                    model: self.embedding_model_id.clone(),
                    source: Box::new(source),
                })?;
            let prompt_tokens =
                usize::try_from(response.usage.prompt_tokens).map_or(usize::MAX, |value| value);
            let vector = response
                .into_embeddings()
                .into_iter()
                .next()
                .ok_or_else(|| QueryError::QueryEmbedding {
                    method: SearchMethod::Local,
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
                    method: SearchMethod::Local,
                    operation: "validate Local Search query embedding",
                    model: self.embedding_model_id.clone(),
                    source: Box::new(graphloom_llm::LlmError::InvalidResponse {
                        model_instance: self.embedding_model_id.clone(),
                        operation: "embedding conversion",
                        message: "provider returned a non-finite query embedding".to_owned(),
                    }),
                });
            }
            let ann_k = self.config.top_k_entities.checked_mul(2).ok_or_else(|| {
                QueryError::InvalidQueryConfig {
                    method: SearchMethod::Local,
                    operation: "compute Local Search ANN oversampling",
                    message: "top_k_entities * 2 exceeds usize".to_owned(),
                }
            })?;
            let results = self
                .vector_store
                .similarity_search_by_vector(&self.vector_schema, &vector, ann_k, false)
                .await
                .map_err(|source| match source {
                    source @ VectorError::MissingIndex { .. } => QueryError::MissingVectorIndex {
                        method: SearchMethod::Local,
                        operation: "search entity_description",
                        index: self.vector_schema.index_name.clone(),
                        source: Box::new(source),
                    },
                    source => QueryError::InvalidVectorIndex {
                        method: SearchMethod::Local,
                        operation: "search entity_description",
                        index: self.vector_schema.index_name.clone(),
                        source: Box::new(source),
                    },
                })?;
            let by_id = self
                .entities
                .iter()
                .map(|entity| (entity.id.as_str(), entity))
                .collect::<BTreeMap<_, _>>();
            let mut entities = Vec::with_capacity(results.len());
            for result in results {
                let direct = by_id.get(result.document.id.as_str());
                let normalized = uuid::Uuid::parse_str(&result.document.id)
                    .ok()
                    .map(|value| value.simple().to_string());
                if let Some(entity) = direct.or_else(|| {
                    normalized
                        .as_deref()
                        .and_then(|normalized_id| by_id.get(normalized_id))
                }) {
                    entities.push(*entity);
                } else {
                    tracing::warn!(
                        method = %SearchMethod::Local,
                        entity_id = %result.document.id,
                        "entity_description contains a stale entity id"
                    );
                }
            }
            return Ok((
                add_entity_filters(
                    &self.entities,
                    entities,
                    include_entity_names,
                    exclude_entity_names,
                ),
                QueryUsageCategory {
                    llm_calls: 1,
                    prompt_tokens,
                    output_tokens: 0,
                },
            ));
        };
        matched = add_entity_filters(
            &self.entities,
            matched,
            include_entity_names,
            exclude_entity_names,
        );
        Ok((matched, QueryUsageCategory::default()))
    }

    fn build_community_context(
        &self,
        selected_entities: &[&Entity],
        max_tokens: usize,
    ) -> Result<Option<Section>> {
        if selected_entities.is_empty() || self.reports.is_empty() {
            return Ok(None);
        }
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
        let reports = self
            .reports
            .iter()
            .map(|report| (report.community_id.as_str(), report))
            .collect::<BTreeMap<_, _>>();
        let mut selected = matches
            .into_iter()
            .filter_map(|(community_id, count)| {
                reports
                    .get(community_id.as_str())
                    .copied()
                    .filter(|report| report.rank.is_some_and(|rank| rank >= 0.0))
                    .map(|report| (report, count))
            })
            .collect::<Vec<_>>();
        selected.sort_by(|(left, left_matches), (right, right_matches)| {
            right_matches.cmp(left_matches).then_with(|| {
                right
                    .rank
                    .unwrap_or_default()
                    .total_cmp(&left.rank.unwrap_or_default())
            })
        });
        let candidates = selected
            .into_iter()
            .map(|(report, _)| {
                vec![
                    report.short_id.clone(),
                    report.title.clone(),
                    report.full_content.clone(),
                ]
            })
            .collect::<Vec<_>>();
        let table = self.fit_report_rows(
            ContextTable::new(["id", "title", "content"], Vec::new()),
            candidates,
            "Reports",
            max_tokens,
            "build Local Reports context",
        )?;
        if table.is_empty() {
            return Ok(None);
        }
        Ok(Some(Section {
            text: table.render_csv_section(
                "Reports",
                SearchMethod::Local,
                "render Local Reports context",
            )?,
            table,
        }))
    }

    fn build_local_context(
        &self,
        selected_entities: &[&Entity],
        max_tokens: usize,
    ) -> Result<LocalSections> {
        if selected_entities.is_empty() {
            return Ok(LocalSections {
                text: String::new(),
                tables: BTreeMap::new(),
            });
        }
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
        let entity_table = self.fit_delimited_rows(
            ContextTable::new(
                ["id", "entity", "description", "number of relationships"],
                Vec::new(),
            ),
            entity_candidates,
            "Entities",
            max_tokens,
            "build Local Entities context",
        )?;
        let entity_text = entity_table.render_delimited_section(
            "Entities",
            SearchMethod::Local,
            "render Local Entities context",
        )?;
        let entity_tokens = self.count(&entity_text, "count Local Entities context")?;

        let covariate_groups = group_covariates(&self.covariates);
        let mut accepted_text = Vec::new();
        let mut accepted_tables = BTreeMap::new();
        let mut learned_links = BTreeMap::new();
        for end in 1..=selected_entities.len() {
            let current_entities = &selected_entities[..end];
            let relationship = self.build_relationship_context_with_links(
                current_entities,
                max_tokens,
                &mut learned_links,
            )?;
            let mut current_text = Vec::new();
            let mut current_tables = BTreeMap::new();
            let mut total_tokens = entity_tokens;
            if let Some(section) = relationship {
                total_tokens = total_tokens.saturating_add(
                    self.count(&section.text, "count Local Relationships context")?,
                );
                current_text.push(section.text);
                current_tables.insert("relationships".to_owned(), section.table);
            } else {
                current_tables.insert(
                    "relationships".to_owned(),
                    ContextTable::new(
                        ["id", "source", "target", "description", "weight"],
                        Vec::new(),
                    ),
                );
            }
            for (name, covariates) in &covariate_groups {
                let section =
                    self.build_covariate_context(name, covariates, current_entities, max_tokens)?;
                if let Some(section) = section {
                    total_tokens = total_tokens.saturating_add(
                        self.count(&section.text, "count Local covariate context")?,
                    );
                    current_text.push(section.text);
                    current_tables.insert(name.to_lowercase(), section.table);
                } else {
                    current_tables.insert(
                        name.to_lowercase(),
                        ContextTable::new(covariate_columns(), Vec::new()),
                    );
                }
            }
            if total_tokens > max_tokens {
                tracing::warn!(
                    method = %SearchMethod::Local,
                    "Local entity expansion reached the token limit; reverting the current entity"
                );
                break;
            }
            accepted_text = current_text;
            accepted_tables = current_tables;
        }
        let mut text = vec![entity_text];
        text.extend(accepted_text);
        accepted_tables.insert("entities".to_owned(), entity_table);
        Ok(LocalSections {
            text: text
                .into_iter()
                .filter(|section| !section.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n\n"),
            tables: accepted_tables,
        })
    }

    fn build_relationship_context_with_links(
        &self,
        selected_entities: &[&Entity],
        max_tokens: usize,
        learned_links: &mut BTreeMap<String, usize>,
    ) -> Result<Option<Section>> {
        let selected = filter_relationships(
            selected_entities,
            &self.relationships,
            self.config.top_k_relationships,
            learned_links,
        );
        if selected.is_empty() {
            return Ok(None);
        }
        let include_links = selected.first().is_some_and(|value| value.links.is_some());
        let mut columns = vec!["id", "source", "target", "description", "weight"];
        if include_links {
            columns.push("links");
        }
        let candidates = selected
            .into_iter()
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
        let table = self.fit_delimited_rows(
            ContextTable::new(columns, Vec::new()),
            candidates,
            "Relationships",
            max_tokens,
            "build Local Relationships context",
        )?;
        Ok(Some(Section {
            text: table.render_delimited_section(
                "Relationships",
                SearchMethod::Local,
                "render Local Relationships context",
            )?,
            table,
        }))
    }

    fn build_covariate_context(
        &self,
        name: &str,
        covariates: &[&Covariate],
        selected_entities: &[&Entity],
        max_tokens: usize,
    ) -> Result<Option<Section>> {
        let mut candidates = Vec::new();
        for entity in selected_entities {
            for covariate in covariates
                .iter()
                .filter(|covariate| covariate.subject_id == entity.title)
            {
                candidates.push(vec![
                    covariate.short_id.clone().unwrap_or_default(),
                    covariate.subject_id.clone(),
                    covariate.object_id.clone().unwrap_or_default(),
                    covariate.status.clone().unwrap_or_default(),
                    covariate.start_date.clone().unwrap_or_default(),
                    covariate.end_date.clone().unwrap_or_default(),
                    covariate.description.clone().unwrap_or_default(),
                ]);
            }
        }
        if candidates.is_empty() {
            return Ok(None);
        }
        let table = self.fit_delimited_rows(
            ContextTable::new(covariate_columns(), Vec::new()),
            candidates,
            name,
            max_tokens,
            "build Local covariate context",
        )?;
        Ok(Some(Section {
            text: table.render_delimited_section(
                name,
                SearchMethod::Local,
                "render Local covariate context",
            )?,
            table,
        }))
    }

    fn build_source_context(
        &self,
        selected_entities: &[&Entity],
        max_tokens: usize,
    ) -> Result<Option<Section>> {
        if selected_entities.is_empty() || self.text_units.is_empty() {
            return Ok(None);
        }
        let units = self
            .text_units
            .iter()
            .map(|unit| (unit.id.as_str(), unit))
            .collect::<BTreeMap<_, _>>();
        let mut seen = BTreeSet::new();
        let mut ranked = Vec::<(&TextUnit, usize, usize)>::new();
        for (entity_order, entity) in selected_entities.iter().enumerate() {
            let entity_relationships = self
                .relationships
                .iter()
                .filter(|relationship| {
                    relationship.source == entity.title || relationship.target == entity.title
                })
                .collect::<Vec<_>>();
            for text_unit_id in &entity.text_unit_ids {
                if !seen.insert(text_unit_id.as_str()) {
                    continue;
                }
                let Some(unit) = units.get(text_unit_id.as_str()).copied() else {
                    tracing::warn!(
                        method = %SearchMethod::Local,
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
        if ranked.is_empty() {
            return Ok(None);
        }
        let candidates = ranked
            .into_iter()
            .map(|(unit, _, _)| vec![unit.short_id.clone(), unit.text.clone()])
            .collect::<Vec<_>>();
        let table = self.fit_delimited_rows(
            ContextTable::new(["id", "text"], Vec::new()),
            candidates,
            "Sources",
            max_tokens,
            "build Local Sources context",
        )?;
        Ok(Some(Section {
            text: table.render_delimited_section(
                "Sources",
                SearchMethod::Local,
                "render Local Sources context",
            )?,
            table,
        }))
    }

    fn fit_delimited_rows(
        &self,
        mut table: ContextTable,
        candidates: Vec<Vec<String>>,
        context_name: &str,
        max_tokens: usize,
        operation: &'static str,
    ) -> Result<ContextTable> {
        let header = table.render_delimited_header(context_name, SearchMethod::Local, operation)?;
        let mut tokens = self.count(&header, operation)?;
        for row in candidates {
            let row_text = table.render_delimited_row(&row, SearchMethod::Local, operation)?;
            let row_tokens = self.count(&row_text, operation)?;
            if tokens.saturating_add(row_tokens) > max_tokens {
                break;
            }
            tokens = tokens.saturating_add(row_tokens);
            table.push(row);
        }
        Ok(table)
    }

    fn fit_report_rows(
        &self,
        mut table: ContextTable,
        candidates: Vec<Vec<String>>,
        context_name: &str,
        max_tokens: usize,
        operation: &'static str,
    ) -> Result<ContextTable> {
        let header = table.render_delimited_header(context_name, SearchMethod::Local, operation)?;
        let mut tokens = self.count(&header, operation)?;
        for row in candidates {
            let row_text = table.render_delimited_row(&row, SearchMethod::Local, operation)?;
            let row_tokens = self.count(&row_text, operation)?;
            if tokens.saturating_add(row_tokens) > max_tokens {
                break;
            }
            tokens = tokens.saturating_add(row_tokens);
            table.push(row);
        }
        Ok(table)
    }

    fn count(&self, text: &str, operation: &'static str) -> Result<usize> {
        self.tokenizer
            .count(text)
            .map_err(|source| QueryError::QueryContext {
                method: SearchMethod::Local,
                operation,
                message: source.to_string(),
            })
    }
}

fn local_table_requires_in_context(name: &str) -> bool {
    !matches!(name, "conversation history" | "reports" | "sources")
}

fn add_entity_filters<'a>(
    all_entities: &'a [Entity],
    matched: Vec<&'a Entity>,
    include_entity_names: &[String],
    exclude_entity_names: &[String],
) -> Vec<&'a Entity> {
    let mut result = Vec::new();
    for name in include_entity_names {
        result.extend(all_entities.iter().filter(|entity| &entity.title == name));
    }
    result.extend(matched.into_iter().filter(|entity| {
        !exclude_entity_names
            .iter()
            .any(|name| name == &entity.title)
    }));
    result
}

fn filter_relationships<'a>(
    selected_entities: &[&Entity],
    relationships: &'a [Relationship],
    top_k_relationships: usize,
    learned_links: &mut BTreeMap<String, usize>,
) -> Vec<RankedRelationship<'a>> {
    let selected_names = selected_entities
        .iter()
        .map(|entity| entity.title.as_str())
        .collect::<BTreeSet<_>>();
    let mut in_network = relationships
        .iter()
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

    let mut out_network = relationships
        .iter()
        .filter(|relationship| {
            selected_names.contains(relationship.source.as_str())
                && !selected_names.contains(relationship.target.as_str())
        })
        .chain(relationships.iter().filter(|relationship| {
            selected_names.contains(relationship.target.as_str())
                && !selected_names.contains(relationship.source.as_str())
        }))
        .map(|relationship| RankedRelationship {
            relationship,
            links: None,
        })
        .collect::<Vec<_>>();
    out_network.sort_by(rank_relationships);
    if out_network.len() > 1 {
        let outside_names = out_network
            .iter()
            .map(|ranked| {
                if selected_names.contains(ranked.relationship.source.as_str()) {
                    ranked.relationship.target.as_str()
                } else {
                    ranked.relationship.source.as_str()
                }
            })
            .collect::<BTreeSet<_>>();
        let links = outside_names
            .into_iter()
            .map(|outside| {
                let neighbors = out_network
                    .iter()
                    .filter_map(|ranked| {
                        let relationship = ranked.relationship;
                        if relationship.source == outside {
                            Some(relationship.target.as_str())
                        } else if relationship.target == outside {
                            Some(relationship.source.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<BTreeSet<_>>();
                (outside, neighbors.len())
            })
            .collect::<BTreeMap<_, _>>();
        for ranked in &mut out_network {
            let outside = if selected_names.contains(ranked.relationship.source.as_str()) {
                ranked.relationship.target.as_str()
            } else {
                ranked.relationship.source.as_str()
            };
            ranked.links = links.get(outside).copied();
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
        out_network.truncate(budget);
    }
    in_network.extend(out_network);
    in_network
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

fn group_covariates(covariates: &[Covariate]) -> Vec<(String, Vec<&Covariate>)> {
    let mut groups = Vec::<(String, Vec<&Covariate>)>::new();
    for covariate in covariates {
        if let Some((_, values)) = groups
            .iter_mut()
            .find(|(name, _)| name == &covariate.covariate_type)
        {
            values.push(covariate);
        } else {
            groups.push((covariate.covariate_type.clone(), vec![covariate]));
        }
    }
    groups
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
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering as AtomicOrdering},
    };

    use async_trait::async_trait;
    use graphloom_llm::{EmbeddingResponse, EmbeddingUsage, LlmError};
    use graphloom_vectors::{VectorDocument, VectorSearchResult};
    use polars_core::prelude::DataType;

    use super::*;
    use crate::query::{ConversationRole, ConversationTurn};

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
    struct RecordingEmbedding {
        inputs: Arc<Mutex<Vec<Vec<String>>>>,
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
                prompt_tokens: 7,
                total_tokens: 7,
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
        Fixture {
            builder: LocalContextBuilder {
                config: LocalSearchConfig {
                    max_context_tokens,
                    top_k_entities: 2,
                    top_k_relationships: 1,
                    community_prop: 0.2,
                    text_unit_prop: 0.3,
                    ..LocalSearchConfig::default()
                },
                entities,
                reports: vec![
                    report("1", 8.0, "Alpha report"),
                    report("2", 5.0, "Shared report"),
                    report("3", 9.0, "Carol report"),
                ],
                text_units: vec![
                    text_unit("tu-a", "0", "Alice source", &["rel-ab", "rel-ax"]),
                    text_unit("tu-b", "1", "Bob source", &["rel-ab"]),
                    text_unit("tu-c", "2", "Carol source", &[]),
                    text_unit("tu-shared", "3", "Shared source", &["rel-ab"]),
                ],
                relationships: vec![
                    relationship("rel-ab", "0", "Alice", "Bob", 9, 1.5, &["tu-a", "tu-b"]),
                    relationship("rel-ax", "1", "Alice", "External", 7, 0.0, &["tu-a"]),
                    relationship("rel-bx", "2", "Bob", "External", 6, 2.0, &[]),
                    relationship("rel-ay", "3", "Alice", "Other", 8, 3.0, &[]),
                ],
                covariates: vec![
                    covariate("claim-1", "10", "Alice", "claims", "Alice claim"),
                    covariate("fact-1", "11", "Bob", "facts", "Bob fact"),
                ],
                embedding_model: Arc::new(RecordingEmbedding {
                    inputs: Arc::clone(&embedding_inputs),
                }),
                embedding_model_id: "embedding".to_owned(),
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
            .map_entities("question", &[], &[])
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
    async fn test_should_match_dashed_ann_uuid_to_undashed_entity_id() {
        let dashed = "550e8400-e29b-41d4-a716-446655440000";
        let mut fixture = fixture(20_000, &[dashed]);
        fixture.builder.entities[0].id = dashed.replace('-', "");

        let (selected, _) = fixture
            .builder
            .map_entities("question", &[], &[])
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
            .map_entities("question", &include, &exclude)
            .await
            .expect("entity filters");

        assert_eq!(
            selected
                .iter()
                .map(|entity| entity.title.as_str())
                .collect::<Vec<_>>(),
            ["Carol", "Alice", "Carol"]
        );
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
    fn test_should_skip_community_without_report_and_stop_at_record_boundary() {
        let mut fixture = fixture(20_000, &[]);
        fixture
            .builder
            .reports
            .retain(|report| report.community_id != "3");
        let carol = vec![&fixture.builder.entities[2]];
        assert!(
            fixture
                .builder
                .build_community_context(&carol, 20_000)
                .expect("missing report context")
                .is_none()
        );

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
        assert!(
            fixture
                .builder
                .build_community_context(&selected, budget - 1)
                .expect("under-budget Local Reports")
                .is_none()
        );
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

        let ranked = filter_relationships(
            &selected,
            &fixture.builder.relationships,
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
            .render_delimited_section("Entities", SearchMethod::Local, "test entities")
            .expect("entity text");
        let relationship = fixture
            .builder
            .build_relationship_context_with_links(&selected[..1], 20_000, &mut BTreeMap::new())
            .expect("relationship section")
            .expect("Alice relationships");
        let claim_group = group_covariates(&fixture.builder.covariates);
        let claim = fixture
            .builder
            .build_covariate_context(&claim_group[0].0, &claim_group[0].1, &selected[..1], 20_000)
            .expect("claim context")
            .expect("Alice claim");
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
            .map_entities("question", &[], &[])
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
            .map_entities("question", &[], &[])
            .await
            .expect_err("dimension mismatch");

        assert!(matches!(error, QueryError::InvalidVectorIndex { .. }));
        assert!(error.to_string().contains("dimension 2"));
    }
}
