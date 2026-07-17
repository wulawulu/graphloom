//! Entity and relationship merge operations.

use std::collections::{BTreeSet, HashMap};

use super::{EntityRow, RawEntityRow, RawRelationshipRow, RelationshipRow};

pub(crate) fn merge_entities(rows: &[RawEntityRow]) -> Vec<EntityRow> {
    let mut indexes = HashMap::<(String, String), usize>::new();
    let mut grouped = Vec::<EntityRow>::new();
    for row in rows {
        let key = (row.title.clone(), row.entity_type.clone());
        if let Some(index) = indexes.get(&key).copied() {
            if let Some(entry) = grouped.get_mut(index) {
                entry.description.push(row.description.clone());
                entry.text_unit_ids.push(row.source_id.clone());
                entry.frequency = entry.frequency.saturating_add(1);
            }
        } else {
            indexes.insert(key, grouped.len());
            grouped.push(EntityRow {
                title: row.title.clone(),
                entity_type: row.entity_type.clone(),
                description: vec![row.description.clone()],
                text_unit_ids: vec![row.source_id.clone()],
                frequency: 1,
            });
        }
    }
    grouped
}

pub(crate) fn merge_relationships(rows: &[RawRelationshipRow]) -> Vec<RelationshipRow> {
    let mut indexes = HashMap::<(String, String), usize>::new();
    let mut grouped = Vec::<RelationshipRow>::new();
    for row in rows {
        let key = (row.source.clone(), row.target.clone());
        if let Some(index) = indexes.get(&key).copied() {
            if let Some(entry) = grouped.get_mut(index) {
                entry.description.push(row.description.clone());
                entry.text_unit_ids.push(row.source_id.clone());
                entry.weight += row.weight;
            }
        } else {
            indexes.insert(key, grouped.len());
            grouped.push(RelationshipRow {
                source: row.source.clone(),
                target: row.target.clone(),
                description: vec![row.description.clone()],
                text_unit_ids: vec![row.source_id.clone()],
                weight: row.weight,
            });
        }
    }
    grouped
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

    #[test]
    fn test_should_preserve_first_seen_group_order_like_graphrag() {
        let entities = merge_entities(&[
            raw_entity("ZED", "first"),
            raw_entity("ALICE", "second"),
            raw_entity("ZED", "third"),
        ]);
        assert_eq!(
            entities
                .iter()
                .map(|entity| entity.title.as_str())
                .collect::<Vec<_>>(),
            vec!["ZED", "ALICE"]
        );

        let relationships = merge_relationships(&[
            raw_relationship("ZED", "ALICE", 2.0),
            raw_relationship("ALICE", "ZED", 3.0),
            raw_relationship("ZED", "ALICE", 5.0),
        ]);
        assert_eq!(
            relationships
                .iter()
                .map(|relationship| (
                    relationship.source.as_str(),
                    relationship.target.as_str(),
                    relationship.weight,
                ))
                .collect::<Vec<_>>(),
            vec![("ZED", "ALICE", 7.0), ("ALICE", "ZED", 3.0)]
        );
    }

    fn raw_entity(title: &str, description: &str) -> RawEntityRow {
        RawEntityRow {
            title: title.to_owned(),
            entity_type: "person".to_owned(),
            description: description.to_owned(),
            source_id: "tu-1".to_owned(),
        }
    }

    fn raw_relationship(source: &str, target: &str, weight: f64) -> RawRelationshipRow {
        RawRelationshipRow {
            source: source.to_owned(),
            target: target.to_owned(),
            description: format!("{source} -> {target}"),
            source_id: "tu-1".to_owned(),
            weight,
        }
    }
}
