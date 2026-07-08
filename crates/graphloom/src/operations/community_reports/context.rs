//! Graph context construction for community reports.

use std::collections::{BTreeMap, BTreeSet};

use csv::WriterBuilder;
use graphloom_llm::Tokenizer;

use super::{
    ClaimContextRow, CommunityInputRow, CommunityLocalContext, EntityContextRow, ExplodedEntityRow,
    RelationshipContextRow,
};
use crate::{Result, dataframe::invalid_data};

const COMMUNITY_REPORTS_CONTEXT: &str = "create_community_reports";
const NO_DESCRIPTION: &str = "No Description";

#[derive(Debug, Clone, Default)]
struct ContextRecords {
    entities: BTreeMap<i64, EntityContextRow>,
    claims: BTreeMap<i64, ClaimContextRow>,
    relationships: BTreeMap<i64, RelationshipContextRow>,
}

pub(crate) fn explode_communities(
    communities: &[CommunityInputRow],
    entities: &[EntityContextRow],
) -> Vec<ExplodedEntityRow> {
    let entities_by_id = entities
        .iter()
        .map(|entity| (entity.id.as_str(), entity))
        .collect::<BTreeMap<_, _>>();
    let mut exploded = Vec::new();
    for community in communities {
        for entity_id in &community.entity_ids {
            if let Some(entity) = entities_by_id.get(entity_id.as_str()) {
                exploded.push(ExplodedEntityRow {
                    community: community.community,
                    level: community.level,
                    entity: (*entity).clone(),
                });
            }
        }
    }
    exploded
}

pub(crate) fn build_local_contexts(
    communities: &[CommunityInputRow],
    entities: &[EntityContextRow],
    relationships: &[RelationshipContextRow],
    claims: &[ClaimContextRow],
    tokenizer: &dyn Tokenizer,
    max_input_length: usize,
) -> Result<BTreeMap<i64, CommunityLocalContext>> {
    let exploded = explode_communities(communities, entities);
    let mut entities_by_community: BTreeMap<i64, Vec<EntityContextRow>> = BTreeMap::new();
    for row in exploded {
        entities_by_community
            .entry(row.community)
            .or_default()
            .push(row.entity);
    }

    let mut contexts = BTreeMap::new();
    for community in communities {
        let mut community_entities = entities_by_community
            .get(&community.community)
            .cloned()
            .unwrap_or_default();
        community_entities.sort_by(|left, right| {
            right
                .degree
                .cmp(&left.degree)
                .then_with(|| left.human_readable_id.cmp(&right.human_readable_id))
        });
        let context = build_context_for_entities(
            community.community,
            &community_entities,
            relationships,
            claims,
            tokenizer,
            max_input_length,
        )?;
        contexts.insert(community.community, context);
    }
    Ok(contexts)
}

pub(crate) fn render_reports_section(rows: &[(i64, String)]) -> Result<String> {
    render_section(
        "----Reports-----",
        &["community", "full_content"],
        rows.iter()
            .map(|(community, content)| vec![community.to_string(), content.clone()])
            .collect(),
    )
}

fn build_context_for_entities(
    community: i64,
    entities: &[EntityContextRow],
    relationships: &[RelationshipContextRow],
    claims: &[ClaimContextRow],
    tokenizer: &dyn Tokenizer,
    max_input_length: usize,
) -> Result<CommunityLocalContext> {
    let entity_by_title = entities
        .iter()
        .map(|entity| (entity.title.as_str(), entity))
        .collect::<BTreeMap<_, _>>();
    let entity_titles = entity_by_title.keys().copied().collect::<BTreeSet<_>>();
    let claims_by_subject = claims_by_subject(claims);
    let mut ordered_relationships = relationships
        .iter()
        .filter(|relationship| {
            entity_titles.contains(relationship.source.as_str())
                && entity_titles.contains(relationship.target.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();
    ordered_relationships.sort_by(|left, right| {
        right
            .combined_degree
            .cmp(&left.combined_degree)
            .then_with(|| left.human_readable_id.cmp(&right.human_readable_id))
    });

    let mut committed = ContextRecords::default();
    let mut committed_text = render_graph_context(&committed)?;
    let mut committed_tokens = tokenizer.count(&committed_text)?;
    let mut exceeded = false;

    if ordered_relationships.is_empty() {
        return build_entity_only_context(
            community,
            entities,
            &claims_by_subject,
            tokenizer,
            max_input_length,
        );
    }

    for relationship in ordered_relationships {
        let mut candidate = committed.clone();
        add_entity_and_claims(
            &mut candidate,
            relationship.source.as_str(),
            &entity_by_title,
            &claims_by_subject,
        );
        add_entity_and_claims(
            &mut candidate,
            relationship.target.as_str(),
            &entity_by_title,
            &claims_by_subject,
        );
        candidate
            .relationships
            .insert(relationship.human_readable_id, relationship);
        let candidate_text = render_graph_context(&candidate)?;
        let candidate_tokens = tokenizer.count(&candidate_text)?;
        if candidate_tokens > max_input_length && !candidate.entities.is_empty() {
            exceeded = true;
            if committed.entities.is_empty() {
                committed_text = candidate_text;
                committed_tokens = candidate_tokens;
            }
            break;
        }
        committed = candidate;
        committed_text = candidate_text;
        committed_tokens = candidate_tokens;
    }

    Ok(CommunityLocalContext {
        community,
        context: committed_text,
        token_count: committed_tokens,
        exceeds_limit: exceeded || committed_tokens > max_input_length,
    })
}

fn build_entity_only_context(
    community: i64,
    entities: &[EntityContextRow],
    claims_by_subject: &BTreeMap<&str, Vec<&ClaimContextRow>>,
    tokenizer: &dyn Tokenizer,
    max_input_length: usize,
) -> Result<CommunityLocalContext> {
    let entity_by_title = entities
        .iter()
        .map(|entity| (entity.title.as_str(), entity))
        .collect::<BTreeMap<_, _>>();
    let mut committed = ContextRecords::default();
    let mut committed_text = render_graph_context(&committed)?;
    let mut committed_tokens = tokenizer.count(&committed_text)?;
    let mut exceeded = false;

    for entity in entities {
        let mut candidate = committed.clone();
        add_entity_and_claims(
            &mut candidate,
            entity.title.as_str(),
            &entity_by_title,
            claims_by_subject,
        );
        let candidate_text = render_graph_context(&candidate)?;
        let candidate_tokens = tokenizer.count(&candidate_text)?;
        if candidate_tokens > max_input_length && !candidate.entities.is_empty() {
            exceeded = true;
            if committed.entities.is_empty() {
                committed_text = candidate_text;
                committed_tokens = candidate_tokens;
            }
            break;
        }
        committed = candidate;
        committed_text = candidate_text;
        committed_tokens = candidate_tokens;
    }

    Ok(CommunityLocalContext {
        community,
        context: committed_text,
        token_count: committed_tokens,
        exceeds_limit: exceeded || committed_tokens > max_input_length,
    })
}

fn add_entity_and_claims<'a>(
    records: &mut ContextRecords,
    title: &str,
    entity_by_title: &BTreeMap<&str, &'a EntityContextRow>,
    claims_by_subject: &BTreeMap<&str, Vec<&'a ClaimContextRow>>,
) {
    if let Some(entity) = entity_by_title.get(title) {
        records
            .entities
            .insert(entity.human_readable_id, (*entity).clone());
    }
    if let Some(claims) = claims_by_subject.get(title) {
        for claim in claims {
            records
                .claims
                .insert(claim.human_readable_id, (*claim).clone());
        }
    }
}

fn claims_by_subject(claims: &[ClaimContextRow]) -> BTreeMap<&str, Vec<&ClaimContextRow>> {
    let mut grouped: BTreeMap<&str, Vec<&ClaimContextRow>> = BTreeMap::new();
    for claim in claims {
        grouped
            .entry(claim.subject_id.as_str())
            .or_default()
            .push(claim);
    }
    for values in grouped.values_mut() {
        values.sort_by_key(|claim| claim.human_readable_id);
    }
    grouped
}

fn render_graph_context(records: &ContextRecords) -> Result<String> {
    let entities = render_section(
        "-----Entities-----",
        &["human_readable_id", "title", "description"],
        records
            .entities
            .values()
            .map(|entity| {
                vec![
                    entity.human_readable_id.to_string(),
                    entity.title.clone(),
                    description_or_default(&entity.description),
                ]
            })
            .collect(),
    )?;
    let claims = if records.claims.is_empty() {
        String::new()
    } else {
        render_section(
            "-----Claims-----",
            &[
                "human_readable_id",
                "subject_id",
                "type",
                "status",
                "description",
            ],
            records
                .claims
                .values()
                .map(|claim| {
                    vec![
                        claim.human_readable_id.to_string(),
                        claim.subject_id.clone(),
                        claim.claim_type.clone(),
                        claim.status.clone(),
                        description_or_default(&claim.description),
                    ]
                })
                .collect(),
        )?
    };
    let relationships = render_section(
        "-----Relationships-----",
        &["human_readable_id", "source", "target", "description"],
        records
            .relationships
            .values()
            .map(|relationship| {
                vec![
                    relationship.human_readable_id.to_string(),
                    relationship.source.clone(),
                    relationship.target.clone(),
                    description_or_default(&relationship.description),
                ]
            })
            .collect(),
    )?;

    Ok([entities, claims, relationships]
        .into_iter()
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>()
        .join("\n"))
}

fn render_section(title: &str, headers: &[&str], rows: Vec<Vec<String>>) -> Result<String> {
    let mut writer = WriterBuilder::new()
        .has_headers(false)
        .from_writer(Vec::new());
    writer.write_record(headers).map_err(|source| {
        invalid_data(
            COMMUNITY_REPORTS_CONTEXT,
            &format!("failed to write community context csv: {source}"),
        )
    })?;
    for row in rows {
        writer.write_record(row).map_err(|source| {
            invalid_data(
                COMMUNITY_REPORTS_CONTEXT,
                &format!("failed to write community context csv: {source}"),
            )
        })?;
    }
    let bytes = writer.into_inner().map_err(|source| {
        invalid_data(
            COMMUNITY_REPORTS_CONTEXT,
            &format!("failed to finish csv section {title}: {source}"),
        )
    })?;
    let csv = String::from_utf8(bytes).map_err(|source| {
        invalid_data(
            COMMUNITY_REPORTS_CONTEXT,
            &format!("failed to encode csv section {title}: {source}"),
        )
    })?;
    Ok(format!("{title}\n{}", csv.trim_end()))
}

fn description_or_default(description: &str) -> String {
    if description.trim().is_empty() {
        NO_DESCRIPTION.to_owned()
    } else {
        description.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use graphloom_llm::TiktokenTokenizer;

    use super::*;

    #[test]
    fn test_should_explode_entity_memberships_across_levels() {
        let entities = vec![entity("e1", 0, "ALICE", 2)];
        let communities = vec![
            community(0, 0, vec!["e1"]),
            community(1, 1, vec!["missing", "e1"]),
        ];

        let exploded = explode_communities(&communities, &entities);

        assert_eq!(
            exploded
                .iter()
                .map(|row| (row.community, row.level, row.entity.id.as_str()))
                .collect::<Vec<_>>(),
            vec![(0, 0, "e1"), (1, 1, "e1")]
        );
    }

    #[test]
    fn test_should_escape_csv_and_sort_relationship_context() {
        let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");
        let communities = vec![community(0, 0, vec!["e1", "e2", "e3"])];
        let entities = vec![
            entity("e1", 0, "ALICE", 1),
            entity("e2", 1, "BOB", 1),
            entity("e3", 2, "CAROL", 1),
        ];
        let relationships = vec![
            relationship(2, "ALICE", "BOB", "low"),
            relationship(1, "BOB", "CAROL", "quote \"and\", comma\nline"),
        ];
        let contexts = build_local_contexts(
            &communities,
            &entities,
            &relationships,
            &[],
            &tokenizer,
            8_000,
        )
        .expect("contexts");

        let context = &contexts.get(&0).expect("community").context;

        assert!(context.contains("\"quote \"\"and\"\", comma\nline\""));
        assert!(
            context.find("1,BOB,CAROL").expect("relationship 1")
                < context.find("2,ALICE,BOB").expect("relationship 2")
        );
    }

    #[test]
    fn test_should_include_entities_without_relationships() {
        let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");
        let contexts = build_local_contexts(
            &[community(0, 0, vec!["e1"])],
            &[entity("e1", 0, "ALICE", 3)],
            &[],
            &[],
            &tokenizer,
            8_000,
        )
        .expect("contexts");

        assert!(contexts.get(&0).expect("context").context.contains("ALICE"));
    }

    fn community(community: i64, level: i64, entity_ids: Vec<&str>) -> CommunityInputRow {
        CommunityInputRow {
            community,
            level,
            parent: -1,
            children: Vec::new(),
            entity_ids: entity_ids.into_iter().map(str::to_owned).collect(),
            period: "2026-07-08".to_owned(),
            size: 1,
        }
    }

    fn entity(id: &str, human_readable_id: i64, title: &str, degree: i64) -> EntityContextRow {
        EntityContextRow {
            id: id.to_owned(),
            human_readable_id,
            title: title.to_owned(),
            description: "desc".to_owned(),
            degree,
        }
    }

    fn relationship(
        human_readable_id: i64,
        source: &str,
        target: &str,
        description: &str,
    ) -> RelationshipContextRow {
        RelationshipContextRow {
            id: format!("rel-{human_readable_id}"),
            human_readable_id,
            source: source.to_owned(),
            target: target.to_owned(),
            description: description.to_owned(),
            combined_degree: 10 - human_readable_id,
        }
    }
}
