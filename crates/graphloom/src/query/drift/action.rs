//! DRIFT action values and completion metadata.

use crate::query::{QueryContext, QueryUsageCategory};

#[derive(Debug, Clone)]
pub(super) struct DriftActionResponse {
    pub(super) answer: Option<String>,
    pub(super) score: f64,
    pub(super) follow_up_queries: Vec<String>,
}

impl DriftActionResponse {
    pub(super) fn fallback() -> Self {
        Self {
            answer: None,
            score: f64::NEG_INFINITY,
            follow_up_queries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct DriftActionMetadata {
    pub(super) usage: QueryUsageCategory,
    pub(super) context: Option<QueryContext>,
}

#[derive(Debug, Clone)]
pub(super) struct DriftAction {
    pub(super) id: usize,
    pub(super) query: String,
    pub(super) answer: Option<String>,
    pub(super) score: f64,
    pub(super) metadata: DriftActionMetadata,
}

impl DriftAction {
    pub(super) fn incomplete(id: usize, query: String) -> Self {
        Self {
            id,
            query,
            answer: None,
            score: f64::NEG_INFINITY,
            metadata: DriftActionMetadata::default(),
        }
    }
}
