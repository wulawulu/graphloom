//! Query contracts, read-only runtime, data adapters, and search implementations.

mod callbacks;
mod context;
mod data_loader;
mod data_model;
mod error;
mod indexer_adapters;
mod requirements;
mod result;
mod runtime;

pub(crate) mod basic;
pub(crate) mod global;
pub(crate) mod local;

use std::{path::PathBuf, str::FromStr, sync::Arc};

pub use callbacks::{NoopQueryCallbacks, QueryCallbackChain, QueryCallbacks};
use clap::ValueEnum;
pub use context::{ConversationHistory, ConversationRole, ConversationTurn};
pub use data_loader::{
    BasicQueryData, DriftQueryData, GlobalQueryData, LocalQueryData, QueryDataLoader,
};
pub use data_model::{Community, CommunityReport, Covariate, Entity, Relationship, TextUnit};
pub use error::{QueryError, QueryTableErrorDetails, Result};
pub use global::{MapPoint, MapSearchResult};
pub use indexer_adapters::{
    read_indexer_communities, read_indexer_covariates, read_indexer_entities,
    read_indexer_relationships, read_indexer_report_embeddings, read_indexer_reports,
    read_indexer_text_units,
};
pub use requirements::{QueryEmbedding, QueryPrompt, QueryRequirements, QueryTable};
pub use result::{
    QueryContext, QueryContextRecords, QueryContextText, QueryEvent, QueryEventStream, QueryResult,
    QueryUsage, QueryUsageCategory,
};
pub(crate) use runtime::{
    BasicQueryRuntime, GlobalQueryRuntime, LocalQueryRuntime, QueryRuntimeFactory,
};

/// Public `GraphRAG` query method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, ValueEnum)]
#[value(rename_all = "lower")]
#[non_exhaustive]
pub enum SearchMethod {
    /// Global community map/reduce search.
    Global,
    /// Local graph-neighborhood search.
    Local,
    /// DRIFT exploratory search.
    Drift,
    /// Basic text-unit vector search.
    Basic,
}

impl std::fmt::Display for SearchMethod {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Global => "global",
            Self::Local => "local",
            Self::Drift => "drift",
            Self::Basic => "basic",
        })
    }
}

impl FromStr for SearchMethod {
    type Err = QueryError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "global" => Ok(Self::Global),
            "local" => Ok(Self::Local),
            "drift" => Ok(Self::Drift),
            "basic" => Ok(Self::Basic),
            _ => Err(QueryError::QueryMethod {
                method: None,
                operation: "parse query method",
                message: format!("unknown query method {value:?}"),
            }),
        }
    }
}

/// Options shared by the unified Query API.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct QueryOptions {
    /// Project root used for relative paths and prompts.
    pub project_root: PathBuf,
    /// Original query text.
    pub query: String,
    /// Query method.
    pub method: SearchMethod,
    /// Optional Parquet table directory override.
    pub data_dir: Option<PathBuf>,
    /// Maximum community hierarchy level.
    pub community_level: i64,
    /// Whether Global Search uses dynamic community selection.
    pub dynamic_community_selection: bool,
    /// Requested answer shape.
    pub response_type: String,
    /// Query callbacks.
    pub callbacks: Vec<Arc<dyn QueryCallbacks>>,
    /// Optional prior conversation turns.
    pub conversation_history: Option<ConversationHistory>,
}

impl QueryOptions {
    /// Create options with GraphRAG-compatible CLI defaults.
    #[must_use]
    pub fn new(project_root: PathBuf, query: String, method: SearchMethod) -> Self {
        Self {
            project_root,
            query,
            method,
            data_dir: None,
            community_level: 2,
            dynamic_community_selection: false,
            response_type: "Multiple Paragraphs".to_owned(),
            callbacks: Vec::new(),
            conversation_history: None,
        }
    }
}
