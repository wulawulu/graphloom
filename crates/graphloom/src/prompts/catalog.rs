//! Built-in `GraphRAG` prompt metadata.

/// `GraphRAG` prompt kinds used by indexing workflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PromptKind {
    /// Entity and relationship extraction.
    ExtractGraph,
    /// Entity and relationship extraction continuation.
    ExtractGraphContinue,
    /// Entity and relationship extraction loop check.
    ExtractGraphLoop,
    /// Entity and relationship description summarization.
    SummarizeDescriptions,
    /// Claim extraction.
    ExtractClaims,
    /// Claim extraction continuation.
    ExtractClaimsContinue,
    /// Claim extraction loop check.
    ExtractClaimsLoop,
    /// Community report generation.
    CommunityReport,
}

impl PromptKind {
    /// Return the canonical project filename.
    pub(crate) const fn filename(self) -> &'static str {
        match self {
            Self::ExtractGraph => "extract_graph.txt",
            Self::ExtractGraphContinue => "extract_graph_continue.txt",
            Self::ExtractGraphLoop => "extract_graph_loop.txt",
            Self::SummarizeDescriptions => "summarize_descriptions.txt",
            Self::ExtractClaims => "extract_claims.txt",
            Self::ExtractClaimsContinue => "extract_claims_continue.txt",
            Self::ExtractClaimsLoop => "extract_claims_loop.txt",
            Self::CommunityReport => "community_report.txt",
        }
    }

    /// Return the embedded `GraphRAG` template.
    pub(crate) const fn default_template(self) -> &'static str {
        match self {
            Self::ExtractGraph => include_str!("defaults/extract_graph.txt"),
            Self::ExtractGraphContinue => {
                "MANY entities and relationships were missed in the last extraction. Remember to \
                 ONLY emit entities that match any of the previously extracted types. Add them \
                 below using the same format:\n"
            }
            Self::ExtractGraphLoop => {
                "It appears some entities and relationships may have still been missed. Answer Y \
                 if there are still entities or relationships that need to be added, or N if there \
                 are none. Please answer with a single letter Y or N.\n"
            }
            Self::SummarizeDescriptions => {
                include_str!("defaults/summarize_descriptions.txt")
            }
            Self::ExtractClaims => include_str!("defaults/extract_claims.txt"),
            Self::ExtractClaimsContinue => {
                "MANY entities were missed in the last extraction.  Add them below using the same \
                 format:\n"
            }
            Self::ExtractClaimsLoop => {
                "It appears some entities may have still been missed. Answer Y if there are still \
                 entities that need to be added, or N if there are none. Please answer with a \
                 single letter Y or N.\n"
            }
            Self::CommunityReport => include_str!("defaults/community_report.txt"),
        }
    }

    /// Return variables supplied by the workflow for this prompt.
    pub(crate) const fn variables(self) -> &'static [&'static str] {
        match self {
            Self::ExtractGraph => &["entity_types", "input_text"],
            Self::SummarizeDescriptions => &["entity_name", "description_list", "max_length"],
            Self::ExtractClaims => &["entity_specs", "claim_description", "input_text"],
            Self::CommunityReport => &["input_text", "max_report_length"],
            Self::ExtractGraphContinue
            | Self::ExtractGraphLoop
            | Self::ExtractClaimsContinue
            | Self::ExtractClaimsLoop => &[],
        }
    }

    /// Return the stable prompt kind name used in diagnostics.
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::ExtractGraph => "ExtractGraph",
            Self::ExtractGraphContinue => "ExtractGraphContinue",
            Self::ExtractGraphLoop => "ExtractGraphLoop",
            Self::SummarizeDescriptions => "SummarizeDescriptions",
            Self::ExtractClaims => "ExtractClaims",
            Self::ExtractClaimsContinue => "ExtractClaimsContinue",
            Self::ExtractClaimsLoop => "ExtractClaimsLoop",
            Self::CommunityReport => "CommunityReport",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_expose_prompt_tuning_metadata_without_public_api() {
        assert_eq!(
            PromptKind::CommunityReport.filename(),
            "community_report.txt"
        );
        assert_eq!(
            PromptKind::ExtractGraph.variables(),
            &["entity_types", "input_text"]
        );
        assert_eq!(
            PromptKind::ExtractClaims.variables(),
            &["entity_specs", "claim_description", "input_text"]
        );
        assert!(PromptKind::ExtractClaimsLoop.variables().is_empty());
    }
}
