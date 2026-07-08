//! Final graph row construction operations.

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use super::{FinalEntityRow, FinalRelationshipRow, SummarizedEntityRow, SummarizedRelationshipRow};

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
) -> Vec<FinalEntityRow> {
    let mut seen = BTreeSet::new();
    let mut final_rows = Vec::new();
    for row in rows {
        if !seen.insert(row.title.clone()) {
            continue;
        }
        final_rows.push(FinalEntityRow {
            id: Uuid::new_v4().to_string(),
            human_readable_id: final_rows.len(),
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
    final_rows
}

pub(crate) fn finalize_relationships(
    rows: &[SummarizedRelationshipRow],
    degree_map: &BTreeMap<String, i64>,
) -> Vec<FinalRelationshipRow> {
    let mut seen = BTreeSet::new();
    let mut final_rows = Vec::new();
    for row in rows {
        let key = (row.source.clone(), row.target.clone());
        if !seen.insert(key.clone()) {
            continue;
        }
        final_rows.push(FinalRelationshipRow {
            id: Uuid::new_v4().to_string(),
            human_readable_id: final_rows.len(),
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
    final_rows
}

fn sorted_pair(left: &str, right: &str) -> (String, String) {
    if left <= right {
        (left.to_owned(), right.to_owned())
    } else {
        (right.to_owned(), left.to_owned())
    }
}
