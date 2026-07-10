//! Community clustering operations.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use html_escape::decode_html_entities;
use network_partitions::{
    clustering::Clustering,
    leiden::{self, HierarchicalCluster},
    network::prelude::{CompactNetwork, Edge, LabeledNetwork, LabeledNetworkBuilder},
};
use rand::{SeedableRng, rngs::SmallRng};

use crate::{GraphLoomError, Result};

const CREATE_COMMUNITIES_CONTEXT: &str = "create_communities";

#[derive(Debug, Clone)]
pub(crate) struct ClusterRelationship {
    pub(crate) source: String,
    pub(crate) target: String,
    pub(crate) weight: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CommunityCluster {
    pub(crate) level: i64,
    pub(crate) community: i64,
    pub(crate) parent: i64,
    pub(crate) titles: Vec<String>,
}

pub(crate) fn cluster_graph(
    relationships: &[ClusterRelationship],
    max_cluster_size: u32,
    use_lcc: bool,
    seed: u64,
) -> Result<Vec<CommunityCluster>> {
    let edges = prepare_cluster_edges(relationships, use_lcc);
    let mut builder = LabeledNetworkBuilder::new();
    let labeled_network: LabeledNetwork<String> = builder.build(edges.into_iter(), true);
    let compact_network: &CompactNetwork = labeled_network.compact();
    let mut rng = SmallRng::seed_from_u64(seed);
    let internal = leiden::hierarchical_leiden(
        compact_network,
        None::<Clustering>,
        Some(1),
        Some(1.0),
        Some(0.001),
        &mut rng,
        true,
        max_cluster_size,
        None,
    )
    .map_err(|source| GraphLoomError::InvalidData {
        workflow: CREATE_COMMUNITIES_CONTEXT,
        message: format!("{source:?}"),
    })?;
    let mut node_map: BTreeMap<(i64, i64, i64), Vec<String>> = BTreeMap::new();
    for cluster in internal {
        let key = cluster_key(&cluster)?;
        node_map
            .entry(key)
            .or_default()
            .push(labeled_network.label_for(cluster.node).to_owned());
    }

    Ok(node_map
        .into_iter()
        .map(|((level, community, parent), mut titles)| {
            titles.sort();
            CommunityCluster {
                level,
                community,
                parent,
                titles,
            }
        })
        .collect())
}

fn cluster_key(cluster: &HierarchicalCluster) -> Result<(i64, i64, i64)> {
    Ok((
        i64::from(cluster.level),
        cluster_index_to_i64(cluster.cluster, "community")?,
        cluster
            .parent_cluster
            .map_or(Ok(-1), |parent| cluster_index_to_i64(parent, "parent"))?,
    ))
}

fn cluster_index_to_i64(value: usize, column: &'static str) -> Result<i64> {
    i64::try_from(value).map_err(|source| GraphLoomError::InvalidData {
        workflow: CREATE_COMMUNITIES_CONTEXT,
        message: format!("{column} cluster index is too large for i64: {source}"),
    })
}

pub(crate) fn prepare_cluster_edges(
    relationships: &[ClusterRelationship],
    use_lcc: bool,
) -> Vec<Edge> {
    let mut pair_indexes = BTreeMap::new();
    let mut deduped = Vec::<Option<ClusterEdge>>::new();
    for relationship in relationships {
        let (source, target) = sorted_pair(&relationship.source, &relationship.target);
        let next_index = deduped.len();
        if let Some(previous_index) =
            pair_indexes.insert((source.clone(), target.clone()), next_index)
            && let Some(previous) = deduped.get_mut(previous_index)
        {
            *previous = None;
        }
        deduped.push(Some(ClusterEdge {
            source,
            target,
            weight: relationship.weight,
        }));
    }
    let mut edges = deduped.into_iter().flatten().collect::<Vec<_>>();
    if use_lcc {
        edges = stable_lcc(edges);
    } else {
        sort_edges(&mut edges);
    }
    edges
        .into_iter()
        .map(|edge| (edge.source, edge.target, edge.weight))
        .collect()
}

#[derive(Debug, Clone)]
struct ClusterEdge {
    source: String,
    target: String,
    weight: f64,
}

fn stable_lcc(edges: Vec<ClusterEdge>) -> Vec<ClusterEdge> {
    if edges.is_empty() {
        return Vec::new();
    }
    let mut normalized = edges
        .into_iter()
        .map(|edge| {
            let source = normalize_node_name(&edge.source);
            let target = normalize_node_name(&edge.target);
            ClusterEdge {
                source,
                target,
                weight: edge.weight,
            }
        })
        .collect::<Vec<_>>();
    let lcc_nodes = largest_connected_component(&normalized);
    normalized.retain(|edge| lcc_nodes.contains(&edge.source) && lcc_nodes.contains(&edge.target));

    let mut by_pair = BTreeMap::new();
    for edge in normalized {
        let (source, target) = sorted_pair(&edge.source, &edge.target);
        by_pair.entry((source, target)).or_insert(edge.weight);
    }
    by_pair
        .into_iter()
        .map(|((source, target), weight)| ClusterEdge {
            source,
            target,
            weight,
        })
        .collect()
}

fn sort_edges(edges: &mut [ClusterEdge]) {
    edges.sort_by(|left, right| {
        left.source
            .cmp(&right.source)
            .then_with(|| left.target.cmp(&right.target))
    });
}

fn largest_connected_component(edges: &[ClusterEdge]) -> BTreeSet<String> {
    let mut adjacency: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for edge in edges {
        adjacency
            .entry(edge.source.clone())
            .or_default()
            .insert(edge.target.clone());
        adjacency
            .entry(edge.target.clone())
            .or_default()
            .insert(edge.source.clone());
    }
    let mut visited = BTreeSet::new();
    let mut largest = BTreeSet::new();
    for node in adjacency.keys() {
        if visited.contains(node) {
            continue;
        }
        let mut component = BTreeSet::new();
        let mut queue = VecDeque::from([node.clone()]);
        visited.insert(node.clone());
        while let Some(current) = queue.pop_front() {
            component.insert(current.clone());
            if let Some(neighbors) = adjacency.get(&current) {
                for neighbor in neighbors {
                    if visited.insert(neighbor.clone()) {
                        queue.push_back(neighbor.clone());
                    }
                }
            }
        }
        if component.len() > largest.len() {
            largest = component;
        }
    }
    largest
}

fn normalize_node_name(name: &str) -> String {
    decode_html_entities(name).trim().to_uppercase()
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
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn test_should_keep_last_reversed_duplicate_edge() {
        let relationships = vec![
            relationship("BOB", "ALICE", 1.0),
            relationship("ALICE", "BOB", 2.0),
        ];

        let edges = prepare_cluster_edges(&relationships, false);

        assert_eq!(edges, vec![("ALICE".to_owned(), "BOB".to_owned(), 2.0)]);
    }

    #[test]
    fn test_should_normalize_and_select_stable_lcc_for_equal_components() {
        let relationships = vec![
            relationship("bob", "Alice &amp; Co", 1.0),
            relationship("  carol ", "dave", 3.0),
            relationship("eve", "frank", 4.0),
        ];

        let edges = prepare_cluster_edges(&relationships, true);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].0, "ALICE & CO");
        assert_eq!(edges[0].1, "BOB");
        assert!((edges[0].2 - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_should_use_stable_lexical_lcc_tiebreaker_independent_of_input_order() {
        let first = vec![relationship("C", "D", 1.0), relationship("A", "B", 1.0)];
        let second = vec![relationship("A", "B", 1.0), relationship("C", "D", 1.0)];

        let first_edges = prepare_cluster_edges(&first, true);
        let second_edges = prepare_cluster_edges(&second, true);

        // Microsoft GraphRAG's current union-find keeps the first equal-sized
        // component by input order. GraphLoom intentionally keeps the
        // lexically first component so shuffled equal-sized inputs are stable.
        assert_eq!(first_edges, vec![("A".to_owned(), "B".to_owned(), 1.0)]);
        assert_eq!(second_edges, first_edges);
    }

    #[test]
    fn test_should_normalize_and_keep_largest_connected_component() {
        let relationships = vec![
            relationship("bob", "Alice &amp; Co", 1.0),
            relationship("  carol ", "dave", 3.0),
            relationship("dave", "erin", 4.0),
        ];

        let edges = prepare_cluster_edges(&relationships, true);

        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].0, "CAROL");
        assert_eq!(edges[0].1, "DAVE");
        assert_eq!(edges[1].0, "DAVE");
        assert_eq!(edges[1].1, "ERIN");
    }

    #[test]
    fn test_should_keep_fixed_seed_cluster_members_stable() {
        let relationships = vec![
            relationship("ALICE", "BOB", 1.0),
            relationship("BOB", "CAROL", 1.0),
            relationship("DAVE", "ERIN", 1.0),
        ];

        let first = cluster_graph(&relationships, 10, false, 42).expect("cluster should run");
        let second = cluster_graph(&relationships, 10, false, 42).expect("cluster should run");

        assert_eq!(cluster_sets(&first), cluster_sets(&second));
    }

    #[test]
    fn test_should_match_fixed_community_fixture_by_normalized_hierarchy() {
        // `network_partitions` and Microsoft GraphRAG's `graspologic_native`
        // may assign different raw community ids. This fixture locks the
        // normalized hierarchy shape that GraphLoom emits for a fixed graph.
        let relationships = vec![
            relationship("A", "B", 1.0),
            relationship("B", "C", 1.0),
            relationship("C", "D", 1.0),
            relationship("D", "A", 1.0),
            relationship("A", "B", 2.0),
            relationship("E", "F", 1.0),
            relationship("F", "G", 1.0),
            relationship("G", "H", 1.0),
            relationship("H", "E", 1.0),
            relationship("D", "E", 0.2),
        ];

        let clusters = cluster_graph(&relationships, 3, false, 0xDEAD_BEEF)
            .expect("fixture cluster should run");
        let hierarchy = normalized_hierarchy(&clusters);

        assert_eq!(
            hierarchy,
            vec![
                NormalizedCluster {
                    level: 0,
                    nodes: set(["A", "B", "C", "D"]),
                    parent_nodes: None,
                },
                NormalizedCluster {
                    level: 0,
                    nodes: set(["E", "F", "G", "H"]),
                    parent_nodes: None,
                },
                NormalizedCluster {
                    level: 1,
                    nodes: set(["A", "B"]),
                    parent_nodes: Some(set(["A", "B", "C", "D"])),
                },
                NormalizedCluster {
                    level: 1,
                    nodes: set(["C", "D"]),
                    parent_nodes: Some(set(["A", "B", "C", "D"])),
                },
                NormalizedCluster {
                    level: 1,
                    nodes: set(["E", "F"]),
                    parent_nodes: Some(set(["E", "F", "G", "H"])),
                },
                NormalizedCluster {
                    level: 1,
                    nodes: set(["G", "H"]),
                    parent_nodes: Some(set(["E", "F", "G", "H"])),
                },
            ]
        );
    }

    fn relationship(source: &str, target: &str, weight: f64) -> ClusterRelationship {
        ClusterRelationship {
            source: source.to_owned(),
            target: target.to_owned(),
            weight,
        }
    }

    fn cluster_sets(clusters: &[CommunityCluster]) -> Vec<(i64, i64, i64, BTreeSet<String>)> {
        clusters
            .iter()
            .map(|cluster| {
                (
                    cluster.level,
                    cluster.community,
                    cluster.parent,
                    cluster.titles.iter().cloned().collect(),
                )
            })
            .collect()
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct NormalizedCluster {
        level: i64,
        nodes: BTreeSet<String>,
        parent_nodes: Option<BTreeSet<String>>,
    }

    fn normalized_hierarchy(clusters: &[CommunityCluster]) -> Vec<NormalizedCluster> {
        let by_community = clusters
            .iter()
            .map(|cluster| {
                (
                    cluster.community,
                    cluster.titles.iter().cloned().collect::<BTreeSet<_>>(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut normalized = clusters
            .iter()
            .map(|cluster| NormalizedCluster {
                level: cluster.level,
                nodes: cluster.titles.iter().cloned().collect(),
                parent_nodes: if cluster.parent < 0 {
                    None
                } else {
                    by_community.get(&cluster.parent).cloned()
                },
            })
            .collect::<Vec<_>>();
        normalized.sort_by(|left, right| {
            left.level
                .cmp(&right.level)
                .then_with(|| left.nodes.cmp(&right.nodes))
                .then_with(|| left.parent_nodes.cmp(&right.parent_nodes))
        });
        normalized
    }

    fn set<const N: usize>(nodes: [&str; N]) -> BTreeSet<String> {
        nodes.into_iter().map(str::to_owned).collect()
    }
}
