//! Final graph row construction operations.

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use super::{
    FinalEntityRow, FinalRelationshipRow, FinalizedGraph, SummarizedEntityRow,
    SummarizedRelationshipRow,
};
use crate::{Result, dataframe::usize_to_i64};

const FINALIZE_GRAPH_WORKFLOW: &str = "finalize_graph";

pub(crate) fn finalize_graph(
    entities: &[SummarizedEntityRow],
    relationships: &[SummarizedRelationshipRow],
) -> Result<FinalizedGraph> {
    let degree = degree_map(relationships);
    Ok(FinalizedGraph {
        entities: finalize_entities(entities, &degree)?,
        relationships: finalize_relationships(relationships, &degree)?,
    })
}

pub(crate) fn degree_map(rows: &[SummarizedRelationshipRow]) -> BTreeMap<String, i64> {
    let mut seen = BTreeSet::new();
    let mut degree = BTreeMap::new();
    for row in rows {
        let (left, right) = sorted_pair(&row.source, &row.target);
        if seen.insert((left.clone(), right.clone())) {
            *degree.entry(left).or_insert(0) += 1;
            *degree.entry(right).or_insert(0) += 1;
        }
    }
    degree
}

pub(crate) fn finalize_entities(
    rows: &[SummarizedEntityRow],
    degree_map: &BTreeMap<String, i64>,
) -> Result<Vec<FinalEntityRow>> {
    let mut seen = BTreeSet::new();
    let mut final_rows = Vec::new();
    for row in rows {
        if !seen.insert(row.title.clone()) {
            continue;
        }
        final_rows.push(FinalEntityRow {
            id: Uuid::new_v4().to_string(),
            human_readable_id: usize_to_i64(
                final_rows.len(),
                FINALIZE_GRAPH_WORKFLOW,
                "human_readable_id",
            )?,
            title: row.title.clone(),
            entity_type: row.entity_type.clone(),
            description: row.description.clone(),
            text_unit_ids: row.text_unit_ids.clone(),
            frequency: row.frequency,
            degree: degree_map
                .get(&row.title)
                .copied()
                .map_or(0, |degree| degree),
        });
    }
    Ok(final_rows)
}

pub(crate) fn finalize_relationships(
    rows: &[SummarizedRelationshipRow],
    degree_map: &BTreeMap<String, i64>,
) -> Result<Vec<FinalRelationshipRow>> {
    let mut seen = BTreeSet::new();
    let mut final_rows = Vec::new();
    for row in rows {
        let key = (row.source.clone(), row.target.clone());
        if !seen.insert(key.clone()) {
            continue;
        }
        final_rows.push(FinalRelationshipRow {
            id: Uuid::new_v4().to_string(),
            human_readable_id: usize_to_i64(
                final_rows.len(),
                FINALIZE_GRAPH_WORKFLOW,
                "human_readable_id",
            )?,
            source: row.source.clone(),
            target: row.target.clone(),
            description: row.description.clone(),
            weight: row.weight,
            combined_degree: degree_map
                .get(&row.source)
                .copied()
                .map_or(0, |degree| degree)
                .saturating_add(
                    degree_map
                        .get(&row.target)
                        .copied()
                        .map_or(0, |degree| degree),
                ),
            text_unit_ids: row.text_unit_ids.clone(),
        });
    }
    Ok(final_rows)
}

fn sorted_pair(left: &str, right: &str) -> (String, String) {
    if left <= right {
        (left.to_owned(), right.to_owned())
    } else {
        (right.to_owned(), left.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_count_reverse_edges_once_in_degree_map() {
        let degree = degree_map(&[
            SummarizedRelationshipRow {
                source: "ALICE".to_owned(),
                target: "BOB".to_owned(),
                description: "one".to_owned(),
                text_unit_ids: vec!["tu-1".to_owned()],
                weight: 1.0,
            },
            SummarizedRelationshipRow {
                source: "BOB".to_owned(),
                target: "ALICE".to_owned(),
                description: "reverse".to_owned(),
                text_unit_ids: vec!["tu-2".to_owned()],
                weight: 1.0,
            },
        ]);

        assert_eq!(degree.get("ALICE"), Some(&1));
        assert_eq!(degree.get("BOB"), Some(&1));
    }

    #[test]
    fn test_should_finalize_entities_and_relationships_with_shared_degree_map() {
        let entities = vec![
            summarized_entity("ALICE"),
            summarized_entity("BOB"),
            summarized_entity("BOB"),
            summarized_entity("CAROL"),
        ];
        let relationships = vec![
            summarized_relationship("ALICE", "BOB"),
            summarized_relationship("ALICE", "BOB"),
            summarized_relationship("BOB", "CAROL"),
        ];

        let graph = finalize_graph(&entities, &relationships).expect("finalized graph");

        assert_eq!(graph.entities.len(), 3);
        assert_eq!(graph.relationships.len(), 2);
        assert_eq!(
            graph
                .entities
                .iter()
                .map(|row| (row.title.as_str(), row.degree, row.human_readable_id))
                .collect::<Vec<_>>(),
            vec![("ALICE", 1, 0), ("BOB", 2, 1), ("CAROL", 1, 2)]
        );
        assert_eq!(
            graph
                .relationships
                .iter()
                .map(|row| (
                    row.source.as_str(),
                    row.target.as_str(),
                    row.combined_degree,
                    row.human_readable_id
                ))
                .collect::<Vec<_>>(),
            vec![("ALICE", "BOB", 3, 0), ("BOB", "CAROL", 3, 1)]
        );
    }

    fn summarized_entity(title: &str) -> SummarizedEntityRow {
        SummarizedEntityRow {
            title: title.to_owned(),
            entity_type: "person".to_owned(),
            description: title.to_owned(),
            text_unit_ids: vec!["tu-1".to_owned()],
            frequency: 1,
        }
    }

    fn summarized_relationship(source: &str, target: &str) -> SummarizedRelationshipRow {
        SummarizedRelationshipRow {
            source: source.to_owned(),
            target: target.to_owned(),
            description: format!("{source} to {target}"),
            text_unit_ids: vec!["tu-1".to_owned()],
            weight: 1.0,
        }
    }
}
