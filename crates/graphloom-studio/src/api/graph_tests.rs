use std::{error::Error, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use graphloom::{
    GraphRagConfig,
    explainability::{
        ExplainabilityLiveHub, ExplainabilityLiveHubOptions, ExplainabilityStore,
        InMemoryExplainabilityStore,
    },
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

use super::{StudioApiOptions, StudioApiService, resolve_table_root};
use crate::graph::{
    GraphCommunityReportSummary, GraphDataSnapshot, GraphDataSource, GraphDataSourceError,
    ParquetGraphDataSource,
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

#[derive(Clone)]
struct FakeGraphDataSource {
    snapshot: Option<GraphDataSnapshot>,
}

impl std::fmt::Debug for FakeGraphDataSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FakeGraphDataSource { .. }")
    }
}

#[async_trait]
impl GraphDataSource for FakeGraphDataSource {
    async fn load_snapshot(&self) -> Result<GraphDataSnapshot, GraphDataSourceError> {
        self.snapshot
            .clone()
            .ok_or(GraphDataSourceError::Unavailable)
    }
}

async fn snapshot() -> TestResult<GraphDataSnapshot> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/graph");
    let mut snapshot = ParquetGraphDataSource::new(root).load_snapshot().await?;
    let mut untyped = snapshot.entities.first().ok_or("entity fixture")?.clone();
    untyped.id = "entity-d".to_owned();
    untyped.short_id = Some("short-entity-d".to_owned());
    untyped.title = "Untyped".to_owned();
    untyped.entity_type = None;
    untyped.description = Some("GRAPH_ENTITY_SECRET_SENTINEL".to_owned());
    snapshot.entities.push(untyped);
    Ok(snapshot)
}

fn router(source: Arc<dyn GraphDataSource>) -> (StudioApiService, Router) {
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new(),
    ));
    let service = StudioApiService::new_with_graph_data_source(
        GraphRagConfig::default(),
        PathBuf::from("GRAPH_PATH_SECRET_SENTINEL"),
        store,
        hub,
        source,
        StudioApiOptions::new(),
    );
    let router = service.router();
    (service, router)
}

async fn get_json(router: &Router, uri: &str) -> TestResult<(StatusCode, Value)> {
    let response = router
        .clone()
        .oneshot(Request::get(uri).body(Body::empty())?)
        .await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

#[tokio::test]
async fn test_should_return_graph_summary_with_typed_and_untyped_counts() -> TestResult {
    let (service, router) = router(Arc::new(FakeGraphDataSource {
        snapshot: Some(snapshot().await?),
    }));
    let (status, value) = get_json(&router, "/api/graph/summary").await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["entity_count"], 4);
    assert_eq!(value["relationship_count"], 3);
    assert_eq!(value["community_count"], 2);
    assert_eq!(value["community_report_count"], 2);
    assert_eq!(value["community_levels"], json!([0, 1]));
    assert_eq!(value["entity_types"], json!({"ORG": 1, "PERSON": 2}));
    assert_eq!(value["untyped_entity_count"], 1);
    assert_eq!(format!("{service:?}"), "StudioApiService { .. }");
    assert!(!format!("{service:?}").contains("GRAPH_PATH_SECRET_SENTINEL"));
    Ok(())
}

#[tokio::test]
async fn test_should_paginate_entities_by_stable_id_and_apply_exact_filters() -> TestResult {
    let (_, router) = router(Arc::new(FakeGraphDataSource {
        snapshot: Some(snapshot().await?),
    }));
    let (status, first) = get_json(&router, "/api/graph/entities?limit=2").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["items"][0]["id"], "entity-a");
    assert_eq!(first["items"][1]["id"], "entity-b");
    assert_eq!(first["next_cursor"], "entity-b");
    let (_, second) = get_json(&router, "/api/graph/entities?limit=2&after=entity-b").await?;
    assert_eq!(second["items"][0]["id"], "entity-c");
    assert_eq!(second["items"][1]["id"], "entity-d");
    assert!(second["next_cursor"].is_null());

    let (_, filtered) = get_json(&router, "/api/graph/entities?type=PERSON&community=5").await?;
    assert_eq!(filtered["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(filtered["items"][0]["id"], "entity-a");
    assert!(filtered["items"][0].get("description").is_none());
    assert!(filtered["items"][0].get("text_unit_ids").is_none());
    Ok(())
}

#[tokio::test]
async fn test_should_paginate_and_filter_relationships_and_communities() -> TestResult {
    let (_, router) = router(Arc::new(FakeGraphDataSource {
        snapshot: Some(snapshot().await?),
    }));
    let (_, relationships) = get_json(
        &router,
        "/api/graph/relationships?source=Alice&target=Bob%20Corp&limit=1",
    )
    .await?;
    assert_eq!(relationships["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(relationships["items"][0]["id"], "relationship-a");
    assert!(relationships["next_cursor"].is_null());
    assert!(relationships["items"][0].get("description").is_none());

    let (_, relationship_first) = get_json(&router, "/api/graph/relationships?limit=2").await?;
    assert_eq!(relationship_first["next_cursor"], "relationship-b");
    let (_, relationship_last) = get_json(
        &router,
        "/api/graph/relationships?limit=2&after=relationship-b",
    )
    .await?;
    assert_eq!(relationship_last["items"][0]["id"], "relationship-c");
    assert!(relationship_last["next_cursor"].is_null());

    let (_, first) = get_json(&router, "/api/graph/communities?limit=1").await?;
    assert_eq!(first["items"][0]["id"], "community-a");
    assert_eq!(first["next_cursor"], "community-a");
    let (_, community_last) =
        get_json(&router, "/api/graph/communities?limit=1&after=community-a").await?;
    assert_eq!(community_last["items"][0]["id"], "community-b");
    assert!(community_last["next_cursor"].is_null());
    let (_, filtered) = get_json(&router, "/api/graph/communities?level=1&parent=3").await?;
    assert_eq!(filtered["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(filtered["items"][0]["id"], "community-a");
    assert_eq!(filtered["items"][0]["report"]["id"], "report-a");
    assert_eq!(filtered["items"][0]["report"]["summary"], "Summary 5");
    assert!(filtered.to_string().find("full_content").is_none());
    Ok(())
}

#[tokio::test]
async fn test_should_return_graph_details_without_exposing_embeddings() -> TestResult {
    let (_, router) = router(Arc::new(FakeGraphDataSource {
        snapshot: Some(snapshot().await?),
    }));
    for (uri, expected_id) in [
        ("/api/graph/entities/entity-a", "entity-a"),
        ("/api/graph/relationships/relationship-a", "relationship-a"),
        ("/api/graph/communities/community-a", "community-a"),
        ("/api/graph/communities/community-a/report", "report-a"),
    ] {
        let (status, value) = get_json(&router, uri).await?;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["id"], expected_id);
        let wire = value.to_string();
        assert!(!wire.contains("embedding"));
        assert!(!wire.contains("vector"));
    }
    let (_, report) = get_json(&router, "/api/graph/communities/community-a/report").await?;
    assert_eq!(report["full_content"], "GRAPH_REPORT_SECRET_SENTINEL");
    Ok(())
}

#[tokio::test]
async fn test_should_return_safe_graph_errors() -> TestResult {
    let (_, available) = router(Arc::new(FakeGraphDataSource {
        snapshot: Some(snapshot().await?),
    }));
    for uri in [
        "/api/graph/entities/missing",
        "/api/graph/relationships/missing",
        "/api/graph/communities/missing",
        "/api/graph/communities/missing/report",
    ] {
        let response = available
            .clone()
            .oneshot(Request::get(uri).body(Body::empty())?)
            .await?;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await?.as_ref(),
            b"graph item not found"
        );
    }
    for uri in [
        "/api/graph/entities?limit=0",
        "/api/graph/entities?limit=201",
        "/api/graph/entities?unknown=value",
        "/api/graph/relationships?unknown=value",
        "/api/graph/communities?level=invalid",
        "/api/graph/communities?parent=invalid",
        "/api/graph/entities/%20",
    ] {
        let response = available
            .clone()
            .oneshot(Request::get(uri).body(Body::empty())?)
            .await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await?.as_ref(),
            b"invalid graph request"
        );
    }

    let (_, unavailable) = router(Arc::new(FakeGraphDataSource { snapshot: None }));
    let response = unavailable
        .oneshot(Request::get("/api/graph/summary").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    assert_eq!(body.as_ref(), b"graph data is unavailable");
    assert!(!String::from_utf8_lossy(&body).contains("GRAPH_PATH_SECRET_SENTINEL"));
    assert_eq!(
        GraphDataSourceError::Unavailable.to_string(),
        "graph data is unavailable"
    );
    Ok(())
}

#[tokio::test]
async fn test_should_start_service_without_index_output_and_return_503() -> TestResult {
    let project = TempDir::new()?;
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new(),
    ));
    let service = StudioApiService::new(
        GraphRagConfig::default(),
        project.path().to_path_buf(),
        store,
        hub,
        StudioApiOptions::new(),
    );
    let response = service
        .router()
        .oneshot(Request::get("/api/graph/summary").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await?.as_ref(),
        b"graph data is unavailable"
    );
    Ok(())
}

#[tokio::test]
async fn test_should_reject_duplicate_snapshot_identities_as_unavailable() -> TestResult {
    let original = snapshot().await?;
    let mut variants = Vec::new();

    let mut duplicate = original.clone();
    duplicate.entities[0].id = duplicate.entities[1].id.clone();
    variants.push(duplicate);
    let mut duplicate = original.clone();
    duplicate.relationships[0].id = duplicate.relationships[1].id.clone();
    variants.push(duplicate);
    let mut duplicate = original.clone();
    duplicate.communities[0].id = duplicate.communities[1].id.clone();
    variants.push(duplicate);
    let mut duplicate = original.clone();
    duplicate.communities[0].short_id = duplicate.communities[1].short_id.clone();
    variants.push(duplicate);
    let mut duplicate = original.clone();
    duplicate.community_reports[0].id = duplicate.community_reports[1].id.clone();
    variants.push(duplicate);
    let mut duplicate = original;
    duplicate.community_reports[0].community_id = "5".to_owned();
    variants.push(duplicate);

    assert!(variants.iter().all(|snapshot| snapshot.validate().is_err()));
    let duplicate = variants.pop().ok_or("duplicate fixture")?;
    let (_, router) = router(Arc::new(FakeGraphDataSource {
        snapshot: Some(duplicate),
    }));
    let response = router
        .oneshot(Request::get("/api/graph/summary").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    Ok(())
}

#[test]
fn test_should_resolve_configured_graph_root_without_http_override() {
    let project = std::path::Path::new("/project");
    assert_eq!(
        resolve_table_root(project, "custom-output"),
        PathBuf::from("/project/custom-output")
    );
    assert_eq!(
        resolve_table_root(project, "/absolute/output"),
        PathBuf::from("/absolute/output")
    );
}

#[tokio::test]
async fn test_should_keep_content_bearing_graph_debug_opaque() -> TestResult {
    let snapshot = snapshot().await?;
    let entity = snapshot
        .entities
        .iter()
        .find(|entity| entity.id == "entity-d")
        .ok_or("entity")?;
    let relationship = snapshot.relationships.first().ok_or("relationship")?;
    let report = snapshot
        .community_reports
        .iter()
        .find(|report| report.community_id == "5")
        .ok_or("report")?;

    assert_eq!(format!("{entity:?}"), "GraphEntityDetail { .. }");
    assert_eq!(
        format!("{relationship:?}"),
        "GraphRelationshipDetail { .. }"
    );
    assert_eq!(
        format!("{:?}", GraphCommunityReportSummary::from(report)),
        "GraphCommunityReportSummary { .. }"
    );
    assert_eq!(format!("{report:?}"), "GraphCommunityReportDetail { .. }");
    let debug = format!("{snapshot:?}");
    assert!(!debug.contains("GRAPH_ENTITY_SECRET_SENTINEL"));
    assert!(!debug.contains("GRAPH_REPORT_SECRET_SENTINEL"));
    Ok(())
}

#[tokio::test]
async fn test_should_return_404_when_query_visible_community_has_no_report() -> TestResult {
    let mut missing_report = snapshot().await?;
    missing_report
        .community_reports
        .retain(|report| report.community_id != "5");
    let (_, router) = router(Arc::new(FakeGraphDataSource {
        snapshot: Some(missing_report),
    }));
    let response = router
        .oneshot(Request::get("/api/graph/communities/community-a/report").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}
