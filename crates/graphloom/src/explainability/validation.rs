//! Serde boundary limits for persisted explainability content.

use std::{fmt, marker::PhantomData};

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as _, SeqAccess, Visitor},
    ser::Error as _,
};

use super::{ContextSectionBudget, ExplainabilityCandidate};

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
    fn test_should_reject_oversized_record_id_and_candidate_collections() {
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
        let retrieved = CandidatesRetrieved::new(
            ExplainabilityRecordType::Entity,
            vec![candidate; MAX_CANDIDATES.saturating_add(1)],
        );
        assert!(serde_json::to_value(retrieved).is_err());
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
