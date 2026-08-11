//! Deterministic, bounded visualization projections over full graph snapshots.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

use thiserror::Error;

use super::{
    GraphDataSnapshot, GraphEntityDetail, GraphProjection, GraphProjectionEntity,
    GraphProjectionRelationship, GraphRelationshipDetail,
};

/// Projection construction failure caused by seed-preservation limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum GraphProjectionError {
    /// Valid seeds alone exceed the requested projection limits.
    #[error("graph subgraph limits are too small for the requested seeds")]
    SeedLimitsTooSmall,
}

#[derive(Debug)]
struct ResolvedRelationship<'a> {
    relationship: &'a GraphRelationshipDetail,
    source_entity_id: &'a str,
    target_entity_id: &'a str,
}

#[derive(Debug)]
struct ResolutionIndex<'a> {
    entities_by_id: BTreeMap<&'a str, &'a GraphEntityDetail>,
    relationships_by_id: BTreeMap<&'a str, &'a GraphRelationshipDetail>,
    resolved_by_id: BTreeMap<&'a str, usize>,
    resolved_relationships: Vec<ResolvedRelationship<'a>>,
    adjacency: BTreeMap<&'a str, Vec<usize>>,
    unresolved_relationship_count: u64,
}

#[derive(Debug)]
struct SeedSelection<'a> {
    entity_ids: BTreeSet<&'a str>,
    relationship_indexes: Vec<usize>,
    missing_entity_ids: Vec<String>,
    missing_relationship_ids: Vec<String>,
    unresolved_relationship_ids: Vec<String>,
}

impl<'a> ResolutionIndex<'a> {
    fn new(snapshot: &'a GraphDataSnapshot) -> Self {
        let entities_by_id = snapshot
            .entities
            .iter()
            .map(|entity| (entity.id.as_str(), entity))
            .collect::<BTreeMap<_, _>>();
        let relationships_by_id = snapshot
            .relationships
            .iter()
            .map(|relationship| (relationship.id.as_str(), relationship))
            .collect::<BTreeMap<_, _>>();
        let mut entities_by_title = BTreeMap::<&str, Vec<&GraphEntityDetail>>::new();
        for entity in &snapshot.entities {
            entities_by_title
                .entry(entity.title.as_str())
                .or_default()
                .push(entity);
        }

        let mut resolved_relationships = Vec::with_capacity(snapshot.relationships.len());
        for relationship in &snapshot.relationships {
            let Some([source]) = entities_by_title
                .get(relationship.source.as_str())
                .map(Vec::as_slice)
            else {
                continue;
            };
            let Some([target]) = entities_by_title
                .get(relationship.target.as_str())
                .map(Vec::as_slice)
            else {
                continue;
            };
            resolved_relationships.push(ResolvedRelationship {
                relationship,
                source_entity_id: source.id.as_str(),
                target_entity_id: target.id.as_str(),
            });
        }
        resolved_relationships.sort_by(compare_relationship_priority);

        let mut resolved_by_id = BTreeMap::new();
        let mut adjacency = BTreeMap::<&str, Vec<usize>>::new();
        for (index, relationship) in resolved_relationships.iter().enumerate() {
            resolved_by_id.insert(relationship.relationship.id.as_str(), index);
            adjacency
                .entry(relationship.source_entity_id)
                .or_default()
                .push(index);
            if relationship.source_entity_id != relationship.target_entity_id {
                adjacency
                    .entry(relationship.target_entity_id)
                    .or_default()
                    .push(index);
            }
        }

        Self {
            entities_by_id,
            relationships_by_id,
            resolved_by_id,
            unresolved_relationship_count: u64::try_from(
                snapshot
                    .relationships
                    .len()
                    .saturating_sub(resolved_relationships.len()),
            )
            .unwrap_or(u64::MAX),
            resolved_relationships,
            adjacency,
        }
    }
}

fn additional_endpoint_count(
    entity_ids: &BTreeSet<&str>,
    source_entity_id: &str,
    target_entity_id: &str,
) -> usize {
    let source_is_missing = !entity_ids.contains(source_entity_id);
    if source_entity_id == target_entity_id {
        return usize::from(source_is_missing);
    }
    usize::from(source_is_missing)
        .saturating_add(usize::from(!entity_ids.contains(target_entity_id)))
}

/// Build an edge-first overview from resolved full-snapshot topology.
pub(crate) fn overview(
    snapshot: &GraphDataSnapshot,
    max_entities: usize,
    max_relationships: usize,
) -> GraphProjection {
    let index = ResolutionIndex::new(snapshot);
    if index.resolved_relationships.is_empty() {
        return entity_fallback(&index, max_entities);
    }

    let mut entity_ids = BTreeSet::new();
    let mut selected_relationships = Vec::new();
    let mut truncated = false;
    for relationship in &index.resolved_relationships {
        let additional_entities = additional_endpoint_count(
            &entity_ids,
            relationship.source_entity_id,
            relationship.target_entity_id,
        );
        if selected_relationships.len() >= max_relationships
            || entity_ids.len().saturating_add(additional_entities) > max_entities
        {
            truncated = true;
            continue;
        }
        entity_ids.insert(relationship.source_entity_id);
        entity_ids.insert(relationship.target_entity_id);
        selected_relationships.push(relationship);
    }

    GraphProjection {
        entities: projection_entities(&index, entity_ids.iter().copied()),
        relationships: selected_relationships
            .into_iter()
            .map(projection_relationship)
            .collect(),
        seed_entity_ids: Vec::new(),
        seed_relationship_ids: Vec::new(),
        missing_entity_ids: Vec::new(),
        missing_relationship_ids: Vec::new(),
        unresolved_relationship_ids: Vec::new(),
        unresolved_relationship_count: index.unresolved_relationship_count,
        truncated,
    }
}

/// Build a seed-preserving depth-zero or depth-one subgraph.
pub(crate) fn subgraph(
    snapshot: &GraphDataSnapshot,
    requested_entity_ids: &[String],
    requested_relationship_ids: &[String],
    depth: u8,
    max_entities: usize,
    max_relationships: usize,
) -> Result<GraphProjection, GraphProjectionError> {
    let index = ResolutionIndex::new(snapshot);
    let seeds = resolve_seeds(&index, requested_entity_ids, requested_relationship_ids);
    if seeds.entity_ids.len() > max_entities || seeds.relationship_indexes.len() > max_relationships
    {
        return Err(GraphProjectionError::SeedLimitsTooSmall);
    }
    let (entity_ids, relationship_indexes, truncated) =
        expand_subgraph(&index, &seeds, depth, max_entities, max_relationships);

    let neighbor_entity_ids = entity_ids
        .difference(&seeds.entity_ids)
        .copied()
        .collect::<Vec<_>>();
    let mut entities = projection_entities(&index, seeds.entity_ids.iter().copied());
    entities.extend(projection_entities(
        &index,
        neighbor_entity_ids.iter().copied(),
    ));

    let relationships = relationship_indexes
        .iter()
        .map(|relationship_index| {
            projection_relationship(&index.resolved_relationships[*relationship_index])
        })
        .collect();

    Ok(GraphProjection {
        entities,
        relationships,
        seed_entity_ids: seeds.entity_ids.into_iter().map(str::to_owned).collect(),
        seed_relationship_ids: seeds
            .relationship_indexes
            .iter()
            .map(|relationship_index| {
                index.resolved_relationships[*relationship_index]
                    .relationship
                    .id
                    .clone()
            })
            .collect(),
        missing_entity_ids: seeds.missing_entity_ids,
        missing_relationship_ids: seeds.missing_relationship_ids,
        unresolved_relationship_ids: seeds.unresolved_relationship_ids,
        unresolved_relationship_count: index.unresolved_relationship_count,
        truncated,
    })
}

fn resolve_seeds<'a>(
    index: &ResolutionIndex<'a>,
    requested_entity_ids: &[String],
    requested_relationship_ids: &[String],
) -> SeedSelection<'a> {
    let mut selection = SeedSelection {
        entity_ids: BTreeSet::new(),
        relationship_indexes: Vec::new(),
        missing_entity_ids: Vec::new(),
        missing_relationship_ids: Vec::new(),
        unresolved_relationship_ids: Vec::new(),
    };
    for id in requested_entity_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
    {
        if let Some(entity) = index.entities_by_id.get(id) {
            selection.entity_ids.insert(entity.id.as_str());
        } else {
            selection.missing_entity_ids.push(id.to_owned());
        }
    }
    for id in requested_relationship_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
    {
        if !index.relationships_by_id.contains_key(id) {
            selection.missing_relationship_ids.push(id.to_owned());
        } else if let Some(resolved_index) = index.resolved_by_id.get(id).copied() {
            let relationship = &index.resolved_relationships[resolved_index];
            selection.entity_ids.insert(relationship.source_entity_id);
            selection.entity_ids.insert(relationship.target_entity_id);
            selection.relationship_indexes.push(resolved_index);
        } else {
            selection.unresolved_relationship_ids.push(id.to_owned());
        }
    }
    selection.relationship_indexes.sort_by(|left, right| {
        index.resolved_relationships[*left]
            .relationship
            .id
            .cmp(&index.resolved_relationships[*right].relationship.id)
    });
    selection
}

fn expand_subgraph<'a>(
    index: &ResolutionIndex<'a>,
    seeds: &SeedSelection<'a>,
    depth: u8,
    max_entities: usize,
    max_relationships: usize,
) -> (BTreeSet<&'a str>, Vec<usize>, bool) {
    let mut entity_ids = seeds.entity_ids.clone();
    let mut relationship_indexes = seeds.relationship_indexes.clone();
    if depth == 0 {
        return (entity_ids, relationship_indexes, false);
    }
    let seed_relationship_ids = seeds
        .relationship_indexes
        .iter()
        .map(|relationship_index| {
            index.resolved_relationships[*relationship_index]
                .relationship
                .id
                .as_str()
        })
        .collect::<BTreeSet<_>>();
    let mut candidates = seeds
        .entity_ids
        .iter()
        .filter_map(|id| index.adjacency.get(id))
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|relationship_index| {
            !seed_relationship_ids.contains(
                index.resolved_relationships[*relationship_index]
                    .relationship
                    .id
                    .as_str(),
            )
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        compare_relationship_priority(
            &index.resolved_relationships[*left],
            &index.resolved_relationships[*right],
        )
    });

    let mut truncated = false;
    for relationship_index in candidates {
        let relationship = &index.resolved_relationships[relationship_index];
        let additional_entities = additional_endpoint_count(
            &entity_ids,
            relationship.source_entity_id,
            relationship.target_entity_id,
        );
        if relationship_indexes.len() >= max_relationships
            || entity_ids.len().saturating_add(additional_entities) > max_entities
        {
            truncated = true;
            continue;
        }
        entity_ids.insert(relationship.source_entity_id);
        entity_ids.insert(relationship.target_entity_id);
        relationship_indexes.push(relationship_index);
    }
    (entity_ids, relationship_indexes, truncated)
}

fn entity_fallback(index: &ResolutionIndex<'_>, max_entities: usize) -> GraphProjection {
    let mut entities = index.entities_by_id.values().copied().collect::<Vec<_>>();
    entities.sort_by(|left, right| compare_entity_priority(left, right));
    let truncated = entities.len() > max_entities;
    entities.truncate(max_entities);
    GraphProjection {
        entities: entities
            .into_iter()
            .map(GraphProjectionEntity::from)
            .collect(),
        relationships: Vec::new(),
        seed_entity_ids: Vec::new(),
        seed_relationship_ids: Vec::new(),
        missing_entity_ids: Vec::new(),
        missing_relationship_ids: Vec::new(),
        unresolved_relationship_ids: Vec::new(),
        unresolved_relationship_count: index.unresolved_relationship_count,
        truncated,
    }
}

fn projection_entities<'a>(
    index: &ResolutionIndex<'a>,
    ids: impl Iterator<Item = &'a str>,
) -> Vec<GraphProjectionEntity> {
    ids.filter_map(|id| index.entities_by_id.get(id).copied())
        .map(GraphProjectionEntity::from)
        .collect()
}

fn projection_relationship(relationship: &ResolvedRelationship<'_>) -> GraphProjectionRelationship {
    GraphProjectionRelationship {
        id: relationship.relationship.id.clone(),
        source_entity_id: relationship.source_entity_id.to_owned(),
        target_entity_id: relationship.target_entity_id.to_owned(),
        source: relationship.relationship.source.clone(),
        target: relationship.relationship.target.clone(),
        weight: relationship.relationship.weight,
        rank: relationship.relationship.rank,
    }
}

fn compare_entity_priority(left: &GraphEntityDetail, right: &GraphEntityDetail) -> Ordering {
    right
        .rank
        .cmp(&left.rank)
        .then_with(|| left.id.cmp(&right.id))
}

fn compare_relationship_priority(
    left: &ResolvedRelationship<'_>,
    right: &ResolvedRelationship<'_>,
) -> Ordering {
    right
        .relationship
        .rank
        .cmp(&left.relationship.rank)
        .then_with(|| {
            compare_optional_weight_desc(left.relationship.weight, right.relationship.weight)
        })
        .then_with(|| left.relationship.id.cmp(&right.relationship.id))
}

fn compare_optional_weight_desc(left: Option<f64>, right: Option<f64>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => right.total_cmp(&left),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

impl From<&GraphEntityDetail> for GraphProjectionEntity {
    fn from(entity: &GraphEntityDetail) -> Self {
        Self {
            id: entity.id.clone(),
            title: entity.title.clone(),
            entity_type: entity.entity_type.clone(),
            rank: entity.rank,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::graph::{GraphDataSourceError, GraphEntityDetail, GraphRelationshipDetail};

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    fn entity(id: &str, title: &str, rank: i64) -> GraphEntityDetail {
        GraphEntityDetail::new(id.to_owned(), title.to_owned()).with_rank(rank)
    }

    fn relationship(
        id: &str,
        source: &str,
        target: &str,
        rank: i64,
        weight: f64,
    ) -> GraphRelationshipDetail {
        GraphRelationshipDetail::new(id.to_owned(), source.to_owned(), target.to_owned())
            .with_rank(rank)
            .with_weight(weight)
    }

    fn snapshot(
        entities: Vec<GraphEntityDetail>,
        relationships: Vec<GraphRelationshipDetail>,
    ) -> Result<GraphDataSnapshot, GraphDataSourceError> {
        GraphDataSnapshot::new(entities, relationships, Vec::new(), Vec::new())
    }

    fn topology_snapshot() -> Result<GraphDataSnapshot, GraphDataSourceError> {
        snapshot(
            vec![
                entity("a", "Alice", 10),
                entity("b", "Bob", 9),
                entity("c", "Carol", 8),
                entity("d", "Detached", 100),
            ],
            vec![
                relationship("r-low", "Alice", "Carol", 1, 1.0),
                relationship("r-high", "Alice", "Bob", 5, 0.5),
                relationship("r-weight", "Bob", "Carol", 5, 0.9),
                relationship("r-missing", "Nobody", "Alice", 100, 1.0),
            ],
        )
    }

    #[test]
    fn test_should_resolve_only_unique_endpoint_titles() -> TestResult {
        let snapshot = snapshot(
            vec![
                entity("a-1", "Alice", 1),
                entity("a-2", "Alice", 2),
                entity("b", "Bob", 3),
            ],
            vec![
                relationship("ambiguous", "Alice", "Bob", 2, 1.0),
                relationship("missing", "Nobody", "Bob", 3, 1.0),
                relationship("unique", "Bob", "Bob", 1, 1.0),
            ],
        )?;
        let projection = overview(&snapshot, 10, 10);

        assert_eq!(projection.unresolved_relationship_count, 2);
        assert_eq!(projection.relationships.len(), 1);
        assert_eq!(projection.relationships[0].id, "unique");
        assert_eq!(projection.relationships[0].source_entity_id, "b");
        assert_eq!(projection.relationships[0].target_entity_id, "b");
        Ok(())
    }

    #[test]
    fn test_should_count_unique_missing_relationship_endpoints() {
        let mut selected = BTreeSet::new();
        assert_eq!(additional_endpoint_count(&selected, "a", "b"), 2);
        assert_eq!(additional_endpoint_count(&selected, "a", "a"), 1);

        selected.insert("a");
        assert_eq!(additional_endpoint_count(&selected, "a", "b"), 1);
        assert_eq!(additional_endpoint_count(&selected, "a", "a"), 0);

        selected.insert("b");
        assert_eq!(additional_endpoint_count(&selected, "a", "b"), 0);
    }

    #[test]
    fn test_should_count_self_loop_endpoint_once_in_overview() -> TestResult {
        let snapshot = snapshot(
            vec![entity("a", "Alice", 1)],
            vec![relationship("loop", "Alice", "Alice", 1, 1.0)],
        )?;

        let projection = overview(&snapshot, 1, 1);

        assert_eq!(projection.entities.len(), 1);
        assert_eq!(projection.entities[0].id, "a");
        assert_eq!(projection.relationships.len(), 1);
        assert_eq!(projection.relationships[0].id, "loop");
        assert!(!projection.truncated);
        Ok(())
    }

    #[test]
    fn test_should_build_deterministic_edge_first_overview() -> TestResult {
        let mut snapshot = topology_snapshot()?;
        snapshot.relationships[0].weight = Some(f64::NAN);
        let expected = overview(&snapshot, 3, 3);
        let expected_entity_ids = expected
            .entities
            .iter()
            .map(|entity| entity.id.as_str())
            .collect::<Vec<_>>();
        let expected_relationship_ids = expected
            .relationships
            .iter()
            .map(|relationship| relationship.id.as_str())
            .collect::<Vec<_>>();
        for _ in 0..100 {
            let projection = overview(&snapshot, 3, 3);
            assert_eq!(
                projection
                    .entities
                    .iter()
                    .map(|entity| entity.id.as_str())
                    .collect::<Vec<_>>(),
                expected_entity_ids
            );
            assert_eq!(
                projection
                    .relationships
                    .iter()
                    .map(|relationship| relationship.id.as_str())
                    .collect::<Vec<_>>(),
                expected_relationship_ids
            );
        }
        assert_eq!(
            expected
                .relationships
                .iter()
                .map(|relationship| relationship.id.as_str())
                .collect::<Vec<_>>(),
            ["r-weight", "r-high", "r-low"]
        );
        assert!(expected.entities.iter().all(|entity| entity.id != "d"));
        Ok(())
    }

    #[test]
    fn test_should_bound_overview_and_keep_every_edge_endpoint() -> TestResult {
        let snapshot = topology_snapshot()?;
        let entity_bounded = overview(&snapshot, 2, 10);
        assert_eq!(entity_bounded.entities.len(), 2);
        assert_eq!(entity_bounded.relationships.len(), 1);
        assert!(entity_bounded.truncated);

        let relationship_bounded = overview(&snapshot, 10, 1);
        assert_eq!(relationship_bounded.relationships.len(), 1);
        assert!(relationship_bounded.truncated);
        let entity_ids = relationship_bounded
            .entities
            .iter()
            .map(|entity| entity.id.as_str())
            .collect::<BTreeSet<_>>();
        assert!(
            relationship_bounded
                .relationships
                .iter()
                .all(|relationship| {
                    entity_ids.contains(relationship.source_entity_id.as_str())
                        && entity_ids.contains(relationship.target_entity_id.as_str())
                })
        );
        Ok(())
    }

    #[test]
    fn test_should_not_fill_overview_with_unrelated_isolated_entities() -> TestResult {
        let projection = overview(&topology_snapshot()?, 80, 160);
        assert_eq!(projection.entities.len(), 3);
        assert!(projection.entities.iter().all(|entity| entity.id != "d"));
        assert!(!projection.truncated);
        Ok(())
    }

    #[test]
    fn test_should_fallback_to_ranked_entities_when_no_edges_resolve() -> TestResult {
        let snapshot = snapshot(
            vec![entity("a", "Alice", 1), entity("b", "Bob", 9)],
            vec![relationship("missing", "Nobody", "Bob", 1, 1.0)],
        )?;
        let projection = overview(&snapshot, 1, 10);
        assert_eq!(projection.entities[0].id, "b");
        assert!(projection.relationships.is_empty());
        assert!(projection.truncated);
        assert_eq!(projection.unresolved_relationship_count, 1);
        Ok(())
    }

    #[test]
    fn test_should_return_entity_seed_at_depth_zero_and_neighbors_at_depth_one() -> TestResult {
        let snapshot = topology_snapshot()?;
        let seeds = vec!["a".to_owned()];
        let depth_zero = subgraph(&snapshot, &seeds, &[], 0, 80, 160)?;
        assert_eq!(depth_zero.seed_entity_ids, ["a"]);
        assert_eq!(depth_zero.entities.len(), 1);
        assert!(depth_zero.relationships.is_empty());

        let depth_one = subgraph(&snapshot, &seeds, &[], 1, 80, 160)?;
        assert_eq!(depth_one.seed_entity_ids, ["a"]);
        assert_eq!(depth_one.entities[0].id, "a");
        assert_eq!(depth_one.entities.len(), 3);
        assert_eq!(
            depth_one
                .relationships
                .iter()
                .map(|relationship| relationship.id.as_str())
                .collect::<Vec<_>>(),
            ["r-high", "r-low"]
        );
        Ok(())
    }

    #[test]
    fn test_should_include_self_loop_with_tight_subgraph_budget() -> TestResult {
        let snapshot = snapshot(
            vec![entity("a", "Alice", 1)],
            vec![relationship("loop", "Alice", "Alice", 1, 1.0)],
        )?;

        let projection = subgraph(&snapshot, &["a".to_owned()], &[], 1, 1, 1)?;

        assert_eq!(projection.entities.len(), 1);
        assert_eq!(projection.entities[0].id, "a");
        assert_eq!(projection.relationships.len(), 1);
        assert_eq!(projection.relationships[0].id, "loop");
        assert!(!projection.truncated);
        Ok(())
    }

    #[test]
    fn test_should_resolve_relationship_seed_and_include_endpoints() -> TestResult {
        let snapshot = topology_snapshot()?;
        let relationship_seeds = vec!["r-high".to_owned()];
        let projection = subgraph(&snapshot, &[], &relationship_seeds, 0, 80, 160)?;
        assert_eq!(projection.seed_entity_ids, ["a", "b"]);
        assert_eq!(projection.seed_relationship_ids, ["r-high"]);
        assert_eq!(projection.entities.len(), 2);
        assert_eq!(projection.relationships.len(), 1);
        Ok(())
    }

    #[test]
    fn test_should_report_sorted_deduplicated_missing_and_unresolved_seeds() -> TestResult {
        let snapshot = snapshot(
            vec![
                entity("a-1", "Alice", 1),
                entity("a-2", "Alice", 2),
                entity("b", "Bob", 3),
            ],
            vec![relationship("ambiguous", "Alice", "Bob", 1, 1.0)],
        )?;
        let projection = subgraph(
            &snapshot,
            &["z".to_owned(), "missing".to_owned(), "z".to_owned()],
            &[
                "unknown".to_owned(),
                "ambiguous".to_owned(),
                "unknown".to_owned(),
            ],
            0,
            80,
            160,
        )?;
        assert_eq!(projection.missing_entity_ids, ["missing", "z"]);
        assert_eq!(projection.missing_relationship_ids, ["unknown"]);
        assert_eq!(projection.unresolved_relationship_ids, ["ambiguous"]);
        assert_eq!(projection.unresolved_relationship_count, 1);
        Ok(())
    }

    #[test]
    fn test_should_never_silently_truncate_valid_seeds() -> TestResult {
        let snapshot = topology_snapshot()?;
        assert_eq!(
            subgraph(&snapshot, &["a".to_owned(), "b".to_owned()], &[], 0, 1, 160,),
            Err(GraphProjectionError::SeedLimitsTooSmall)
        );
        assert_eq!(
            subgraph(
                &snapshot,
                &[],
                &["r-high".to_owned(), "r-low".to_owned()],
                0,
                80,
                1,
            ),
            Err(GraphProjectionError::SeedLimitsTooSmall)
        );
        Ok(())
    }

    #[test]
    fn test_should_bound_neighbor_expansion_and_mark_truncated() -> TestResult {
        let snapshot = topology_snapshot()?;
        let projection = subgraph(&snapshot, &["a".to_owned()], &[], 1, 2, 1)?;
        assert_eq!(projection.entities.len(), 2);
        assert_eq!(projection.relationships.len(), 1);
        assert_eq!(projection.relationships[0].id, "r-high");
        assert!(projection.truncated);
        Ok(())
    }

    #[test]
    fn test_should_include_edges_between_already_included_seed_entities() -> TestResult {
        let snapshot = topology_snapshot()?;
        let projection = subgraph(
            &snapshot,
            &["a".to_owned(), "b".to_owned(), "c".to_owned()],
            &[],
            1,
            3,
            3,
        )?;
        assert_eq!(projection.relationships.len(), 3);
        assert!(!projection.truncated);
        Ok(())
    }
}
