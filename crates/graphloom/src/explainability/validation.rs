//! Serde boundary limits for persisted explainability content.

use std::{fmt, marker::PhantomData};

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as _, SeqAccess, Visitor},
    ser::Error as _,
};

use super::{
    ContextSectionBudget, DriftRankedReportEvidence, DynamicCommunityRatingEvidence,
    ExplainabilityCandidate, GlobalMapPointDecision, GlobalMapPointEvidence,
};

/// Maximum bytes for record IDs, model IDs, codes, titles, and other short metadata.
pub(crate) const MAX_METADATA_STRING_BYTES: usize = 256;
/// Maximum bytes for safe displayable error and warning messages.
pub(crate) const MAX_MESSAGE_BYTES: usize = 4 * 1_024;
/// Maximum bytes for explicitly enabled query, context, prompt, or response content.
pub(crate) const MAX_CONTENT_BYTES: usize = 1024 * 1024;
/// Maximum candidate DTOs in one event.
pub(crate) const MAX_CANDIDATES: usize = 10_000;
/// Maximum record IDs attached to one event or context section.
pub(crate) const MAX_RECORD_IDS: usize = 10_000;
/// Maximum logical context-section budgets in one event.
pub(crate) const MAX_CONTEXT_SECTIONS: usize = 32;
/// Maximum Global map points or Reduce decisions in one event.
pub(crate) const MAX_GLOBAL_MAP_POINTS: usize = 10_000;
/// Maximum Dynamic Global communities represented by one event.
pub(crate) const MAX_DYNAMIC_COMMUNITIES: usize = 10_000;
/// Maximum DRIFT evidence entries in one event.
pub(crate) const MAX_DRIFT_ITEMS: usize = 10_000;

pub(crate) fn validate_dynamic_id(value: &str, label: &str) -> Result<(), String> {
    validate_string(value, MAX_METADATA_STRING_BYTES, label)?;
    if value.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(format!(
            "{label} may contain only ASCII letters, digits, hyphen, underscore, period, or colon"
        ));
    }
    Ok(())
}

fn validate_string(value: &str, max_bytes: usize, label: &str) -> Result<(), String> {
    if value.len() > max_bytes {
        return Err(format!("{label} exceeds {max_bytes} bytes"));
    }
    Ok(())
}

fn serialize_string<S>(
    value: &str,
    serializer: S,
    max_bytes: usize,
    label: &str,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    validate_string(value, max_bytes, label).map_err(S::Error::custom)?;
    serializer.serialize_str(value)
}

struct BoundedStringVisitor {
    max_bytes: usize,
    label: &'static str,
}

impl Visitor<'_> for BoundedStringVisitor {
    type Value = String;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "a {} string no longer than {} bytes",
            self.label, self.max_bytes
        )
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        validate_string(value, self.max_bytes, self.label).map_err(E::custom)?;
        Ok(value.to_owned())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        validate_string(&value, self.max_bytes, self.label).map_err(E::custom)?;
        Ok(value)
    }
}

fn deserialize_string<'de, D>(
    deserializer: D,
    max_bytes: usize,
    label: &'static str,
) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_string(BoundedStringVisitor { max_bytes, label })
}

#[derive(Debug)]
struct BoundedString<const MAX: usize>(String);

impl<'de, const MAX: usize> Deserialize<'de> for BoundedString<MAX> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_string(deserializer, MAX, "optional field").map(Self)
    }
}

#[allow(
    clippy::ref_option,
    reason = "shared implementation serves serde `with` serializers that borrow exact Option \
              fields"
)]
fn serialize_optional_string<S>(
    value: &Option<String>,
    serializer: S,
    max_bytes: usize,
    label: &str,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let Some(value) = value {
        validate_string(value, max_bytes, label).map_err(S::Error::custom)?;
    }
    value.serialize(serializer)
}

fn deserialize_optional_string<'de, D, const MAX: usize>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<BoundedString<MAX>>::deserialize(deserializer)
        .map(|value| value.map(|bounded| bounded.0))
}

struct BoundedVecVisitor<T, const MAX: usize>(PhantomData<T>);

impl<'de, T, const MAX: usize> Visitor<'de> for BoundedVecVisitor<T, MAX>
where
    T: Deserialize<'de>,
{
    type Value = Vec<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "an array containing at most {MAX} elements")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let capacity = sequence.size_hint().unwrap_or_default().min(MAX);
        let mut values = Vec::with_capacity(capacity);
        while let Some(value) = sequence.next_element()? {
            if values.len() == MAX {
                return Err(A::Error::custom(format!("array exceeds {MAX} elements")));
            }
            values.push(value);
        }
        Ok(values)
    }
}

fn deserialize_vec<'de, D, T, const MAX: usize>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    deserializer.deserialize_seq(BoundedVecVisitor::<T, MAX>(PhantomData))
}

fn serialize_slice<S, T>(
    value: &[T],
    serializer: S,
    max_elements: usize,
    label: &str,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    if value.len() > max_elements {
        return Err(S::Error::custom(format!(
            "{label} exceeds {max_elements} elements"
        )));
    }
    value.serialize(serializer)
}

macro_rules! string_module {
    ($module:ident, $max:expr, $label:literal) => {
        pub(crate) mod $module {
            use serde::{Deserializer, Serializer};

            pub(crate) fn serialize<S>(value: &str, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                super::serialize_string(value, serializer, $max, $label)
            }

            pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<String, D::Error>
            where
                D: Deserializer<'de>,
            {
                super::deserialize_string(deserializer, $max, $label)
            }
        }
    };
}

macro_rules! optional_string_module {
    ($module:ident, $max:expr, $label:literal) => {
        pub(crate) mod $module {
            use serde::{Deserializer, Serializer};

            #[allow(
                clippy::ref_option,
                reason = "serde `with` requires a serializer that borrows the field's exact \
                          Option type"
            )]
            pub(crate) fn serialize<S>(
                value: &Option<String>,
                serializer: S,
            ) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                super::serialize_optional_string(value, serializer, $max, $label)
            }

            pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
            where
                D: Deserializer<'de>,
            {
                super::deserialize_optional_string::<'de, D, { $max }>(deserializer)
            }
        }
    };
}

string_module!(
    metadata_string,
    super::MAX_METADATA_STRING_BYTES,
    "metadata field"
);
string_module!(message_string, super::MAX_MESSAGE_BYTES, "message field");
optional_string_module!(
    optional_metadata_string,
    super::MAX_METADATA_STRING_BYTES,
    "metadata field"
);
optional_string_module!(
    optional_content_string,
    super::MAX_CONTENT_BYTES,
    "content field"
);

pub(crate) mod record_ids {
    use serde::{Deserializer, Serializer, ser::Error as _};

    use super::{
        BoundedString, MAX_METADATA_STRING_BYTES, MAX_RECORD_IDS, deserialize_vec, validate_string,
    };

    pub(crate) fn serialize<S>(value: &[String], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if value.len() > MAX_RECORD_IDS {
            return Err(S::Error::custom(format!(
                "record ID array exceeds {MAX_RECORD_IDS} elements"
            )));
        }
        for id in value {
            validate_string(id, MAX_METADATA_STRING_BYTES, "record ID")
                .map_err(S::Error::custom)?;
        }
        serde::Serialize::serialize(value, serializer)
    }

    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_vec::<D, BoundedString<MAX_METADATA_STRING_BYTES>, MAX_RECORD_IDS>(deserializer)
            .map(|values| values.into_iter().map(|value| value.0).collect())
    }
}

pub(crate) mod action_ids {
    use serde::{Deserializer, Serializer};

    use super::{MAX_DRIFT_ITEMS, deserialize_vec, serialize_slice};

    pub(crate) fn serialize<S>(value: &[u64], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_slice(value, serializer, MAX_DRIFT_ITEMS, "DRIFT action ID array")
    }

    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_vec::<D, u64, MAX_DRIFT_ITEMS>(deserializer)
    }
}

pub(crate) mod content_strings {
    use serde::{Deserializer, Serializer, ser::Error as _};

    use super::{BoundedString, MAX_CONTENT_BYTES, MAX_DRIFT_ITEMS, deserialize_vec};

    pub(crate) fn serialize<S>(value: &[String], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if value.len() > MAX_DRIFT_ITEMS {
            return Err(S::Error::custom(format!(
                "DRIFT content array exceeds {MAX_DRIFT_ITEMS} elements"
            )));
        }
        for item in value {
            super::validate_string(item, MAX_CONTENT_BYTES, "DRIFT content item")
                .map_err(S::Error::custom)?;
        }
        serde::Serialize::serialize(value, serializer)
    }

    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_vec::<D, BoundedString<MAX_CONTENT_BYTES>, MAX_DRIFT_ITEMS>(deserializer)
            .map(|items| items.into_iter().map(|item| item.0).collect())
    }
}

pub(crate) mod optional_content_strings {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[allow(
        clippy::ref_option,
        reason = "serde `with` requires a serializer that borrows the exact Option type"
    )]
    pub(crate) fn serialize<S>(
        value: &Option<Vec<String>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(items) => serializer.serialize_some(&ValidatedContentStrings(items)),
            None => serializer.serialize_none(),
        }
    }

    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<ContentStrings>::deserialize(deserializer).map(|value| value.map(|item| item.0))
    }

    struct ValidatedContentStrings<'a>(&'a [String]);

    impl Serialize for ValidatedContentStrings<'_> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            super::content_strings::serialize(self.0, serializer)
        }
    }

    struct ContentStrings(Vec<String>);

    impl<'de> Deserialize<'de> for ContentStrings {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            super::content_strings::deserialize(deserializer).map(Self)
        }
    }
}

pub(crate) mod drift_ranked_reports {
    use serde::{Deserializer, Serializer};

    use super::{DriftRankedReportEvidence, MAX_DRIFT_ITEMS, deserialize_vec, serialize_slice};

    pub(crate) fn serialize<S>(
        value: &[DriftRankedReportEvidence],
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_slice(
            value,
            serializer,
            MAX_DRIFT_ITEMS,
            "DRIFT ranked report array",
        )
    }

    pub(crate) fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<Vec<DriftRankedReportEvidence>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_vec::<D, DriftRankedReportEvidence, MAX_DRIFT_ITEMS>(deserializer)
    }
}

pub(crate) mod candidates {
    use serde::{Deserializer, Serializer};

    use super::{ExplainabilityCandidate, MAX_CANDIDATES, deserialize_vec, serialize_slice};

    pub(crate) fn serialize<S>(
        value: &[ExplainabilityCandidate],
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_slice(value, serializer, MAX_CANDIDATES, "candidate array")
    }

    pub(crate) fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<Vec<ExplainabilityCandidate>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_vec::<D, ExplainabilityCandidate, MAX_CANDIDATES>(deserializer)
    }
}

pub(crate) mod context_sections {
    use serde::{Deserializer, Serializer};

    use super::{ContextSectionBudget, MAX_CONTEXT_SECTIONS, deserialize_vec, serialize_slice};

    pub(crate) fn serialize<S>(
        value: &[ContextSectionBudget],
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_slice(
            value,
            serializer,
            MAX_CONTEXT_SECTIONS,
            "context section array",
        )
    }

    pub(crate) fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<Vec<ContextSectionBudget>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_vec::<D, ContextSectionBudget, MAX_CONTEXT_SECTIONS>(deserializer)
    }
}

pub(crate) mod global_map_points {
    use serde::{Deserializer, Serializer};

    use super::{GlobalMapPointEvidence, MAX_GLOBAL_MAP_POINTS, deserialize_vec, serialize_slice};

    pub(crate) fn serialize<S>(
        value: &[GlobalMapPointEvidence],
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_slice(
            value,
            serializer,
            MAX_GLOBAL_MAP_POINTS,
            "Global map point array",
        )
    }

    pub(crate) fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<Vec<GlobalMapPointEvidence>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_vec::<D, GlobalMapPointEvidence, MAX_GLOBAL_MAP_POINTS>(deserializer)
    }
}

pub(crate) mod global_map_point_decisions {
    use serde::{Deserializer, Serializer};

    use super::{GlobalMapPointDecision, MAX_GLOBAL_MAP_POINTS, deserialize_vec, serialize_slice};

    pub(crate) fn serialize<S>(
        value: &[GlobalMapPointDecision],
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_slice(
            value,
            serializer,
            MAX_GLOBAL_MAP_POINTS,
            "Global map point decision array",
        )
    }

    pub(crate) fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<Vec<GlobalMapPointDecision>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_vec::<D, GlobalMapPointDecision, MAX_GLOBAL_MAP_POINTS>(deserializer)
    }
}

pub(crate) mod dynamic_community_ids {
    use serde::{Deserializer, Serializer, ser::Error as _};

    use super::{
        BoundedString, MAX_DYNAMIC_COMMUNITIES, MAX_METADATA_STRING_BYTES, deserialize_vec,
        validate_dynamic_id,
    };

    pub(crate) fn serialize<S>(value: &[String], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if value.len() > MAX_DYNAMIC_COMMUNITIES {
            return Err(S::Error::custom(format!(
                "Dynamic community ID array exceeds {MAX_DYNAMIC_COMMUNITIES} elements"
            )));
        }
        for id in value {
            validate_dynamic_id(id, "Dynamic community ID").map_err(S::Error::custom)?;
        }
        serde::Serialize::serialize(value, serializer)
    }

    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = deserialize_vec::<
            D,
            BoundedString<MAX_METADATA_STRING_BYTES>,
            MAX_DYNAMIC_COMMUNITIES,
        >(deserializer)?;
        let values = values.into_iter().map(|value| value.0).collect::<Vec<_>>();
        for id in &values {
            validate_dynamic_id(id, "Dynamic community ID").map_err(serde::de::Error::custom)?;
        }
        Ok(values)
    }
}

pub(crate) mod dynamic_rating_evidence {
    use serde::{Deserializer, Serializer};

    use super::{
        DynamicCommunityRatingEvidence, MAX_DYNAMIC_COMMUNITIES, deserialize_vec, serialize_slice,
    };

    pub(crate) fn serialize<S>(
        value: &[DynamicCommunityRatingEvidence],
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_slice(
            value,
            serializer,
            MAX_DYNAMIC_COMMUNITIES,
            "Dynamic rating evidence array",
        )
    }

    pub(crate) fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<Vec<DynamicCommunityRatingEvidence>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_vec::<D, DynamicCommunityRatingEvidence, MAX_DYNAMIC_COMMUNITIES>(deserializer)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        MAX_CANDIDATES, MAX_CONTENT_BYTES, MAX_CONTEXT_SECTIONS, MAX_MESSAGE_BYTES,
        MAX_METADATA_STRING_BYTES, MAX_RECORD_IDS,
    };
    use crate::explainability::{
        CandidatesRetrieved, ContextSectionBudget, ContextSectionKind, ExplainabilityCandidate,
        ExplainabilityQueryMethod, ExplainabilityRecordType, GraphExpansionStarted, QueryStarted,
        RunFailed,
    };

    #[test]
    fn test_should_reject_oversized_metadata_on_read_and_write() {
        let oversized = "x".repeat(MAX_METADATA_STRING_BYTES.saturating_add(1));
        let candidate =
            ExplainabilityCandidate::new(oversized.clone(), ExplainabilityRecordType::Entity);
        assert!(serde_json::to_value(candidate).is_err());

        let value = json!({
            "id": oversized,
            "record_type": "entity",
            "selected": false,
        });
        assert!(serde_json::from_value::<ExplainabilityCandidate>(value).is_err());
    }

    #[test]
    fn test_should_reject_oversized_content_and_messages_on_read_and_write() {
        let mut query = QueryStarted::new(ExplainabilityQueryMethod::Local);
        query.query = Some("q".repeat(MAX_CONTENT_BYTES.saturating_add(1)));
        assert!(serde_json::to_value(query).is_err());
        let query_value = json!({
            "method": "local",
            "query": "q".repeat(MAX_CONTENT_BYTES.saturating_add(1)),
        });
        assert!(serde_json::from_value::<QueryStarted>(query_value).is_err());

        let failed = RunFailed::new(
            "query_error".to_owned(),
            "m".repeat(MAX_MESSAGE_BYTES.saturating_add(1)),
        );
        assert!(serde_json::to_value(failed).is_err());
    }

    #[test]
    fn test_should_reject_oversized_record_id_and_candidate_collections()
    -> Result<(), Box<dyn std::error::Error>> {
        let expansion = GraphExpansionStarted::new(vec![
            "entity-1".to_owned();
            MAX_RECORD_IDS.saturating_add(1)
        ]);
        assert!(serde_json::to_value(expansion).is_err());
        let expansion_value = json!({
            "seed_entity_ids": vec!["entity-1"; MAX_RECORD_IDS.saturating_add(1)],
        });
        assert!(serde_json::from_value::<GraphExpansionStarted>(expansion_value).is_err());

        let candidate =
            ExplainabilityCandidate::new("entity-1".to_owned(), ExplainabilityRecordType::Entity);
        let retrieved = CandidatesRetrieved::try_new(
            ExplainabilityRecordType::Entity,
            vec![candidate; MAX_CANDIDATES.saturating_add(1)],
        )?;
        assert!(serde_json::to_value(retrieved).is_err());
        Ok(())
    }

    #[test]
    fn test_should_reject_oversized_context_section_collection() {
        let section = ContextSectionBudget::new(ContextSectionKind::Entities, 100);
        let value = json!({
            "total_token_budget": 100,
            "sections": vec![section; MAX_CONTEXT_SECTIONS.saturating_add(1)],
        });
        assert!(
            serde_json::from_value::<crate::explainability::ContextBudgetAllocated>(value).is_err()
        );
    }
}
