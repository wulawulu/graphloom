//! Explainability content-disclosure levels.

use serde::{Deserialize, Serialize};

/// Controls how much user and model content an explainability producer may include.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExplainabilityContentMode {
    /// Include identifiers, counts, scores, ranks, timing, and other metadata only.
    #[default]
    Metadata,
    /// Permit full non-secret query, context, prompt, and response content.
    Content,
    /// Permit additional non-secret diagnostic details.
    Debug,
}

impl ExplainabilityContentMode {
    /// Return whether full non-secret query and model content may be recorded.
    #[must_use]
    pub const fn includes_content(self) -> bool {
        matches!(self, Self::Content | Self::Debug)
    }
}

#[cfg(test)]
mod tests {
    use super::ExplainabilityContentMode;

    #[test]
    fn test_should_default_to_metadata_and_keep_stable_json_values() -> serde_json::Result<()> {
        assert_eq!(
            ExplainabilityContentMode::default(),
            ExplainabilityContentMode::Metadata
        );
        assert!(!ExplainabilityContentMode::Metadata.includes_content());
        assert!(ExplainabilityContentMode::Content.includes_content());
        assert!(ExplainabilityContentMode::Debug.includes_content());
        for (mode, expected) in [
            (ExplainabilityContentMode::Metadata, "\"metadata\""),
            (ExplainabilityContentMode::Content, "\"content\""),
            (ExplainabilityContentMode::Debug, "\"debug\""),
        ] {
            let json = serde_json::to_string(&mode)?;
            assert_eq!(json, expected);
            assert_eq!(
                serde_json::from_str::<ExplainabilityContentMode>(&json)?,
                mode
            );
        }
        Ok(())
    }
}
