//! Method-specific read-only Query capability introspection.
//!
//! Runtime loading remains explicit and method-typed so adapter dependencies and error operations
//! stay reviewable. Exact-set tests below keep this public matrix synchronized; it is not a
//! generic runtime loader.

use std::collections::BTreeSet;

use super::SearchMethod;

/// Persisted table kinds consumed by Query methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum QueryTable {
    /// Entity table.
    Entities,
    /// Community hierarchy table.
    Communities,
    /// Community report table.
    CommunityReports,
    /// Text-unit table.
    TextUnits,
    /// Relationship table.
    Relationships,
    /// Optional covariate table.
    Covariates,
}

impl QueryTable {
    /// Return the canonical Parquet table name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Entities => "entities",
            Self::Communities => "communities",
            Self::CommunityReports => "community_reports",
            Self::TextUnits => "text_units",
            Self::Relationships => "relationships",
            Self::Covariates => "covariates",
        }
    }
}

/// Vector index kinds consumed by Query methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum QueryEmbedding {
    /// Entity-description vector index.
    EntityDescription,
    /// Community-full-content vector index.
    CommunityFullContent,
    /// Text-unit-text vector index.
    TextUnitText,
}

impl QueryEmbedding {
    /// Return the canonical vector index name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::EntityDescription => crate::ENTITY_DESCRIPTION_EMBEDDING,
            Self::CommunityFullContent => crate::COMMUNITY_FULL_CONTENT_EMBEDDING,
            Self::TextUnitText => crate::TEXT_UNIT_TEXT_EMBEDDING,
        }
    }
}

/// Prompt kinds consumed by Query methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum QueryPrompt {
    /// Basic Search prompt.
    Basic,
    /// Local Search prompt.
    Local,
    /// Global map prompt.
    GlobalMap,
    /// Global reduce prompt.
    GlobalReduce,
    /// Global general-knowledge prompt.
    GlobalKnowledge,
    /// DRIFT search prompt.
    Drift,
    /// DRIFT reduce prompt.
    DriftReduce,
}

/// Resources required to assemble one Query method without unrelated I/O.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueryRequirements {
    /// Required Parquet tables.
    pub tables: BTreeSet<QueryTable>,
    /// Optional Parquet tables.
    pub optional_tables: BTreeSet<QueryTable>,
    /// Required vector indices.
    pub embeddings: BTreeSet<QueryEmbedding>,
    /// Required completion model identifiers.
    pub completion_models: BTreeSet<String>,
    /// Required embedding model identifiers.
    pub embedding_models: BTreeSet<String>,
    /// Required prompt kinds.
    pub prompts: BTreeSet<QueryPrompt>,
}

impl QueryRequirements {
    /// Resolve the exact resource matrix for `method`.
    #[must_use]
    pub fn for_method(method: SearchMethod, config: &crate::GraphRagConfig) -> Self {
        let mut value = Self::default();
        match method {
            SearchMethod::Global => {
                value.tables.extend([
                    QueryTable::Entities,
                    QueryTable::Communities,
                    QueryTable::CommunityReports,
                ]);
                value
                    .completion_models
                    .insert(config.global_search.completion_model_id.clone());
                value.prompts.extend([
                    QueryPrompt::GlobalMap,
                    QueryPrompt::GlobalReduce,
                    QueryPrompt::GlobalKnowledge,
                ]);
            }
            SearchMethod::Local => {
                value.tables.extend([
                    QueryTable::Entities,
                    QueryTable::Communities,
                    QueryTable::CommunityReports,
                    QueryTable::TextUnits,
                    QueryTable::Relationships,
                ]);
                value.optional_tables.insert(QueryTable::Covariates);
                value.embeddings.insert(QueryEmbedding::EntityDescription);
                value
                    .completion_models
                    .insert(config.local_search.completion_model_id.clone());
                value
                    .embedding_models
                    .insert(config.local_search.embedding_model_id.clone());
                value.prompts.insert(QueryPrompt::Local);
            }
            SearchMethod::Drift => {
                value.tables.extend([
                    QueryTable::Entities,
                    QueryTable::Communities,
                    QueryTable::CommunityReports,
                    QueryTable::TextUnits,
                    QueryTable::Relationships,
                ]);
                value.embeddings.extend([
                    QueryEmbedding::EntityDescription,
                    QueryEmbedding::CommunityFullContent,
                ]);
                value
                    .completion_models
                    .insert(config.drift_search.completion_model_id.clone());
                value
                    .embedding_models
                    .insert(config.drift_search.embedding_model_id.clone());
                value
                    .prompts
                    .extend([QueryPrompt::Drift, QueryPrompt::DriftReduce]);
            }
            SearchMethod::Basic => {
                value.tables.insert(QueryTable::TextUnits);
                value.embeddings.insert(QueryEmbedding::TextUnitText);
                value
                    .completion_models
                    .insert(config.basic_search.completion_model_id.clone());
                value
                    .embedding_models
                    .insert(config.basic_search.embedding_model_id.clone());
                value.prompts.insert(QueryPrompt::Basic);
            }
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_resolve_exact_method_resource_matrix() {
        let config = crate::GraphRagConfig::default();
        assert_eq!(
            QueryRequirements::for_method(SearchMethod::Global, &config),
            QueryRequirements {
                tables: BTreeSet::from([
                    QueryTable::Entities,
                    QueryTable::Communities,
                    QueryTable::CommunityReports,
                ]),
                completion_models: BTreeSet::from([config
                    .global_search
                    .completion_model_id
                    .clone()]),
                prompts: BTreeSet::from([
                    QueryPrompt::GlobalMap,
                    QueryPrompt::GlobalReduce,
                    QueryPrompt::GlobalKnowledge,
                ]),
                ..QueryRequirements::default()
            }
        );

        assert_eq!(
            QueryRequirements::for_method(SearchMethod::Local, &config),
            QueryRequirements {
                tables: BTreeSet::from([
                    QueryTable::Entities,
                    QueryTable::Communities,
                    QueryTable::CommunityReports,
                    QueryTable::TextUnits,
                    QueryTable::Relationships,
                ]),
                optional_tables: BTreeSet::from([QueryTable::Covariates]),
                embeddings: BTreeSet::from([QueryEmbedding::EntityDescription]),
                completion_models: BTreeSet::from([config
                    .local_search
                    .completion_model_id
                    .clone()]),
                embedding_models: BTreeSet::from([config.local_search.embedding_model_id.clone()]),
                prompts: BTreeSet::from([QueryPrompt::Local]),
            }
        );

        assert_eq!(
            QueryRequirements::for_method(SearchMethod::Drift, &config),
            QueryRequirements {
                tables: BTreeSet::from([
                    QueryTable::Entities,
                    QueryTable::Communities,
                    QueryTable::CommunityReports,
                    QueryTable::TextUnits,
                    QueryTable::Relationships,
                ]),
                embeddings: BTreeSet::from([
                    QueryEmbedding::EntityDescription,
                    QueryEmbedding::CommunityFullContent,
                ]),
                completion_models: BTreeSet::from([config
                    .drift_search
                    .completion_model_id
                    .clone()]),
                embedding_models: BTreeSet::from([config.drift_search.embedding_model_id.clone()]),
                prompts: BTreeSet::from([QueryPrompt::Drift, QueryPrompt::DriftReduce]),
                ..QueryRequirements::default()
            }
        );

        assert_eq!(
            QueryRequirements::for_method(SearchMethod::Basic, &config),
            QueryRequirements {
                tables: BTreeSet::from([QueryTable::TextUnits]),
                embeddings: BTreeSet::from([QueryEmbedding::TextUnitText]),
                completion_models: BTreeSet::from([config
                    .basic_search
                    .completion_model_id
                    .clone()]),
                embedding_models: BTreeSet::from([config.basic_search.embedding_model_id.clone()]),
                prompts: BTreeSet::from([QueryPrompt::Basic]),
                ..QueryRequirements::default()
            }
        );
    }
}
