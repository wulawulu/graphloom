//! Feature-gated Axum host for the Studio API and optional Vite assets.

use std::{fmt, net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{Router, http::StatusCode, routing::any};
use graphloom::{
    GraphLoomError,
    explainability::{
        ExplainabilityLiveHub, ExplainabilityLiveHubOptions, ExplainabilityStore,
        ExplainabilityStoreError, SqliteExplainabilityStore,
    },
    load_project_config,
};
use thiserror::Error;
use tower_http::services::{ServeDir, ServeFile};

use crate::api::{StudioApiOptions, StudioApiService};

const DEFAULT_DATABASE_DIRECTORY: &str = ".graphloom-studio";
const DEFAULT_DATABASE_NAME: &str = "explainability.sqlite";

/// Runtime options for the feature-gated Studio executable host.
#[derive(Clone)]
#[non_exhaustive]
pub struct StudioServerOptions {
    root: PathBuf,
    listen: SocketAddr,
    assets_dir: Option<PathBuf>,
    explainability_db: Option<PathBuf>,
}

impl fmt::Debug for StudioServerOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StudioServerOptions { .. }")
    }
}

impl StudioServerOptions {
    /// Create API-only options for a project root, listening on localhost port 8080.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            listen: SocketAddr::from(([127, 0, 0, 1], 8080)),
            assets_dir: None,
            explainability_db: None,
        }
    }

    /// Override the socket address. Non-loopback addresses expose the unauthenticated MVP.
    #[must_use]
    pub const fn with_listen(mut self, listen: SocketAddr) -> Self {
        self.listen = listen;
        self
    }

    /// Serve a Vite build directory with an `index.html` SPA fallback.
    #[must_use]
    pub fn with_assets_dir(mut self, assets_dir: PathBuf) -> Self {
        self.assets_dir = Some(assets_dir);
        self
    }

    /// Override the Explainability `SQLite` path.
    ///
    /// Relative values are resolved from the loaded project root.
    #[must_use]
    pub fn with_explainability_db(mut self, explainability_db: PathBuf) -> Self {
        self.explainability_db = Some(explainability_db);
        self
    }
}

/// Safe Studio host startup or serving failure.
#[derive(Error)]
#[non_exhaustive]
pub enum StudioServerError {
    /// Project configuration could not be loaded.
    #[error("GraphLoom Studio project configuration is unavailable")]
    Project(#[source] GraphLoomError),
    /// The Explainability database directory could not be prepared.
    #[error("GraphLoom Studio data directory is unavailable")]
    DataDirectory(#[source] std::io::Error),
    /// The Explainability `SQLite` Store could not be opened.
    #[error("GraphLoom Studio Explainability Store is unavailable")]
    Store(#[source] ExplainabilityStoreError),
    /// The supplied asset directory does not contain a readable index document.
    #[error("GraphLoom Studio frontend assets are unavailable")]
    Assets(#[source] std::io::Error),
    /// The configured listen socket could not be bound.
    #[error("GraphLoom Studio listen socket is unavailable")]
    Bind(#[source] std::io::Error),
    /// The Axum server stopped with an I/O failure.
    #[error("GraphLoom Studio server failed")]
    Serve(#[source] std::io::Error),
}

impl fmt::Debug for StudioServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StudioServerError { .. }")
    }
}

/// Load the project, open its Studio Store, and serve until Ctrl+C.
///
/// # Errors
///
/// Returns [`StudioServerError`] when project loading, Store startup, static
/// asset validation, socket binding, or serving fails.
pub async fn serve(options: StudioServerOptions) -> Result<(), StudioServerError> {
    let project = load_project_config(&options.root)
        .await
        .map_err(StudioServerError::Project)?;
    let database_path = resolve_database_path(&project.root, options.explainability_db.as_ref());
    let Some(database_parent) = database_path.parent() else {
        return Err(StudioServerError::DataDirectory(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "database path has no parent",
        )));
    };
    tokio::fs::create_dir_all(database_parent)
        .await
        .map_err(StudioServerError::DataDirectory)?;
    let store: Arc<dyn ExplainabilityStore> = Arc::new(
        SqliteExplainabilityStore::open(&database_path)
            .await
            .map_err(StudioServerError::Store)?,
    );
    let live_hub = Arc::new(ExplainabilityLiveHub::new(
        ExplainabilityLiveHubOptions::new(),
    ));
    let api = StudioApiService::new(
        project.config,
        project.root,
        store,
        live_hub,
        StudioApiOptions::new(),
    )
    .router();
    let router = build_host_router(api, options.assets_dir.as_ref()).await?;
    let listener = tokio::net::TcpListener::bind(options.listen)
        .await
        .map_err(StudioServerError::Bind)?;
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(StudioServerError::Serve)
}

fn resolve_database_path(root: &std::path::Path, override_path: Option<&PathBuf>) -> PathBuf {
    match override_path {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => root.join(path),
        None => root
            .join(DEFAULT_DATABASE_DIRECTORY)
            .join(DEFAULT_DATABASE_NAME),
    }
}

async fn build_host_router(
    api: Router,
    assets_dir: Option<&PathBuf>,
) -> Result<Router, StudioServerError> {
    let router = api
        .route("/api", any(api_not_found))
        .route("/api/{*path}", any(api_not_found));
    let Some(assets_dir) = assets_dir else {
        return Ok(router);
    };
    let index = assets_dir.join("index.html");
    let metadata = tokio::fs::metadata(&index)
        .await
        .map_err(StudioServerError::Assets)?;
    if !metadata.is_file() {
        return Err(StudioServerError::Assets(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "frontend index is not a file",
        )));
    }
    Ok(router
        .route_service("/", ServeFile::new(index.clone()))
        .fallback_service(ServeDir::new(assets_dir).fallback(ServeFile::new(index))))
}

async fn api_not_found() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "Studio API route not found")
}

async fn shutdown_signal() {
    let _signal_result = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request, routing::get};
    use tempfile::tempdir;
    use tower::ServiceExt;

    use super::{StudioServerError, build_host_router};

    async fn body(response: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("response body");
        String::from_utf8(bytes.to_vec()).expect("UTF-8 response")
    }

    #[tokio::test]
    async fn test_should_keep_api_only_router_operational() {
        let router = build_host_router(
            axum::Router::new().route("/api/probe", get(|| async { "api" })),
            None,
        )
        .await
        .expect("router");
        let response = router
            .oneshot(
                Request::get("/api/probe")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn test_should_serve_assets_and_spa_without_falling_back_for_unknown_api() {
        let assets = tempdir().expect("assets");
        tokio::fs::create_dir(assets.path().join("assets"))
            .await
            .expect("asset directory");
        tokio::fs::write(
            assets.path().join("index.html"),
            "<html>STUDIO_INDEX</html>",
        )
        .await
        .expect("index");
        tokio::fs::write(assets.path().join("assets/test.js"), "STUDIO_ASSET")
            .await
            .expect("asset");
        let router = build_host_router(axum::Router::new(), Some(&assets.path().to_path_buf()))
            .await
            .expect("router");

        for path in ["/", "/frontend/deep/link"] {
            let response = router
                .clone()
                .oneshot(Request::get(path).body(Body::empty()).expect("request"))
                .await
                .expect("response");
            assert_eq!(response.status(), axum::http::StatusCode::OK, "{path}");
            assert!(body(response).await.contains("STUDIO_INDEX"));
        }
        let asset = router
            .clone()
            .oneshot(
                Request::get("/assets/test.js")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(asset.status(), axum::http::StatusCode::OK);
        assert_eq!(body(asset).await, "STUDIO_ASSET");

        let unknown_api = router
            .oneshot(
                Request::get("/api/not-real")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(unknown_api.status(), axum::http::StatusCode::NOT_FOUND);
        assert!(!body(unknown_api).await.contains("STUDIO_INDEX"));
    }

    #[tokio::test]
    async fn test_should_reject_assets_without_index_document() {
        let assets = tempdir().expect("assets");
        let error = build_host_router(axum::Router::new(), Some(&assets.path().to_path_buf()))
            .await
            .expect_err("missing index");
        assert!(matches!(error, StudioServerError::Assets(_)));
        assert_eq!(format!("{error:?}"), "StudioServerError { .. }");
    }
}
