//! Read-only Graph Explorer HTTP handlers.

use std::{cmp::Ordering, collections::BTreeSet, sync::Arc};

use axum::{
    Json,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use super::StudioApiState;
use crate::graph::{
    GraphCommunity, GraphDataSnapshot, GraphEntity, GraphProjectionError, GraphRelationship,
    GraphSummary, overview, subgraph as project_subgraph,
};

const DEFAULT_PAGE_LIMIT: usize = 50;
const MAX_PAGE_LIMIT: usize = 200;
const MAX_ID_BYTES: usize = 256;
const MAX_FILTER_BYTES: usize = 1024;
const DEFAULT_PROJECTION_ENTITY_LIMIT: usize = 80;
const DEFAULT_PROJECTION_RELATIONSHIP_LIMIT: usize = 160;
const MAX_PROJECTION_ENTITY_LIMIT: usize = 200;
const MAX_PROJECTION_RELATIONSHIP_LIMIT: usize = 400;
const MAX_ENTITY_SEEDS: usize = 200;
const MAX_RELATIONSHIP_SEEDS: usize = 400;

const INVALID_GRAPH_REQUEST_BODY: &str = "invalid graph request";
const GRAPH_ITEM_NOT_FOUND_BODY: &str = "graph item not found";
const GRAPH_UNAVAILABLE_BODY: &str = "graph data is unavailable";
const SUBGRAPH_LIMITS_TOO_SMALL_BODY: &str =
    "graph subgraph limits are too small for the requested seeds";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GraphOverviewQuery {
    max_entities: Option<usize>,
    max_relationships: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GraphSubgraphRequest {
    #[serde(default)]
    entity_ids: Vec<String>,
    #[serde(default)]
    relationship_ids: Vec<String>,
    #[serde(default = "default_subgraph_depth")]
    depth: u8,
    #[serde(default = "default_projection_entity_limit")]
    max_entities: usize,
    #[serde(default = "default_projection_relationship_limit")]
    max_relationships: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EntityListQuery {
    #[serde(rename = "type")]
    entity_type: Option<String>,
    community: Option<String>,
    limit: Option<usize>,
    after: Option<String>,
    #[serde(default)]
    sort: EntitySort,
    #[serde(default)]
    order: SortOrder,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RelationshipListQuery {
    source: Option<String>,
    target: Option<String>,
    limit: Option<usize>,
    after: Option<String>,
    #[serde(default)]
    sort: RelationshipSort,
    #[serde(default)]
    order: SortOrder,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EntitySort {
    #[default]
    Id,
    Degree,
    Rank,
    Title,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RelationshipSort {
    #[default]
    Id,
    Weight,
    Rank,
    Source,
    Target,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SortOrder {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CommunityListQuery {
    level: Option<i64>,
    parent: Option<i64>,
    limit: Option<usize>,
    after: Option<String>,
}

#[derive(Debug, Serialize)]
struct GraphListResponse<T> {
    items: Vec<T>,
    next_cursor: Option<String>,
}

pub(super) async fn get_summary(State(state): State<Arc<StudioApiState>>) -> Response {
    let Ok(snapshot) = load_snapshot(&state).await else {
        return fixed_error(StatusCode::SERVICE_UNAVAILABLE, GRAPH_UNAVAILABLE_BODY);
    };
    match summarize(&snapshot) {
        Ok(summary) => Json(summary).into_response(),
        Err(()) => fixed_error(StatusCode::SERVICE_UNAVAILABLE, GRAPH_UNAVAILABLE_BODY),
    }
}

pub(super) async fn get_overview(
    State(state): State<Arc<StudioApiState>>,
    query: Result<Query<GraphOverviewQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_GRAPH_REQUEST_BODY);
    };
    let max_entities = query
        .max_entities
        .unwrap_or(DEFAULT_PROJECTION_ENTITY_LIMIT);
    let max_relationships = query
        .max_relationships
        .unwrap_or(DEFAULT_PROJECTION_RELATIONSHIP_LIMIT);
    if !valid_projection_limits(max_entities, max_relationships) {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_GRAPH_REQUEST_BODY);
    }
    let Ok(snapshot) = load_snapshot(&state).await else {
        return fixed_error(StatusCode::SERVICE_UNAVAILABLE, GRAPH_UNAVAILABLE_BODY);
    };
    Json(overview(&snapshot, max_entities, max_relationships)).into_response()
}

pub(super) async fn get_subgraph(
    State(state): State<Arc<StudioApiState>>,
    request: Result<Json<GraphSubgraphRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = request else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_GRAPH_REQUEST_BODY);
    };
    if !valid_subgraph_request(&request) {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_GRAPH_REQUEST_BODY);
    }
    let Ok(snapshot) = load_snapshot(&state).await else {
        return fixed_error(StatusCode::SERVICE_UNAVAILABLE, GRAPH_UNAVAILABLE_BODY);
    };
    match project_subgraph(
        &snapshot,
        &request.entity_ids,
        &request.relationship_ids,
        request.depth,
        request.max_entities,
        request.max_relationships,
    ) {
        Ok(projection) => Json(projection).into_response(),
        Err(GraphProjectionError::SeedLimitsTooSmall) => {
            fixed_error(StatusCode::BAD_REQUEST, SUBGRAPH_LIMITS_TOO_SMALL_BODY)
        }
    }
}

pub(super) async fn list_entities(
    State(state): State<Arc<StudioApiState>>,
    query: Result<Query<EntityListQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_GRAPH_REQUEST_BODY);
    };
    let Ok(limit) = validated_page(
        query.limit,
        query.after.as_deref(),
        [query.entity_type.as_deref(), query.community.as_deref()],
    ) else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_GRAPH_REQUEST_BODY);
    };
    let Ok(snapshot) = load_snapshot(&state).await else {
        return fixed_error(StatusCode::SERVICE_UNAVAILABLE, GRAPH_UNAVAILABLE_BODY);
    };
    let mut items = snapshot
        .entities
        .iter()
        .filter(|entity| {
            query
                .entity_type
                .as_ref()
                .is_none_or(|value| entity.entity_type.as_ref() == Some(value))
                && query
                    .community
                    .as_ref()
                    .is_none_or(|value| entity.community_ids.contains(value))
        })
        .map(GraphEntity::from)
        .collect::<Vec<_>>();
    sort_entities(&mut items, query.sort, query.order);
    Json(paginate(items, query.after.as_deref(), limit, |item| {
        item.id.as_str()
    }))
    .into_response()
}

pub(super) async fn get_entity(
    State(state): State<Arc<StudioApiState>>,
    path: Result<Path<String>, PathRejection>,
) -> Response {
    let Some(id) = valid_path_id(path) else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_GRAPH_REQUEST_BODY);
    };
    let Ok(snapshot) = load_snapshot(&state).await else {
        return fixed_error(StatusCode::SERVICE_UNAVAILABLE, GRAPH_UNAVAILABLE_BODY);
    };
    snapshot
        .entities
        .iter()
        .find(|entity| entity.id == id)
        .map_or_else(
            || fixed_error(StatusCode::NOT_FOUND, GRAPH_ITEM_NOT_FOUND_BODY),
            |entity| Json(entity.clone()).into_response(),
        )
}

pub(super) async fn list_relationships(
    State(state): State<Arc<StudioApiState>>,
    query: Result<Query<RelationshipListQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_GRAPH_REQUEST_BODY);
    };
    let Ok(limit) = validated_page(
        query.limit,
        query.after.as_deref(),
        [query.source.as_deref(), query.target.as_deref()],
    ) else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_GRAPH_REQUEST_BODY);
    };
    let Ok(snapshot) = load_snapshot(&state).await else {
        return fixed_error(StatusCode::SERVICE_UNAVAILABLE, GRAPH_UNAVAILABLE_BODY);
    };
    let mut items = snapshot
        .relationships
        .iter()
        .filter(|relationship| {
            query
                .source
                .as_ref()
                .is_none_or(|value| &relationship.source == value)
                && query
                    .target
                    .as_ref()
                    .is_none_or(|value| &relationship.target == value)
        })
        .map(GraphRelationship::from)
        .collect::<Vec<_>>();
    sort_relationships(&mut items, query.sort, query.order);
    Json(paginate(items, query.after.as_deref(), limit, |item| {
        item.id.as_str()
    }))
    .into_response()
}

pub(super) async fn get_relationship(
    State(state): State<Arc<StudioApiState>>,
    path: Result<Path<String>, PathRejection>,
) -> Response {
    let Some(id) = valid_path_id(path) else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_GRAPH_REQUEST_BODY);
    };
    let Ok(snapshot) = load_snapshot(&state).await else {
        return fixed_error(StatusCode::SERVICE_UNAVAILABLE, GRAPH_UNAVAILABLE_BODY);
    };
    snapshot
        .relationships
        .iter()
        .find(|relationship| relationship.id == id)
        .map_or_else(
            || fixed_error(StatusCode::NOT_FOUND, GRAPH_ITEM_NOT_FOUND_BODY),
            |relationship| Json(relationship.clone()).into_response(),
        )
}

pub(super) async fn list_communities(
    State(state): State<Arc<StudioApiState>>,
    query: Result<Query<CommunityListQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_GRAPH_REQUEST_BODY);
    };
    let Ok(limit) = validated_page(query.limit, query.after.as_deref(), []) else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_GRAPH_REQUEST_BODY);
    };
    let Ok(snapshot) = load_snapshot(&state).await else {
        return fixed_error(StatusCode::SERVICE_UNAVAILABLE, GRAPH_UNAVAILABLE_BODY);
    };
    let mut items = snapshot
        .communities
        .iter()
        .filter(|community| {
            query.level.is_none_or(|value| community.level == value)
                && query.parent.is_none_or(|value| community.parent == value)
        })
        .map(|community| {
            GraphCommunity::with_report(
                community,
                snapshot
                    .community_reports
                    .iter()
                    .find(|report| report.community_id == community.short_id),
            )
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| left.id.cmp(&right.id));
    Json(paginate(items, query.after.as_deref(), limit, |item| {
        item.id.as_str()
    }))
    .into_response()
}

pub(super) async fn get_community(
    State(state): State<Arc<StudioApiState>>,
    path: Result<Path<String>, PathRejection>,
) -> Response {
    let Some(id) = valid_path_id(path) else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_GRAPH_REQUEST_BODY);
    };
    let Ok(snapshot) = load_snapshot(&state).await else {
        return fixed_error(StatusCode::SERVICE_UNAVAILABLE, GRAPH_UNAVAILABLE_BODY);
    };
    snapshot
        .communities
        .iter()
        .find(|community| community.id == id)
        .map_or_else(
            || fixed_error(StatusCode::NOT_FOUND, GRAPH_ITEM_NOT_FOUND_BODY),
            |community| {
                Json(GraphCommunity::with_report(
                    community,
                    snapshot
                        .community_reports
                        .iter()
                        .find(|report| report.community_id == community.short_id),
                ))
                .into_response()
            },
        )
}

pub(super) async fn get_community_report(
    State(state): State<Arc<StudioApiState>>,
    path: Result<Path<String>, PathRejection>,
) -> Response {
    let Some(id) = valid_path_id(path) else {
        return fixed_error(StatusCode::BAD_REQUEST, INVALID_GRAPH_REQUEST_BODY);
    };
    let Ok(snapshot) = load_snapshot(&state).await else {
        return fixed_error(StatusCode::SERVICE_UNAVAILABLE, GRAPH_UNAVAILABLE_BODY);
    };
    let Some(community) = snapshot
        .communities
        .iter()
        .find(|community| community.id == id)
    else {
        return fixed_error(StatusCode::NOT_FOUND, GRAPH_ITEM_NOT_FOUND_BODY);
    };
    snapshot
        .community_reports
        .iter()
        .find(|report| report.community_id == community.short_id)
        .map_or_else(
            || fixed_error(StatusCode::NOT_FOUND, GRAPH_ITEM_NOT_FOUND_BODY),
            |report| Json(report.clone()).into_response(),
        )
}

async fn load_snapshot(state: &StudioApiState) -> Result<GraphDataSnapshot, ()> {
    let snapshot = state
        .graph_data_source
        .load_snapshot()
        .await
        .map_err(|_| ())?;
    snapshot.validate().map_err(|_| ())?;
    Ok(snapshot)
}

fn summarize(snapshot: &GraphDataSnapshot) -> Result<GraphSummary, ()> {
    let mut entity_types = std::collections::BTreeMap::<String, u64>::new();
    let mut untyped_entity_count = 0_u64;
    for entity in &snapshot.entities {
        if let Some(entity_type) = &entity.entity_type {
            let count = entity_types.entry(entity_type.clone()).or_default();
            *count = count.checked_add(1).ok_or(())?;
        } else {
            untyped_entity_count = untyped_entity_count.checked_add(1).ok_or(())?;
        }
    }
    let community_levels = snapshot
        .communities
        .iter()
        .map(|community| community.level)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(GraphSummary {
        entity_count: u64::try_from(snapshot.entities.len()).map_err(|_| ())?,
        relationship_count: u64::try_from(snapshot.relationships.len()).map_err(|_| ())?,
        community_count: u64::try_from(snapshot.communities.len()).map_err(|_| ())?,
        community_report_count: u64::try_from(snapshot.community_reports.len()).map_err(|_| ())?,
        community_levels,
        entity_types,
        untyped_entity_count,
    })
}

fn validated_page<const N: usize>(
    limit: Option<usize>,
    after: Option<&str>,
    filters: [Option<&str>; N],
) -> Result<usize, ()> {
    let limit = limit.unwrap_or(DEFAULT_PAGE_LIMIT);
    if !(1..=MAX_PAGE_LIMIT).contains(&limit)
        || after.is_some_and(|value| !valid_id(value))
        || filters
            .into_iter()
            .flatten()
            .any(|value| value.is_empty() || value.len() > MAX_FILTER_BYTES)
    {
        return Err(());
    }
    Ok(limit)
}

fn valid_subgraph_request(request: &GraphSubgraphRequest) -> bool {
    (!request.entity_ids.is_empty() || !request.relationship_ids.is_empty())
        && request.entity_ids.len() <= MAX_ENTITY_SEEDS
        && request.relationship_ids.len() <= MAX_RELATIONSHIP_SEEDS
        && request.depth <= 1
        && valid_projection_limits(request.max_entities, request.max_relationships)
        && request
            .entity_ids
            .iter()
            .chain(&request.relationship_ids)
            .all(|id| valid_id(id))
}

const fn valid_projection_limits(max_entities: usize, max_relationships: usize) -> bool {
    max_entities > 0
        && max_entities <= MAX_PROJECTION_ENTITY_LIMIT
        && max_relationships > 0
        && max_relationships <= MAX_PROJECTION_RELATIONSHIP_LIMIT
}

const fn default_subgraph_depth() -> u8 {
    1
}

const fn default_projection_entity_limit() -> usize {
    DEFAULT_PROJECTION_ENTITY_LIMIT
}

const fn default_projection_relationship_limit() -> usize {
    DEFAULT_PROJECTION_RELATIONSHIP_LIMIT
}

fn valid_path_id(path: Result<Path<String>, PathRejection>) -> Option<String> {
    let Ok(Path(id)) = path else {
        return None;
    };
    valid_id(&id).then_some(id)
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn paginate<T, F>(items: Vec<T>, after: Option<&str>, limit: usize, id: F) -> GraphListResponse<T>
where
    F: Fn(&T) -> &str,
{
    let start = after.map_or(0, |cursor| {
        items
            .iter()
            .position(|item| id(item) == cursor)
            .map_or(items.len(), |index| index.saturating_add(1))
    });
    let mut page = items
        .into_iter()
        .skip(start)
        .take(limit.saturating_add(1))
        .collect::<Vec<_>>();
    let has_more = page.len() > limit;
    page.truncate(limit);
    let next_cursor = has_more
        .then(|| page.last().map(|item| id(item).to_owned()))
        .flatten();
    GraphListResponse {
        items: page,
        next_cursor,
    }
}

fn sort_entities(items: &mut [GraphEntity], sort: EntitySort, order: SortOrder) {
    items.sort_by(|left, right| {
        let primary = match sort {
            EntitySort::Id => ordered(left.id.cmp(&right.id), order),
            EntitySort::Degree => compare_optional(left.degree, right.degree, order),
            EntitySort::Rank => compare_optional(left.rank, right.rank, order),
            EntitySort::Title => ordered(left.title.cmp(&right.title), order),
        };
        primary.then_with(|| left.id.cmp(&right.id))
    });
}

fn sort_relationships(items: &mut [GraphRelationship], sort: RelationshipSort, order: SortOrder) {
    items.sort_by(|left, right| {
        let primary = match sort {
            RelationshipSort::Id => ordered(left.id.cmp(&right.id), order),
            RelationshipSort::Weight => compare_optional_f64(left.weight, right.weight, order),
            RelationshipSort::Rank => compare_optional(left.rank, right.rank, order),
            RelationshipSort::Source => ordered(left.source.cmp(&right.source), order),
            RelationshipSort::Target => ordered(left.target.cmp(&right.target), order),
        };
        primary.then_with(|| left.id.cmp(&right.id))
    });
}

fn compare_optional<T: Ord>(left: Option<T>, right: Option<T>, order: SortOrder) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => ordered(left.cmp(&right), order),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare_optional_f64(left: Option<f64>, right: Option<f64>, order: SortOrder) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => ordered(left.total_cmp(&right), order),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

const fn ordered(value: Ordering, order: SortOrder) -> Ordering {
    match order {
        SortOrder::Asc => value,
        SortOrder::Desc => value.reverse(),
    }
}

fn fixed_error(status: StatusCode, body: &'static str) -> Response {
    (status, body).into_response()
}
