//! Project prompt kinds and their built-in Tera templates.

/// `GraphRAG` prompt kinds used by indexing workflows.
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
    fn test_should_expose_only_configurable_index_prompt_assets() {
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
        }
    }
}
