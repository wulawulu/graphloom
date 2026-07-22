//! Project prompt kinds and their built-in Tera templates.

/// `GraphRAG` prompt kinds managed by indexing and query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum PromptKind {
    /// Entity and relationship extraction.
    ExtractGraph,
    /// Entity and relationship description summarization.
    SummarizeDescriptions,
    /// Claim extraction.
    ExtractClaims,
    /// Graph-context community report generation.
    CommunityReportGraph,
    /// Text-context community report generation.
    CommunityReportText,
    /// Basic Search system prompt.
    BasicSearch,
    /// DRIFT local-search system prompt.
    DriftSearch,
    /// DRIFT final reduce prompt.
    DriftReduce,
    /// Global Search map prompt.
    GlobalSearchMap,
    /// Global Search reduce prompt.
    GlobalSearchReduce,
    /// Global Search general-knowledge instruction.
    GlobalSearchKnowledge,
    /// Local Search system prompt.
    LocalSearch,
    /// Question generation system prompt.
    QuestionGeneration,
}

impl PromptKind {
    /// Return every prompt kind managed as a project resource.
    pub(crate) const fn all() -> &'static [Self] {
        &[
            Self::ExtractGraph,
            Self::SummarizeDescriptions,
            Self::ExtractClaims,
            Self::CommunityReportGraph,
            Self::CommunityReportText,
            Self::BasicSearch,
            Self::DriftSearch,
            Self::DriftReduce,
            Self::GlobalSearchMap,
            Self::GlobalSearchReduce,
            Self::GlobalSearchKnowledge,
            Self::LocalSearch,
            Self::QuestionGeneration,
        ]
    }

    /// Return the canonical project filename.
    pub(crate) const fn filename(self) -> &'static str {
        match self {
            Self::ExtractGraph => "extract_graph.txt",
            Self::SummarizeDescriptions => "summarize_descriptions.txt",
            Self::ExtractClaims => "extract_claims.txt",
            Self::CommunityReportGraph => "community_report_graph.txt",
            Self::CommunityReportText => "community_report_text.txt",
            Self::BasicSearch => "basic_search_system_prompt.txt",
            Self::DriftSearch => "drift_search_system_prompt.txt",
            Self::DriftReduce => "drift_reduce_prompt.txt",
            Self::GlobalSearchMap => "global_search_map_system_prompt.txt",
            Self::GlobalSearchReduce => "global_search_reduce_system_prompt.txt",
            Self::GlobalSearchKnowledge => "global_search_knowledge_system_prompt.txt",
            Self::LocalSearch => "local_search_system_prompt.txt",
            Self::QuestionGeneration => "question_gen_system_prompt.txt",
        }
    }

    /// Return the embedded `GraphRAG` template.
    pub(crate) const fn default_template(self) -> &'static str {
        match self {
            Self::ExtractGraph => include_str!("defaults/extract_graph.txt"),
            Self::SummarizeDescriptions => {
                include_str!("defaults/summarize_descriptions.txt")
            }
            Self::ExtractClaims => include_str!("defaults/extract_claims.txt"),
            Self::CommunityReportGraph => include_str!("defaults/community_report_graph.txt"),
            Self::CommunityReportText => include_str!("defaults/community_report_text.txt"),
            Self::BasicSearch => include_str!("defaults/basic_search_system_prompt.txt"),
            Self::DriftSearch => include_str!("defaults/drift_search_system_prompt.txt"),
            Self::DriftReduce => include_str!("defaults/drift_reduce_prompt.txt"),
            Self::GlobalSearchMap => {
                include_str!("defaults/global_search_map_system_prompt.txt")
            }
            Self::GlobalSearchReduce => {
                include_str!("defaults/global_search_reduce_system_prompt.txt")
            }
            Self::GlobalSearchKnowledge => {
                include_str!("defaults/global_search_knowledge_system_prompt.txt")
            }
            Self::LocalSearch => include_str!("defaults/local_search_system_prompt.txt"),
            Self::QuestionGeneration => include_str!("defaults/question_gen_system_prompt.txt"),
        }
    }

    /// Return the embedded Chinese `GraphRAG`-compatible template.
    pub(crate) const fn chinese_template(self) -> &'static str {
        match self {
            Self::ExtractGraph => include_str!("defaults/zh/extract_graph.txt"),
            Self::SummarizeDescriptions => {
                include_str!("defaults/zh/summarize_descriptions.txt")
            }
            Self::ExtractClaims => include_str!("defaults/zh/extract_claims.txt"),
            Self::CommunityReportGraph => {
                include_str!("defaults/zh/community_report_graph.txt")
            }
            Self::CommunityReportText => {
                include_str!("defaults/zh/community_report_text.txt")
            }
            Self::BasicSearch => include_str!("defaults/zh/basic_search_system_prompt.txt"),
            Self::DriftSearch => include_str!("defaults/zh/drift_search_system_prompt.txt"),
            Self::DriftReduce => include_str!("defaults/zh/drift_reduce_prompt.txt"),
            Self::GlobalSearchMap => {
                include_str!("defaults/zh/global_search_map_system_prompt.txt")
            }
            Self::GlobalSearchReduce => {
                include_str!("defaults/zh/global_search_reduce_system_prompt.txt")
            }
            Self::GlobalSearchKnowledge => {
                include_str!("defaults/zh/global_search_knowledge_system_prompt.txt")
            }
            Self::LocalSearch => include_str!("defaults/zh/local_search_system_prompt.txt"),
            Self::QuestionGeneration => {
                include_str!("defaults/zh/question_gen_system_prompt.txt")
            }
        }
    }

    /// Return variables supplied by the workflow for this prompt.
    pub(crate) const fn variables(self) -> &'static [&'static str] {
        match self {
            Self::ExtractGraph => &["entity_types", "input_text"],
            Self::SummarizeDescriptions => &["entity_name", "description_list", "max_length"],
            Self::ExtractClaims => &["entity_specs", "claim_description", "input_text"],
            Self::CommunityReportGraph | Self::CommunityReportText => {
                &["input_text", "max_report_length"]
            }
            Self::BasicSearch | Self::LocalSearch | Self::DriftReduce => {
                &["context_data", "response_type"]
            }
            Self::DriftSearch => &["context_data", "response_type", "global_query", "followups"],
            Self::GlobalSearchMap => &["context_data", "max_length"],
            Self::GlobalSearchReduce => &["report_data", "response_type", "max_length"],
            Self::GlobalSearchKnowledge => &[],
            Self::QuestionGeneration => &["question_count", "context_data"],
        }
    }

    /// Return the stable prompt kind name used in diagnostics.
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::ExtractGraph => "ExtractGraph",
            Self::SummarizeDescriptions => "SummarizeDescriptions",
            Self::ExtractClaims => "ExtractClaims",
            Self::CommunityReportGraph => "CommunityReportGraph",
            Self::CommunityReportText => "CommunityReportText",
            Self::BasicSearch => "BasicSearch",
            Self::DriftSearch => "DriftSearch",
            Self::DriftReduce => "DriftReduce",
            Self::GlobalSearchMap => "GlobalSearchMap",
            Self::GlobalSearchReduce => "GlobalSearchReduce",
            Self::GlobalSearchKnowledge => "GlobalSearchKnowledge",
            Self::LocalSearch => "LocalSearch",
            Self::QuestionGeneration => "QuestionGeneration",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn test_should_expose_prompt_tuning_metadata_without_public_api() {
        assert_eq!(
            PromptKind::CommunityReportGraph.filename(),
            "community_report_graph.txt"
        );
        assert_eq!(
            PromptKind::CommunityReportText.filename(),
            "community_report_text.txt"
        );
        assert_eq!(
            PromptKind::ExtractGraph.variables(),
            &["entity_types", "input_text"]
        );
        assert_eq!(
            PromptKind::ExtractClaims.variables(),
            &["entity_specs", "claim_description", "input_text"]
        );
    }

    #[test]
    fn test_all_prompt_kinds_have_unique_filenames() {
        let filenames = PromptKind::all()
            .iter()
            .map(|kind| kind.filename())
            .collect::<BTreeSet<_>>();

        assert_eq!(filenames.len(), PromptKind::all().len());
    }

    #[test]
    fn test_should_expose_all_configurable_project_prompt_assets() {
        assert_eq!(
            PromptKind::all()
                .iter()
                .map(|kind| kind.filename())
                .collect::<Vec<_>>(),
            vec![
                "extract_graph.txt",
                "summarize_descriptions.txt",
                "extract_claims.txt",
                "community_report_graph.txt",
                "community_report_text.txt",
                "basic_search_system_prompt.txt",
                "drift_search_system_prompt.txt",
                "drift_reduce_prompt.txt",
                "global_search_map_system_prompt.txt",
                "global_search_reduce_system_prompt.txt",
                "global_search_knowledge_system_prompt.txt",
                "local_search_system_prompt.txt",
                "question_gen_system_prompt.txt",
            ]
        );
    }

    #[test]
    fn test_all_prompt_kinds_have_non_empty_default_templates() {
        for kind in PromptKind::all() {
            assert!(
                !kind.default_template().trim().is_empty(),
                "{} must have a default template",
                kind.filename(),
            );
            assert!(
                !kind.chinese_template().trim().is_empty(),
                "{} must have a Chinese template",
                kind.filename(),
            );
        }
    }
}
