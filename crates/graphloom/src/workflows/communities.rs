//! Community creation workflow.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use async_trait::async_trait;
use chrono::Utc;
use html_escape::decode_html_entities;
use network_partitions::{
    clustering::Clustering,
    leiden::{self, HierarchicalCluster},
    network::prelude::{CompactNetwork, Edge, LabeledNetwork, LabeledNetworkBuilder},
};
use polars_core::prelude::*;
use rand::{SeedableRng, rngs::SmallRng};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    common::{invalid_data, string_value},
    graph::{list_at, row_to_static},
    input_documents::{list_column, usize_to_i64},
};
use crate::{
    GraphLoomError, GraphRagConfig, PipelineRunContext, Result, Workflow, WorkflowFunctionOutput,
};

/// Workflow name.
pub const CREATE_COMMUNITIES_WORKFLOW: &str = "create_communities";

/// Cluster graph entities and relationships into communities.
#[derive(Debug, Clone, Copy, Default)]
pub struct CreateCommunitiesWorkflow;

#[async_trait]
impl Workflow for CreateCommunitiesWorkflow {
    fn name(&self) -> &'static str {
        CREATE_COMMUNITIES_WORKFLOW
    }

    async fn run(
        &self,
        config: &GraphRagConfig,
        context: &mut PipelineRunContext,
    ) -> Result<WorkflowFunctionOutput> {
        let entities = read_entities(
            &context
                .output_table_provider
                .read_dataframe("entities")
                .await?,
        )?;
        let relationships = read_relationships(
            &context
                .output_table_provider
                .read_dataframe("relationships")
                .await?,
        )?;
        let communities = create_communities(
            &entities,
            &relationships,
            config.cluster_graph.max_cluster_size,
            config.cluster_graph.use_lcc,
            config.cluster_graph.seed,
        )?;

        context
            .output_table_provider
            .write_dataframe("communities", communities_dataframe(&communities)?)
            .await?;
        context.stats.community_count = communities.len();

        Ok(WorkflowFunctionOutput {
            result: communities.iter().take(5).map(community_value).collect(),
            stop: false,
            input_rows: entities.len().saturating_add(relationships.len()),
            output_rows: communities.len(),
        })
    }
}

#[derive(Debug, Clone)]
struct EntityRow {
    id: String,
    title: String,
}

#[derive(Debug, Clone)]
struct RelationshipRow {
    id: String,
    source: String,
    target: String,
    weight: f64,
    text_unit_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct CommunityCluster {
    level: i64,
    community: i64,
    parent: i64,
    titles: Vec<String>,
}

#[derive(Debug, Clone)]
struct CommunityRow {
    id: String,
    human_readable_id: i64,
    community: i64,
    level: i64,
    parent: i64,
    children: Vec<i64>,
    title: String,
    entity_ids: Vec<String>,
    relationship_ids: Vec<String>,
    text_unit_ids: Vec<String>,
    period: String,
    size: i64,
}

fn create_communities(
    entities: &[EntityRow],
    relationships: &[RelationshipRow],
    max_cluster_size: u32,
    use_lcc: bool,
    seed: u64,
) -> Result<Vec<CommunityRow>> {
    let clusters = cluster_graph(relationships, max_cluster_size, use_lcc, seed)?;
    let title_to_entity_id = entities
        .iter()
        .map(|entity| (entity.title.as_str(), entity.id.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut level_titles: BTreeMap<(i64, String), i64> = BTreeMap::new();
    for cluster in &clusters {
        for title in &cluster.titles {
            level_titles.insert((cluster.level, title.clone()), cluster.community);
        }
    }

    let mut rows = Vec::new();
    let period = Utc::now().date_naive().to_string();
    for cluster in clusters {
        let entity_ids = cluster
            .titles
            .iter()
            .filter_map(|title| title_to_entity_id.get(title.as_str()).copied())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if entity_ids.is_empty() {
            continue;
        }
        let (relationship_ids, text_unit_ids) =
            intra_community_relationships(&cluster, relationships, &level_titles);
        if relationship_ids.is_empty() {
            continue;
        }
        rows.push(CommunityRow {
            id: Uuid::new_v4().to_string(),
            human_readable_id: cluster.community,
            community: cluster.community,
            level: cluster.level,
            parent: cluster.parent,
            children: Vec::new(),
            title: format!("Community {}", cluster.community),
            entity_ids,
            relationship_ids,
            text_unit_ids,
            period: period.clone(),
            size: usize_to_i64(cluster.titles.len(), CREATE_COMMUNITIES_WORKFLOW)?,
        });
    }

    let mut children_by_parent: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
    for row in &rows {
        children_by_parent
            .entry(row.parent)
            .or_default()
            .insert(row.community);
    }
    for row in &mut rows {
        if let Some(children) = children_by_parent.get(&row.community) {
            row.children = children.iter().copied().collect();
        } else {
            row.children = Vec::new();
        }
    }
    Ok(rows)
}

fn cluster_graph(
    relationships: &[RelationshipRow],
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
        workflow: CREATE_COMMUNITIES_WORKFLOW,
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
        usize_to_i64(cluster.cluster, CREATE_COMMUNITIES_WORKFLOW)?,
        cluster.parent_cluster.map_or(Ok(-1), |parent| {
            usize_to_i64(parent, CREATE_COMMUNITIES_WORKFLOW)
        })?,
    ))
}

fn prepare_cluster_edges(relationships: &[RelationshipRow], use_lcc: bool) -> Vec<Edge> {
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

fn intra_community_relationships(
    cluster: &CommunityCluster,
    relationships: &[RelationshipRow],
    level_titles: &BTreeMap<(i64, String), i64>,
) -> (Vec<String>, Vec<String>) {
    let mut relationship_ids = BTreeSet::new();
    let mut text_unit_ids = BTreeSet::new();
    for relationship in relationships {
        let source = level_titles.get(&(cluster.level, relationship.source.clone()));
        let target = level_titles.get(&(cluster.level, relationship.target.clone()));
        if source == Some(&cluster.community) && target == Some(&cluster.community) {
            relationship_ids.insert(relationship.id.clone());
            text_unit_ids.extend(relationship.text_unit_ids.iter().cloned());
        }
    }
    (
        relationship_ids.into_iter().collect(),
        text_unit_ids.into_iter().collect(),
    )
}

fn read_entities(dataframe: &DataFrame) -> Result<Vec<EntityRow>> {
    let ids = dataframe.column("id")?.str()?;
    let titles = dataframe.column("title")?.str()?;
    let mut rows = Vec::with_capacity(dataframe.height());
    for index in 0..dataframe.height() {
        rows.push(EntityRow {
            id: string_value(ids.get(index), "id", CREATE_COMMUNITIES_WORKFLOW)?,
            title: string_value(titles.get(index), "title", CREATE_COMMUNITIES_WORKFLOW)?,
        });
    }
    Ok(rows)
}

fn read_relationships(dataframe: &DataFrame) -> Result<Vec<RelationshipRow>> {
    let ids = dataframe.column("id")?.str()?;
    let sources = dataframe.column("source")?.str()?;
    let targets = dataframe.column("target")?.str()?;
    let weights = dataframe.column("weight")?.f64()?;
    let text_unit_ids_index = dataframe
        .get_column_names()
        .iter()
        .position(|name| name.as_str() == "text_unit_ids")
        .ok_or_else(|| invalid_data(CREATE_COMMUNITIES_WORKFLOW, "missing text_unit_ids"))?;
    let mut rows = Vec::with_capacity(dataframe.height());
    for index in 0..dataframe.height() {
        let row = row_to_static(dataframe.get_row(index)?);
        rows.push(RelationshipRow {
            id: string_value(ids.get(index), "id", CREATE_COMMUNITIES_WORKFLOW)?,
            source: string_value(sources.get(index), "source", CREATE_COMMUNITIES_WORKFLOW)?,
            target: string_value(targets.get(index), "target", CREATE_COMMUNITIES_WORKFLOW)?,
            weight: weights
                .get(index)
                .ok_or_else(|| invalid_data(CREATE_COMMUNITIES_WORKFLOW, "missing weight"))?,
            text_unit_ids: list_at(&row, text_unit_ids_index)?,
        });
    }
    Ok(rows)
}

fn communities_dataframe(rows: &[CommunityRow]) -> Result<DataFrame> {
    let mut dataframe = df!(
        "id" => rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        "human_readable_id" => rows.iter().map(|row| row.human_readable_id).collect::<Vec<_>>(),
        "community" => rows.iter().map(|row| row.community).collect::<Vec<_>>(),
        "level" => rows.iter().map(|row| row.level).collect::<Vec<_>>(),
        "parent" => rows.iter().map(|row| row.parent).collect::<Vec<_>>(),
        "title" => rows.iter().map(|row| row.title.as_str()).collect::<Vec<_>>(),
        "period" => rows.iter().map(|row| row.period.as_str()).collect::<Vec<_>>(),
        "size" => rows.iter().map(|row| row.size).collect::<Vec<_>>(),
    )?;
    dataframe.insert_column(
        5,
        i64_list_column(
            "children",
            &rows
                .iter()
                .map(|row| row.children.clone())
                .collect::<Vec<_>>(),
        )?,
    )?;
    dataframe.insert_column(
        7,
        list_column(
            "entity_ids",
            &rows
                .iter()
                .map(|row| row.entity_ids.clone())
                .collect::<Vec<_>>(),
        )?,
    )?;
    dataframe.insert_column(
        8,
        list_column(
            "relationship_ids",
            &rows
                .iter()
                .map(|row| row.relationship_ids.clone())
                .collect::<Vec<_>>(),
        )?,
    )?;
    dataframe.insert_column(
        9,
        list_column(
            "text_unit_ids",
            &rows
                .iter()
                .map(|row| row.text_unit_ids.clone())
                .collect::<Vec<_>>(),
        )?,
    )?;
    Ok(dataframe)
}

fn i64_list_column(name: &str, values: &[Vec<i64>]) -> Result<Column> {
    let series = values
        .iter()
        .map(|values| Series::new(name.into(), values.as_slice()))
        .collect::<Vec<_>>();
    Ok(Series::new(name.into(), series).into())
}

fn community_value(row: &CommunityRow) -> Value {
    json!({
        "id": row.id,
        "human_readable_id": row.human_readable_id,
        "community": row.community,
        "level": row.level,
        "parent": row.parent,
        "children": row.children,
        "title": row.title,
        "entity_ids": row.entity_ids,
        "relationship_ids": row.relationship_ids,
        "text_unit_ids": row.text_unit_ids,
        "period": row.period,
        "size": row.size,
    })
}
