//! GraphRAG-compatible graph context construction for community reports.

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

#[derive(Debug, Clone)]
struct LocalContextRecord {
    title: String,
    entity: EntityContextRow,
    relationships: Vec<RelationshipContextRow>,
}

#[derive(Debug, Default)]
struct RenderRecords {
    entities: Vec<EntityContextRow>,
    relationships: Vec<RelationshipContextRow>,
}

pub(crate) fn explode_communities(
    communities: &[CommunityInputRow],
    entities: &[EntityContextRow],
) -> Vec<ExplodedEntityRow> {
    let memberships = communities
        .iter()
        .flat_map(|community| {
            community
                .entity_ids
                .iter()
                .map(move |entity_id| (entity_id.as_str(), community))
        })
        .fold(
            BTreeMap::<&str, Vec<&CommunityInputRow>>::new(),
            |mut memberships, (entity_id, community)| {
                memberships.entry(entity_id).or_default().push(community);
                memberships
            },
        );

    entities
        .iter()
        .flat_map(|entity| {
            memberships
                .get(entity.id.as_str())
                .into_iter()
                .flatten()
                .map(move |community| ExplodedEntityRow {
                    community: community.community,
                    level: community.level,
                    entity: entity.clone(),
                })
        })
        .collect()
}

/// Build the exact graph-context strings used by GraphRAG's standard community-report workflow.
///
/// GraphRAG currently prepares claims as scalar merge values while its context sorter only
/// accepts claim lists. Consequently claims are absent from the rendered graph context. The
/// unused argument is retained because it remains part of the public workflow input contract.
pub(crate) fn build_local_contexts(
    communities: &[CommunityInputRow],
    entities: &[EntityContextRow],
    relationships: &[RelationshipContextRow],
    _claims: &[ClaimContextRow],
    tokenizer: &dyn Tokenizer,
    max_input_length: usize,
) -> Result<BTreeMap<i64, CommunityLocalContext>> {
    let exploded = explode_communities(communities, entities);
    let levels = exploded
        .iter()
        .map(|row| row.level)
        .collect::<BTreeSet<_>>();
    let mut contexts = BTreeMap::new();

    for level in levels.into_iter().rev() {
        let level_rows = exploded
            .iter()
            .filter(|row| row.level == level)
            .collect::<Vec<_>>();
        let level_titles = level_rows
            .iter()
            .map(|row| row.entity.title.as_str())
            .collect::<BTreeSet<_>>();
        let level_relationships = relationships
            .iter()
            .filter(|relationship| {
                level_titles.contains(relationship.source.as_str())
                    && level_titles.contains(relationship.target.as_str())
            })
            .collect::<Vec<_>>();
        let source_first = first_relationships_by_title(&level_relationships, |relationship| {
            relationship.source.as_str()
        });
        let target_first = first_relationships_by_title(&level_relationships, |relationship| {
            relationship.target.as_str()
        });
        let mut grouped = BTreeMap::<(String, i64, i64), LocalContextRecord>::new();

        for row in level_rows {
            let title = row.entity.title.clone();
            let key = (title.clone(), row.community, row.entity.degree);
            let record = grouped.entry(key).or_insert_with(|| LocalContextRecord {
                title: title.clone(),
                entity: entity_with_default_description(&row.entity),
                relationships: Vec::new(),
            });
            if let Some(relationship) = source_first
                .get(title.as_str())
                .or_else(|| target_first.get(title.as_str()))
            {
                record
                    .relationships
                    .push(relationship_with_default_description(relationship));
            }
        }

        let mut records_by_community = BTreeMap::<i64, Vec<LocalContextRecord>>::new();
        for ((_, community, _), record) in grouped {
            records_by_community
                .entry(community)
                .or_default()
                .push(record);
        }
        for community in communities
            .iter()
            .filter(|community| community.level == level)
        {
            let records = records_by_community
                .remove(&community.community)
                .unwrap_or_default();
            let context = sort_context(&records, tokenizer, max_input_length)?;
            contexts.insert(community.community, CommunityLocalContext { context });
        }
    }
    Ok(contexts)
}

fn first_relationships_by_title<'a, F>(
    relationships: &[&'a RelationshipContextRow],
    title: F,
) -> BTreeMap<&'a str, &'a RelationshipContextRow>
where
    F: Fn(&'a RelationshipContextRow) -> &'a str,
{
    relationships
        .iter()
        .fold(BTreeMap::new(), |mut first, row| {
            first.entry(title(row)).or_insert(row);
            first
        })
}

fn sort_context(
    local_context: &[LocalContextRecord],
    tokenizer: &dyn Tokenizer,
    max_input_length: usize,
) -> Result<String> {
    let mut relationships = local_context
        .iter()
        .flat_map(|record| record.relationships.iter().cloned())
        .collect::<Vec<_>>();
    relationships.sort_by(|left, right| {
        right
            .combined_degree
            .cmp(&left.combined_degree)
            .then_with(|| left.human_readable_id.cmp(&right.human_readable_id))
    });
    let entities_by_title = local_context
        .iter()
        .map(|record| (record.title.as_str(), &record.entity))
        .collect::<BTreeMap<_, _>>();
    let mut entity_ids = BTreeSet::new();
    let mut relationship_ids = BTreeSet::new();
    let mut rendered = RenderRecords::default();
    let mut context = String::new();

    for relationship in relationships {
        for title in [relationship.source.as_str(), relationship.target.as_str()] {
            if let Some(entity) = entities_by_title.get(title)
                && entity_ids.insert(entity.human_readable_id)
            {
                rendered.entities.push((*entity).clone());
            }
        }
        if relationship_ids.insert(relationship.human_readable_id) {
            rendered.relationships.push(relationship);
        }

        let candidate = render_context(&rendered)?;
        if tokenizer.count(&candidate)? > max_input_length {
            break;
        }
        context = candidate;
    }

    if context.is_empty() {
        render_context(&rendered)
    } else {
        Ok(context)
    }
}

fn render_context(records: &RenderRecords) -> Result<String> {
    let entities = (!records.entities.is_empty())
        .then(|| render_entities_csv(&records.entities))
        .transpose()?;
    let relationships = (!records.relationships.is_empty())
        .then(|| render_relationships_csv(&records.relationships))
        .transpose()?;

    Ok([entities, relationships]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n\n"))
}

fn render_entities_csv(rows: &[EntityContextRow]) -> Result<String> {
    render_section(
        "-----Entities-----",
        &["human_readable_id", "title", "description", "degree"],
        rows.iter()
            .map(|entity| {
                vec![
                    entity.human_readable_id.to_string(),
                    entity.title.clone(),
                    entity.description.clone(),
                    entity.degree.to_string(),
                ]
            })
            .collect(),
    )
}

fn render_relationships_csv(rows: &[RelationshipContextRow]) -> Result<String> {
    render_section(
        "-----Relationships-----",
        &[
            "human_readable_id",
            "source",
            "target",
            "description",
            "combined_degree",
        ],
        rows.iter()
            .map(|relationship| {
                vec![
                    relationship.human_readable_id.to_string(),
                    relationship.source.clone(),
                    relationship.target.clone(),
                    relationship.description.clone(),
                    relationship.combined_degree.to_string(),
                ]
            })
            .collect(),
    )
}

fn render_section(title: &str, headers: &[&str], rows: Vec<Vec<String>>) -> Result<String> {
    let mut writer = WriterBuilder::new()
        .has_headers(false)
        .from_writer(Vec::new());
    writer
        .write_record(headers)
        .map_err(|source| csv_context_error(&source))?;
    for row in rows {
        writer
            .write_record(row)
            .map_err(|source| csv_context_error(&source))?;
    }
    let bytes = writer
        .into_inner()
        .map_err(|source| csv_context_error(&source))?;
    let csv = String::from_utf8(bytes).map_err(|source| {
        invalid_data(
            COMMUNITY_REPORTS_CONTEXT,
            &format!("failed to encode community context csv: {source}"),
        )
    })?;
    Ok(format!("{title}\n{csv}"))
}

fn csv_context_error(source: &impl std::fmt::Display) -> crate::GraphLoomError {
    invalid_data(
        COMMUNITY_REPORTS_CONTEXT,
        &format!("failed to write community context csv: {source}"),
    )
}

fn entity_with_default_description(entity: &EntityContextRow) -> EntityContextRow {
    let mut entity = entity.clone();
    entity.description = description_or_default(&entity.description);
    entity
}

fn relationship_with_default_description(
    relationship: &RelationshipContextRow,
) -> RelationshipContextRow {
    let mut relationship = relationship.clone();
    relationship.description = description_or_default(&relationship.description);
    relationship
}

fn description_or_default(description: &str) -> String {
    if description.is_empty() {
        NO_DESCRIPTION.to_owned()
    } else {
        description.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use graphloom_llm::{TiktokenTokenizer, Tokenizer};

    use super::*;

    #[test]
    fn test_should_explode_memberships_in_entity_input_order() {
        let entities = vec![entity("e2", 2, "BOB", 1), entity("e1", 1, "ALICE", 2)];
        let communities = vec![community(0, 0, vec!["e1", "e2"])];

        let exploded = explode_communities(&communities, &entities);

        assert_eq!(
            exploded
                .iter()
                .map(|row| row.entity.id.as_str())
                .collect::<Vec<_>>(),
            vec!["e2", "e1"]
        );
    }

    #[test]
    fn test_should_match_graphrag_csv_shape_and_newlines() {
        let tokenizer = TiktokenTokenizer::new("cl100k_base").expect("tokenizer");
        let contexts = build_local_contexts(
            &[community(0, 0, vec!["e1", "e2"])],
            &[
                entity("e1", 10, "ALICE", 7),
                entity_with_description("e2", 20, "BOB", 3, "quote \"and\", comma\nline"),
            ],
            &[relationship(30, "ALICE", "BOB", "works", 10)],
            &[],
            &tokenizer,
            8_000,
        )
        .expect("contexts");

        assert_eq!(
            contexts.get(&0).expect("context").context,
            "-----Entities-----\nhuman_readable_id,title,description,degree\n10,ALICE,ALICE \
             description,7\n20,BOB,\"quote \"\"and\"\", \
             comma\nline\",3\n\n\n-----Relationships-----\nhuman_readable_id,source,target,\
             description,combined_degree\n30,ALICE,BOB,works,10\n"
        );
    }

    #[test]
    fn test_should_use_first_source_edge_before_first_target_edge() {
        let tokenizer = WordCountTokenizer;
        let contexts = build_local_contexts(
            &[community(0, 0, vec!["e1", "e2", "e3"])],
            &[
                entity("e1", 1, "ALICE", 3),
                entity("e2", 2, "BOB", 2),
                entity("e3", 3, "CAROL", 1),
            ],
            &[
                relationship(10, "BOB", "ALICE", "target first", 5),
                relationship(11, "ALICE", "CAROL", "source wins", 4),
                relationship(12, "ALICE", "BOB", "later source", 20),
            ],
            &[],
            &tokenizer,
            1_000,
        )
        .expect("contexts");
        let context = &contexts.get(&0).expect("context").context;

        assert!(context.contains("10,BOB,ALICE,target first,5"));
        assert!(context.contains("11,ALICE,CAROL,source wins,4"));
        assert!(!context.contains("12,ALICE,BOB,later source,20"));
        assert!(!context.contains("-----Claims-----"));
    }

    #[test]
    fn test_should_return_empty_context_when_graphrag_has_no_selected_edge() {
        let tokenizer = WordCountTokenizer;
        let contexts = build_local_contexts(
            &[community(0, 0, vec!["e1"])],
            &[entity("e1", 1, "ALICE", 1)],
            &[],
            &[],
            &tokenizer,
            1_000,
        )
        .expect("contexts");

        assert!(contexts.get(&0).expect("context").context.is_empty());
    }

    #[test]
    fn test_should_keep_first_edge_when_it_alone_exceeds_limit_like_graphrag() {
        let tokenizer = WordCountTokenizer;
        let communities = [community(0, 0, vec!["e1", "e2"])];
        let entities = [
            entity_with_description("e1", 1, "ALICE", 2, "description with several words"),
            entity_with_description("e2", 2, "BOB", 1, "another long description"),
        ];
        let relationships = [relationship(
            10,
            "ALICE",
            "BOB",
            "relationship description",
            3,
        )];

        let constrained =
            build_local_contexts(&communities, &entities, &relationships, &[], &tokenizer, 1)
                .expect("constrained context");
        let unconstrained = build_local_contexts(
            &communities,
            &entities,
            &relationships,
            &[],
            &tokenizer,
            1_000,
        )
        .expect("unconstrained context");
        let constrained = &constrained.get(&0).expect("constrained community").context;
        let unconstrained = &unconstrained
            .get(&0)
            .expect("unconstrained community")
            .context;

        assert_eq!(constrained, unconstrained);
        assert!(tokenizer.count(constrained).expect("token count") > 1);
    }

    #[derive(Debug)]
    struct WordCountTokenizer;

    impl Tokenizer for WordCountTokenizer {
        fn count(&self, text: &str) -> graphloom_llm::Result<usize> {
            Ok(text.split_whitespace().count())
        }

        fn encode(&self, text: &str) -> graphloom_llm::Result<Vec<u32>> {
            Ok((0..text.split_whitespace().count())
                .map(|index| u32::try_from(index).unwrap_or(u32::MAX))
                .collect())
        }

        fn decode(&self, _tokens: &[u32]) -> graphloom_llm::Result<String> {
            Ok(String::new())
        }
    }

    fn entity(id: &str, human_readable_id: i64, title: &str, degree: i64) -> EntityContextRow {
        entity_with_description(
            id,
            human_readable_id,
            title,
            degree,
            &format!("{title} description"),
        )
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
            id: format!("r{human_readable_id}"),
            human_readable_id,
            source: source.to_owned(),
            target: target.to_owned(),
            description: description.to_owned(),
            combined_degree,
        }
    }

    fn community(community: i64, level: i64, entity_ids: Vec<&str>) -> CommunityInputRow {
        CommunityInputRow {
            community,
            level,
            parent: -1,
            children: Vec::new(),
            entity_ids: entity_ids.into_iter().map(str::to_owned).collect(),
            period: String::new(),
            size: 0,
        }
    }
}
