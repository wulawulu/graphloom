//! Graph context construction for community reports.

use std::collections::{BTreeMap, BTreeSet};

use csv::WriterBuilder;
use graphloom_llm::Tokenizer;

use super::{
    ClaimContextRow, CommunityInputRow, CommunityLocalContext, ContextRecords, EntityContextRow,
    ExplodedEntityRow, RelationshipContextRow, ReportContextRow,
};
use crate::{Result, dataframe::invalid_data};

const COMMUNITY_REPORTS_CONTEXT: &str = "create_community_reports";
const NO_DESCRIPTION: &str = "No Description";

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

    if ordered_relationships.is_empty() {
        return build_entity_only_context(
            community,
            entities,
            &claims_by_subject,
            tokenizer,
            max_input_length,
        );
    }

    let full_records =
        full_relationship_records(&ordered_relationships, &entity_by_title, &claims_by_subject);
    let (full_text, full_token_count) = render_and_count(&full_records, tokenizer)?;
    let (records, context, token_count) = trim_relationship_records(
        ordered_relationships,
        &entity_by_title,
        &claims_by_subject,
        tokenizer,
        max_input_length,
    )?;

    let was_truncated = full_token_count > max_input_length || full_text != context;
    Ok(CommunityLocalContext {
        community,
        full_records,
        records,
        context,
        token_count,
        full_token_count,
        was_truncated,
    })
}

fn full_relationship_records(
    relationships: &[RelationshipContextRow],
    entity_by_title: &BTreeMap<&str, &EntityContextRow>,
    claims_by_subject: &BTreeMap<&str, Vec<&ClaimContextRow>>,
) -> ContextRecords {
    let mut records = ContextRecords::default();
    for relationship in relationships {
        add_entity_and_claims(
            &mut records,
            relationship.source.as_str(),
            entity_by_title,
            claims_by_subject,
        );
        add_entity_and_claims(
            &mut records,
            relationship.target.as_str(),
            entity_by_title,
            claims_by_subject,
        );
        records.add_relationship(relationship.clone());
    }
    records
}

fn trim_relationship_records(
    relationships: Vec<RelationshipContextRow>,
    entity_by_title: &BTreeMap<&str, &EntityContextRow>,
    claims_by_subject: &BTreeMap<&str, Vec<&ClaimContextRow>>,
    tokenizer: &dyn Tokenizer,
    max_input_length: usize,
) -> Result<(ContextRecords, String, usize)> {
    let mut committed = ContextRecords::default();
    let mut committed_text = String::new();
    let mut committed_tokens = tokenizer.count(&committed_text)?;

    for relationship in relationships {
        let mut candidate = committed.clone();
        add_entity_and_claims(
            &mut candidate,
            relationship.source.as_str(),
            entity_by_title,
            claims_by_subject,
        );
        add_entity_and_claims(
            &mut candidate,
            relationship.target.as_str(),
            entity_by_title,
            claims_by_subject,
        );
        candidate.add_relationship(relationship.clone());
        if commit_if_within_limit(
            &candidate,
            &mut committed,
            &mut committed_text,
            &mut committed_tokens,
            tokenizer,
            max_input_length,
        )? {
            continue;
        }

        try_add_entity(
            relationship.source.as_str(),
            entity_by_title,
            &mut committed,
            &mut committed_text,
            &mut committed_tokens,
            tokenizer,
            max_input_length,
        )?;
        try_add_claims(
            relationship.source.as_str(),
            claims_by_subject,
            &mut committed,
            &mut committed_text,
            &mut committed_tokens,
            tokenizer,
            max_input_length,
        )?;
        try_add_entity(
            relationship.target.as_str(),
            entity_by_title,
            &mut committed,
            &mut committed_text,
            &mut committed_tokens,
            tokenizer,
            max_input_length,
        )?;
        try_add_claims(
            relationship.target.as_str(),
            claims_by_subject,
            &mut committed,
            &mut committed_text,
            &mut committed_tokens,
            tokenizer,
            max_input_length,
        )?;
        let mut relationship_candidate = committed.clone();
        relationship_candidate.add_relationship(relationship);
        let _ = commit_if_within_limit(
            &relationship_candidate,
            &mut committed,
            &mut committed_text,
            &mut committed_tokens,
            tokenizer,
            max_input_length,
        )?;
    }
    Ok((committed, committed_text, committed_tokens))
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
    let mut full_records = ContextRecords::default();
    for entity in entities {
        add_entity_and_claims(
            &mut full_records,
            entity.title.as_str(),
            &entity_by_title,
            claims_by_subject,
        );
    }
    let (full_text, full_token_count) = render_and_count(&full_records, tokenizer)?;

    let mut committed = ContextRecords::default();
    let mut committed_text = String::new();
    let mut committed_tokens = tokenizer.count(&committed_text)?;

    for entity in entities {
        let mut candidate = committed.clone();
        add_entity_and_claims(
            &mut candidate,
            entity.title.as_str(),
            &entity_by_title,
            claims_by_subject,
        );
        if commit_if_within_limit(
            &candidate,
            &mut committed,
            &mut committed_text,
            &mut committed_tokens,
            tokenizer,
            max_input_length,
        )? {
            continue;
        }

        try_add_entity(
            entity.title.as_str(),
            &entity_by_title,
            &mut committed,
            &mut committed_text,
            &mut committed_tokens,
            tokenizer,
            max_input_length,
        )?;
        if committed.entity_ids.contains(&entity.human_readable_id) {
            try_add_claims(
                entity.title.as_str(),
                claims_by_subject,
                &mut committed,
                &mut committed_text,
                &mut committed_tokens,
                tokenizer,
                max_input_length,
            )?;
        }
    }

    let was_truncated = full_token_count > max_input_length || full_text != committed_text;
    Ok(CommunityLocalContext {
        community,
        full_records,
        records: committed,
        context: committed_text,
        token_count: committed_tokens,
        full_token_count,
        was_truncated,
    })
}

fn add_entity_and_claims<'a>(
    records: &mut ContextRecords,
    title: &str,
    entity_by_title: &BTreeMap<&str, &'a EntityContextRow>,
    claims_by_subject: &BTreeMap<&str, Vec<&'a ClaimContextRow>>,
) {
    if let Some(entity) = entity_by_title.get(title) {
        records.add_entity((*entity).clone());
    }
    if let Some(claims) = claims_by_subject.get(title) {
        for claim in claims {
            records.add_claim((*claim).clone());
        }
    }
}

fn try_add_entity(
    title: &str,
    entity_by_title: &BTreeMap<&str, &EntityContextRow>,
    committed: &mut ContextRecords,
    committed_text: &mut String,
    committed_tokens: &mut usize,
    tokenizer: &dyn Tokenizer,
    max_input_length: usize,
) -> Result<()> {
    let Some(entity) = entity_by_title.get(title) else {
        return Ok(());
    };
    let mut candidate = committed.clone();
    candidate.add_entity((*entity).clone());
    let _ = commit_if_within_limit(
        &candidate,
        committed,
        committed_text,
        committed_tokens,
        tokenizer,
        max_input_length,
    )?;
    Ok(())
}

fn try_add_claims(
    title: &str,
    claims_by_subject: &BTreeMap<&str, Vec<&ClaimContextRow>>,
    committed: &mut ContextRecords,
    committed_text: &mut String,
    committed_tokens: &mut usize,
    tokenizer: &dyn Tokenizer,
    max_input_length: usize,
) -> Result<()> {
    let Some(claims) = claims_by_subject.get(title) else {
        return Ok(());
    };
    for claim in claims {
        let mut candidate = committed.clone();
        candidate.add_claim((*claim).clone());
        let _ = commit_if_within_limit(
            &candidate,
            committed,
            committed_text,
            committed_tokens,
            tokenizer,
            max_input_length,
        )?;
    }
    Ok(())
}

fn commit_if_within_limit(
    candidate: &ContextRecords,
    committed: &mut ContextRecords,
    committed_text: &mut String,
    committed_tokens: &mut usize,
    tokenizer: &dyn Tokenizer,
    max_input_length: usize,
) -> Result<bool> {
    let candidate_text = render_context(candidate)?;
    let candidate_tokens = tokenizer.count(&candidate_text)?;
    if candidate_tokens > max_input_length {
        return Ok(false);
    }
    *committed = candidate.clone();
    *committed_text = candidate_text;
    *committed_tokens = candidate_tokens;
    Ok(true)
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

pub(crate) fn render_context(records: &ContextRecords) -> Result<String> {
    let reports = if records.reports.is_empty() {
        String::new()
    } else {
        render_reports_csv(&records.reports)?
    };
    let entities = if records.entities.is_empty() {
        String::new()
    } else {
        render_entities_csv(&records.entities)?
    };
    let claims = if records.claims.is_empty() {
        String::new()
    } else {
        render_claims_csv(&records.claims)?
    };
    let relationships = if records.relationships.is_empty() {
        String::new()
    } else {
        render_relationships_csv(&records.relationships)?
    };

    Ok([reports, entities, claims, relationships]
        .into_iter()
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>()
        .join("\n"))
}

fn render_reports_csv(rows: &[ReportContextRow]) -> Result<String> {
    render_section(
        "----Reports-----",
        &["community", "full_content"],
        rows.iter()
            .map(|report| vec![report.community.to_string(), report.full_content.clone()])
            .collect(),
    )
}

fn render_entities_csv(rows: &[EntityContextRow]) -> Result<String> {
    render_section(
        "-----Entities-----",
        &["human_readable_id", "title", "description"],
        rows.iter()
            .map(|entity| {
                vec![
                    entity.human_readable_id.to_string(),
                    entity.title.clone(),
                    description_or_default(&entity.description),
                ]
            })
            .collect(),
    )
}

fn render_claims_csv(rows: &[ClaimContextRow]) -> Result<String> {
    render_section(
        "-----Claims-----",
        &[
            "human_readable_id",
            "subject_id",
            "type",
            "status",
            "description",
        ],
        rows.iter()
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
    )
}

fn render_relationships_csv(rows: &[RelationshipContextRow]) -> Result<String> {
    render_section(
        "-----Relationships-----",
        &["human_readable_id", "source", "target", "description"],
        rows.iter()
            .map(|relationship| {
                vec![
                    relationship.human_readable_id.to_string(),
                    relationship.source.clone(),
                    relationship.target.clone(),
                    description_or_default(&relationship.description),
                ]
            })
            .collect(),
    )
}

fn render_and_count(
    records: &ContextRecords,
    tokenizer: &dyn Tokenizer,
) -> Result<(String, usize)> {
    let text = render_context(records)?;
    let count = tokenizer.count(&text)?;
    Ok((text, count))
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
    use graphloom_llm::{TiktokenTokenizer, Tokenizer};

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
            relationship_with_degree(2, "ALICE", "BOB", "low", 8),
            relationship_with_degree(1, "BOB", "CAROL", "quote \"and\", comma\nline", 9),
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

    #[test]
    fn test_should_preserve_relationship_degree_priority_over_id_order() {
        let tokenizer = WordCountTokenizer;
        let contexts = build_local_contexts(
            &[community(0, 0, vec!["e1", "e2", "e3"])],
            &[
                entity("e1", 10, "ALICE", 1),
                entity("e2", 11, "BOB", 1),
                entity("e3", 12, "CAROL", 1),
            ],
            &[
                relationship_with_degree(1, "BOB", "CAROL", "low", 2),
                relationship_with_degree(100, "ALICE", "BOB", "high", 20),
            ],
            &[],
            &tokenizer,
            1_000,
        )
        .expect("contexts");

        let context = &contexts.get(&0).expect("community").context;

        assert!(
            context.find("100,ALICE,BOB").expect("relationship 100")
                < context.find("1,BOB,CAROL").expect("relationship 1")
        );
    }

    #[test]
    fn test_should_preserve_entity_only_degree_priority_over_id_order() {
        let tokenizer = WordCountTokenizer;
        let contexts = build_local_contexts(
            &[community(0, 0, vec!["e1", "e2"])],
            &[entity("e1", 1, "LOW", 2), entity("e2", 100, "HIGH", 20)],
            &[],
            &[],
            &tokenizer,
            1_000,
        )
        .expect("contexts");

        let context = &contexts.get(&0).expect("community").context;

        assert!(context.find("100,HIGH").expect("high") < context.find("1,LOW").expect("low"));
    }

    #[test]
    fn test_should_deduplicate_without_losing_first_insert_order() {
        let tokenizer = WordCountTokenizer;
        let contexts = build_local_contexts(
            &[community(0, 0, vec!["e1", "e2", "e3"])],
            &[
                entity("e1", 10, "ALICE", 1),
                entity("e2", 20, "BOB", 1),
                entity("e3", 30, "CAROL", 1),
            ],
            &[
                relationship_with_degree(100, "ALICE", "BOB", "first", 20),
                relationship_with_degree(100, "ALICE", "BOB", "duplicate", 19),
                relationship_with_degree(1, "BOB", "CAROL", "second", 2),
            ],
            &[
                claim(7, "ALICE", "alpha"),
                claim(7, "ALICE", "duplicate alpha"),
                claim(8, "BOB", "beta"),
            ],
            &tokenizer,
            1_000,
        )
        .expect("contexts");

        let context = &contexts.get(&0).expect("community").context;

        assert_eq!(context.matches("10,ALICE").count(), 1);
        assert_eq!(context.matches("7,ALICE,TYPE").count(), 1);
        assert_eq!(context.matches("100,ALICE,BOB").count(), 1);
        assert!(
            context.find("100,ALICE,BOB").expect("relationship 100")
                < context.find("1,BOB,CAROL").expect("relationship 1")
        );
    }

    #[test]
    fn test_should_not_commit_oversized_first_relationship_bundle() {
        let tokenizer = WordCountTokenizer;
        let contexts = build_local_contexts(
            &[community(0, 0, vec!["e1", "e2"])],
            &[
                entity_with_description("e1", 10, "ALICE", 1, "small"),
                entity_with_description("e2", 20, "BOB", 1, "tiny"),
            ],
            &[relationship_with_degree(
                100,
                "ALICE",
                "BOB",
                "one two three four five six seven eight",
                20,
            )],
            &[],
            &tokenizer,
            4,
        )
        .expect("contexts");
        let local = contexts.get(&0).expect("community");

        assert!(local.token_count <= 4);
        assert!(
            !local
                .context
                .contains("one two three four five six seven eight")
        );
        assert!(local.context.is_char_boundary(local.context.len()));
    }

    #[test]
    fn test_should_skip_oversized_first_entity_and_continue() {
        let tokenizer = WordCountTokenizer;
        let contexts = build_local_contexts(
            &[community(0, 0, vec!["e1", "e2"])],
            &[
                entity_with_description("e1", 100, "BIG", 20, "one two three four five"),
                entity_with_description("e2", 1, "SMALL", 2, "tiny"),
            ],
            &[],
            &[],
            &tokenizer,
            9,
        )
        .expect("contexts");
        let local = contexts.get(&0).expect("community");

        assert!(local.token_count <= 9);
        assert!(!local.context.contains("BIG"));
        assert!(local.context.contains("SMALL"));
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
        entity_with_description(id, human_readable_id, title, degree, "desc")
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

    fn relationship_with_degree(
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

    fn claim(human_readable_id: i64, subject_id: &str, description: &str) -> ClaimContextRow {
        ClaimContextRow {
            human_readable_id,
            subject_id: subject_id.to_owned(),
            claim_type: "TYPE".to_owned(),
            status: "TRUE".to_owned(),
            description: description.to_owned(),
        }
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
