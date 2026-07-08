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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_merge_entities_and_filter_orphan_relationships() {
        let raw_entities = vec![
            RawEntityRow {
                title: "ALICE".to_owned(),
                entity_type: "person".to_owned(),
                description: "engineer".to_owned(),
                source_id: "tu-1".to_owned(),
            },
            RawEntityRow {
                title: "ALICE".to_owned(),
                entity_type: "person".to_owned(),
                description: "mentor".to_owned(),
                source_id: "tu-2".to_owned(),
            },
            RawEntityRow {
                title: "BOB".to_owned(),
                entity_type: "person".to_owned(),
                description: "researcher".to_owned(),
                source_id: "tu-1".to_owned(),
            },
        ];
        let entities = merge_entities(&raw_entities);

        assert_eq!(entities.len(), 2);
        assert_eq!(entities[0].frequency, 2);
        assert_eq!(entities[0].description, vec!["engineer", "mentor"]);

        let relationships = merge_relationships(&[
            RawRelationshipRow {
                source: "ALICE".to_owned(),
                target: "BOB".to_owned(),
                description: "works with".to_owned(),
                source_id: "tu-1".to_owned(),
                weight: 2.0,
            },
            RawRelationshipRow {
                source: "ALICE".to_owned(),
                target: "CAROL".to_owned(),
                description: "missing endpoint".to_owned(),
                source_id: "tu-2".to_owned(),
                weight: 1.0,
            },
        ]);
        let relationships = filter_orphan_relationships(relationships, &entities);

        assert_eq!(relationships.len(), 1);
        assert_eq!(relationships[0].source, "ALICE");
        assert_eq!(relationships[0].target, "BOB");
    }
}
