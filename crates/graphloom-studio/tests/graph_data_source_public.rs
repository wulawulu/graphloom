use std::{error::Error, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use axum::{
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
use graphloom_studio::{
    api::{StudioApiOptions, StudioApiService},
    graph::{
        GraphCommunity, GraphCommunityReportDetail, GraphDataSnapshot, GraphDataSource,
        GraphDataSourceError, GraphEntityDetail, GraphRelationshipDetail,
    },
};
use serde_json::Value;
use tower::ServiceExt;

#[derive(Clone)]
struct ExternalGraphDataSource {
    snapshot: GraphDataSnapshot,
}

impl std::fmt::Debug for ExternalGraphDataSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ExternalGraphDataSource { .. }")
    }
}

#[async_trait]
impl GraphDataSource for ExternalGraphDataSource {
    async fn load_snapshot(&self) -> Result<GraphDataSnapshot, GraphDataSourceError> {
        Ok(self.snapshot.clone())
    }
}

#[tokio::test]
async fn test_should_implement_custom_datasource_outside_library_crate()
-> Result<(), Box<dyn Error>> {
    let entity = GraphEntityDetail::new("entity-1".to_owned(), "Alice".to_owned())
        .with_short_id("1".to_owned())
        .with_entity_type("PERSON".to_owned())
        .with_description("Alice detail".to_owned())
        .with_rank(2)
        .with_community_ids(vec!["7".to_owned()])
        .with_text_unit_ids(vec!["text-1".to_owned()]);
    let relationship = GraphRelationshipDetail::new(
        "relationship-1".to_owned(),
        "Alice".to_owned(),
        "External".to_owned(),
    )
    .with_description("External edge".to_owned())
    .with_weight(1.0)
    .with_rank(2)
    .with_text_unit_ids(vec!["text-1".to_owned()]);
    let community = GraphCommunity::new(
        "community-1".to_owned(),
        "7".to_owned(),
        "Community 7".to_owned(),
    )
    .with_hierarchy(1, -1, Vec::new());
    let report = GraphCommunityReportDetail::new(
        "report-1".to_owned(),
        "7".to_owned(),
        "Report 7".to_owned(),
        "Summary".to_owned(),
        "Full content".to_owned(),
    )
    .with_rank(9.0);
    let source: Arc<dyn GraphDataSource> = Arc::new(ExternalGraphDataSource {
        snapshot: GraphDataSnapshot::new(
            vec![entity],
            vec![relationship],
            vec![community],
            vec![report],
        )?,
    });
    let store: Arc<dyn ExplainabilityStore> = Arc::new(InMemoryExplainabilityStore::new());
    let hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new(),
    ));
    let response = StudioApiService::new_with_graph_data_source(
        GraphRagConfig::default(),
        PathBuf::from("."),
        store,
        hub,
        source,
        StudioApiOptions::new(),
    )
    .router()
    .oneshot(Request::get("/api/graph/summary").body(Body::empty())?)
    .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let value: Value = serde_json::from_slice(&body)?;
    assert_eq!(value["entity_count"], 1);
    assert_eq!(value["relationship_count"], 1);
    assert_eq!(value["community_count"], 1);
    assert_eq!(value["community_report_count"], 1);
    Ok(())
}
