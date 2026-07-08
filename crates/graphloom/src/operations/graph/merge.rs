//! Entity and relationship merge operations.

use std::collections::{BTreeMap, BTreeSet};

use super::{EntityRow, RawEntityRow, RawRelationshipRow, RelationshipRow};

pub(crate) fn merge_entities(rows: &[RawEntityRow]) -> Vec<EntityRow> {
    let mut grouped: BTreeMap<(String, String), EntityRow> = BTreeMap::new();
    for row in rows {
        let key = (row.title.clone(), row.entity_type.clone());
        let entry = grouped.entry(key).or_insert_with(|| EntityRow {
            title: row.title.clone(),
            entity_type: row.entity_type.clone(),
            description: Vec::new(),
            text_unit_ids: Vec::new(),
            frequency: 0,
        });
        entry.description.push(row.description.clone());
        entry.text_unit_ids.push(row.source_id.clone());
        entry.frequency = entry.frequency.saturating_add(1);
    }
    grouped.into_values().collect()
}

pub(crate) fn merge_relationships(rows: &[RawRelationshipRow]) -> Vec<RelationshipRow> {
    let mut grouped: BTreeMap<(String, String), RelationshipRow> = BTreeMap::new();
    for row in rows {
        let key = (row.source.clone(), row.target.clone());
        let entry = grouped.entry(key).or_insert_with(|| RelationshipRow {
            source: row.source.clone(),
            target: row.target.clone(),
            description: Vec::new(),
            text_unit_ids: Vec::new(),
            weight: 0.0,
        });
        entry.description.push(row.description.clone());
        entry.text_unit_ids.push(row.source_id.clone());
        entry.weight += row.weight;
    }
    grouped.into_values().collect()
}

pub(crate) fn filter_orphan_relationships(
    relationships: Vec<RelationshipRow>,
    entities: &[EntityRow],
) -> Vec<RelationshipRow> {
    let titles = entities
        .iter()
        .map(|entity| entity.title.as_str())
        .collect::<BTreeSet<_>>();
    relationships
        .into_iter()
        .filter(|relationship| {
            titles.contains(relationship.source.as_str())
                && titles.contains(relationship.target.as_str())
        })
        .collect()
}
