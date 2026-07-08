//! Community creation workflow.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use chrono::Utc;
use polars_core::prelude::*;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    GraphRagConfig, PipelineRunContext, Result, Workflow, WorkflowFunctionOutput,
    dataframe::{invalid_data, list_at, list_column, row_to_static, string_value},
    operations::communities::{
        ClusterRelationship, CommunityCluster, cluster_graph as cluster_relationship_graph,
    },
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
    size: usize,
}

fn create_communities(
    entities: &[EntityRow],
    relationships: &[RelationshipRow],
    max_cluster_size: u32,
    use_lcc: bool,
    seed: u64,
) -> Result<Vec<CommunityRow>> {
    let cluster_input = relationships
        .iter()
        .map(|relationship| ClusterRelationship {
            source: relationship.source.clone(),
            target: relationship.target.clone(),
            weight: relationship.weight,
        })
        .collect::<Vec<_>>();
    let clusters = cluster_relationship_graph(&cluster_input, max_cluster_size, use_lcc, seed)?;
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
        let size = entity_ids.len();
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
            size,
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
            text_unit_ids: list_at(&row, text_unit_ids_index, CREATE_COMMUNITIES_WORKFLOW)?,
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
        "size" => rows.iter().map(|row| row.size as u64).collect::<Vec<_>>(),
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

#[cfg(test)]
mod tests {
    use polars_core::prelude::*;

    use super::*;

    #[test]
    fn test_should_write_community_schema_in_graphrag_order() {
        let rows = vec![CommunityRow {
            id: "community-1".to_owned(),
            human_readable_id: 0,
            community: 0,
            level: 0,
            parent: -1,
            children: vec![1, 2],
            title: "Community 0".to_owned(),
            entity_ids: vec!["entity-1".to_owned(), "entity-2".to_owned()],
            relationship_ids: vec!["rel-1".to_owned()],
            text_unit_ids: vec!["tu-1".to_owned()],
            period: "2026-07-08".to_owned(),
            size: 2,
        }];

        let dataframe = communities_dataframe(&rows).expect("dataframe should build");

        assert_eq!(
            column_names(&dataframe),
            [
                "id",
                "human_readable_id",
                "community",
                "level",
                "parent",
                "children",
                "title",
                "entity_ids",
                "relationship_ids",
                "text_unit_ids",
                "period",
                "size",
            ]
        );
        assert_eq!(
            dataframe.column("children").expect("children").dtype(),
            &DataType::List(Box::new(DataType::Int64))
        );
        assert_eq!(
            dataframe.column("entity_ids").expect("entity_ids").dtype(),
            &DataType::List(Box::new(DataType::String))
        );
        assert_eq!(
            dataframe
                .column("relationship_ids")
                .expect("relationship_ids")
                .dtype(),
            &DataType::List(Box::new(DataType::String))
        );
        assert_eq!(
            dataframe
                .column("text_unit_ids")
                .expect("text_unit_ids")
                .dtype(),
            &DataType::List(Box::new(DataType::String))
        );
        assert_eq!(
            dataframe.column("size").expect("size").dtype(),
            &DataType::UInt64
        );
    }

    #[test]
    fn test_should_size_community_by_mapped_entity_ids() {
        let entities = vec![EntityRow {
            id: "entity-alice".to_owned(),
            title: "ALICE".to_owned(),
        }];
        let relationships = vec![RelationshipRow {
            id: "rel-1".to_owned(),
            source: "ALICE".to_owned(),
            target: "BOB".to_owned(),
            weight: 1.0,
            text_unit_ids: vec!["tu-1".to_owned()],
        }];

        let rows = create_communities(&entities, &relationships, 10, false, 42)
            .expect("communities should build");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].entity_ids, vec!["entity-alice"]);
        assert_eq!(rows[0].size, 1);
    }

    fn column_names(dataframe: &DataFrame) -> Vec<&str> {
        dataframe
            .get_column_names()
            .into_iter()
            .map(|name| name.as_str())
            .collect()
    }
}
