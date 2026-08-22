//! Business records, validated identifiers, and persisted envelopes.

use std::{fmt, num::NonZeroU64, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use thiserror::Error;
use uuid::Uuid;

use super::{ExplainabilityEvent, ExplainabilityRecordType};

/// Current explainability transport-schema version.
pub const EXPLAINABILITY_SCHEMA_VERSION: u32 = 1;

const MAX_IDENTIFIER_BYTES: usize = 128;
const REDACTED_IDENTIFIER: &str = "[redacted]";

/// Validation error for explainability transport contracts.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ExplainabilityContractError {
    /// An identifier was empty, too long, or contained a disallowed byte.
    #[error("invalid {kind}: {reason}")]
    InvalidIdentifier {
        /// Identifier category.
        kind: &'static str,
        /// Stable validation failure description that excludes the rejected value.
        reason: &'static str,
    },
    /// A candidate score was NaN or infinite.
    #[error("explainability score must be finite")]
    NonFiniteScore,
    /// A candidate's record category differed from its containing event.
    #[error(
        "candidate at index {candidate_index} has record type {actual:?}; expected {expected:?}"
    )]
    CandidateTypeMismatch {
        /// Homogeneous record category declared by the containing event.
        expected: ExplainabilityRecordType,
        /// Record category declared by the mismatched candidate.
        actual: ExplainabilityRecordType,
        /// Zero-based position of the mismatched candidate.
        candidate_index: usize,
    },
    /// A declared collection count did not match the bounded collection length.
    #[error("{collection} count does not match the collection length")]
    CollectionCountMismatch {
        /// Stable collection name without request content.
        collection: &'static str,
    },
    /// A Global map point declared a different batch identity than its event.
    #[error("Global map point at index {point_index} has a mismatched batch index")]
    GlobalMapPointBatchMismatch {
        /// Zero-based point position in the event.
        point_index: usize,
    },
    /// A Global map point's declared index differed from its position in the event.
    #[error("Global map point at index {point_index} has a mismatched point index")]
    GlobalMapPointOrderMismatch {
        /// Zero-based position of the point in the event.
        point_index: usize,
    },
    /// A Global Reduce point decision contradicted its score, selected flag, or reason.
    #[error("Global Reduce point decision at index {point_index} is inconsistent: {reason}")]
    InvalidGlobalReduceDecision {
        /// Zero-based point position in the event.
        point_index: usize,
        /// Stable validation failure description without point content.
        reason: &'static str,
    },
    /// Global Reduce token usage exceeded its declared budget.
    #[error("Global Reduce token usage exceeds its token budget")]
    GlobalReduceTokensExceedBudget,
    /// A Dynamic Global count or configuration value contradicted its contract.
    #[error("invalid Dynamic Global selection metadata: {reason}")]
    InvalidDynamicSelection {
        /// Stable validation failure description without request content.
        reason: &'static str,
    },
    /// A Dynamic Global rating attempt declared an impossible repeat identity.
    #[error("Dynamic Global rating repeat index must be less than repeat count")]
    InvalidDynamicRatingRepeat,
    /// A Dynamic Global selected decision did not pass the configured threshold.
    #[error("Dynamic Global selected rating evidence must have passed the threshold")]
    InvalidDynamicRatingDecision,
    /// DRIFT event metadata contradicted the real lifecycle or collection shape.
    #[error("invalid DRIFT explainability metadata: {reason}")]
    InvalidDriftMetadata {
        /// Stable validation failure description without captured content.
        reason: &'static str,
    },
    /// A persisted envelope used a schema version this reader does not support.
    #[error("unsupported explainability schema version {actual}; expected {expected}")]
    UnsupportedSchemaVersion {
        /// Version found in the serialized envelope.
        actual: u32,
        /// Version supported by this contract.
        expected: u32,
    },
    /// A persisted sequence was zero.
    #[error("explainability sequence must be greater than zero")]
    ZeroSequence,
}

fn validate_identifier(value: &str, kind: &'static str) -> Result<(), ExplainabilityContractError> {
    if value.is_empty() {
        return Err(ExplainabilityContractError::InvalidIdentifier {
            kind,
            reason: "value must not be empty",
        });
    }
    if value.len() > MAX_IDENTIFIER_BYTES {
        return Err(ExplainabilityContractError::InvalidIdentifier {
            kind,
            reason: "value exceeds 128 bytes",
        });
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(ExplainabilityContractError::InvalidIdentifier {
            kind,
            reason: "only ASCII letters, digits, hyphen, underscore, period, and colon are allowed",
        });
    }
    Ok(())
}

macro_rules! explainability_id {
    ($name:ident, $kind:literal, $docs:literal) => {
        #[doc = $docs]
        #[derive(Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            /// Generate a random UUID-form identifier.
            #[must_use]
            pub fn generate() -> Self {
                Self(Uuid::new_v4().to_string())
            }

            /// Borrow the validated identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_tuple(stringify!($name))
                    .field(&REDACTED_IDENTIFIER)
                    .finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = ExplainabilityContractError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                validate_identifier(value, $kind)?;
                Ok(Self(value.to_owned()))
            }
        }

        impl TryFrom<String> for $name {
            type Error = ExplainabilityContractError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                validate_identifier(&value, $kind)?;
                Ok(Self(value))
            }
        }

        impl TryFrom<&str> for $name {
            type Error = ExplainabilityContractError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                value.parse()
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::try_from(value).map_err(D::Error::custom)
            }
        }
    };
}

explainability_id!(
    ExplainabilityRunId,
    "explainability run id",
    "Validated identity of one explainability run. Values contain 1–128 ASCII bytes and may use \
     letters, digits, `-`, `_`, `.`, or `:`. Debug output is redacted."
);

explainability_id!(
    ExplainabilitySpanId,
    "explainability span id",
    "Validated identity of one business span. Values contain 1–128 ASCII bytes and may use \
     letters, digits, `-`, `_`, `.`, or `:`. Debug output is redacted."
);

/// One business event produced by `GraphLoom` Core before persistence ordering is assigned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExplainabilityRecord {
    /// Run that owns the event.
    pub run_id: ExplainabilityRunId,
    /// UTC time at which the business event occurred.
    #[serde(with = "datetime_serde")]
    pub timestamp: DateTime<Utc>,
    /// Business span that emitted the event.
    pub span_id: ExplainabilitySpanId,
    /// Parent business span, when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<ExplainabilitySpanId>,
    /// Structured business event.
    pub event: ExplainabilityEvent,
}

impl ExplainabilityRecord {
    /// Create a record with complete run and span identity.
    #[must_use]
    pub fn new(
        run_id: ExplainabilityRunId,
        timestamp: DateTime<Utc>,
        span_id: ExplainabilitySpanId,
        parent_span_id: Option<ExplainabilitySpanId>,
        event: ExplainabilityEvent,
    ) -> Self {
        Self {
            run_id,
            timestamp,
            span_id,
            parent_span_id,
            event,
        }
    }
}

/// Versioned transport unit produced by the single persistence writer.
///
/// The nested [`ExplainabilityRecord`] is the exact value consumed by live sinks. Keeping it nested
/// avoids copying run, span, timestamp, and event fields while making writer-owned fields explicit.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ExplainabilityEnvelope {
    schema_version: u32,
    sequence: NonZeroU64,
    /// Original business record, unchanged.
    pub record: ExplainabilityRecord,
}

impl ExplainabilityEnvelope {
    /// Add a persistence sequence to a business record.
    ///
    /// # Errors
    ///
    /// Returns [`ExplainabilityContractError::ZeroSequence`] when `sequence` is zero.
    pub fn new(
        sequence: u64,
        record: ExplainabilityRecord,
    ) -> Result<Self, ExplainabilityContractError> {
        let sequence =
            NonZeroU64::new(sequence).ok_or(ExplainabilityContractError::ZeroSequence)?;
        Ok(Self {
            schema_version: EXPLAINABILITY_SCHEMA_VERSION,
            sequence,
            record,
        })
    }

    /// Return the schema version encoded by this envelope.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Return the non-zero per-run persistence sequence.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence.get()
    }
}

#[derive(Deserialize)]
struct ExplainabilityEnvelopeWire {
    schema_version: u32,
    sequence: u64,
    record: ExplainabilityRecord,
}

impl<'de> Deserialize<'de> for ExplainabilityEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ExplainabilityEnvelopeWire::deserialize(deserializer)?;
        if wire.schema_version != EXPLAINABILITY_SCHEMA_VERSION {
            return Err(D::Error::custom(
                ExplainabilityContractError::UnsupportedSchemaVersion {
                    actual: wire.schema_version,
                    expected: EXPLAINABILITY_SCHEMA_VERSION,
                },
            ));
        }
        Self::new(wire.sequence, wire.record).map_err(D::Error::custom)
    }
}

pub(crate) mod datetime_serde {
    use chrono::{DateTime, SecondsFormat, Utc};
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    pub(crate) fn serialize<S>(value: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_rfc3339_opts(SecondsFormat::Nanos, true))
    }

    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        DateTime::parse_from_rfc3339(&value)
            .map(|timestamp| timestamp.with_timezone(&Utc))
            .map_err(D::Error::custom)
    }

    pub(crate) mod option {
        use chrono::{DateTime, SecondsFormat, Utc};
        use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

        #[allow(
            clippy::ref_option,
            reason = "serde `with` requires a serializer that borrows the field's exact Option \
                      type"
        )]
        pub(crate) fn serialize<S>(
            value: &Option<DateTime<Utc>>,
            serializer: S,
        ) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            value
                .as_ref()
                .map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Nanos, true))
                .serialize(serializer)
        }

        pub(crate) fn deserialize<'de, D>(
            deserializer: D,
        ) -> Result<Option<DateTime<Utc>>, D::Error>
        where
            D: Deserializer<'de>,
        {
            Option::<String>::deserialize(deserializer)?
                .map(|value| {
                    DateTime::parse_from_rfc3339(&value)
                        .map(|timestamp| timestamp.with_timezone(&Utc))
                        .map_err(D::Error::custom)
                })
                .transpose()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use chrono::{TimeZone, Utc};
    use serde_json::json;

    use super::{
        EXPLAINABILITY_SCHEMA_VERSION, ExplainabilityContractError, ExplainabilityEnvelope,
        ExplainabilityRecord, ExplainabilityRunId, ExplainabilitySpanId,
    };
    use crate::explainability::{ExplainabilityEvent, QueryStarted};

    fn sample_record() -> Result<ExplainabilityRecord, ExplainabilityContractError> {
        Ok(ExplainabilityRecord::new(
            ExplainabilityRunId::from_str("run-1")?,
            Utc.with_ymd_and_hms(2026, 8, 3, 12, 30, 0).single().ok_or(
                ExplainabilityContractError::InvalidIdentifier {
                    kind: "test timestamp",
                    reason: "timestamp must be representable",
                },
            )?,
            ExplainabilitySpanId::from_str("span-2")?,
            Some(ExplainabilitySpanId::from_str("span-1")?),
            ExplainabilityEvent::QueryStarted(QueryStarted::new(
                crate::explainability::ExplainabilityQueryMethod::Local,
            )),
        ))
    }

    #[test]
    fn test_should_validate_ids_and_redact_debug_output() -> Result<(), ExplainabilityContractError>
    {
        let run_id = ExplainabilityRunId::from_str("run.valid_1:child")?;
        assert_eq!(run_id.as_str(), "run.valid_1:child");
        assert_eq!(run_id.to_string(), "run.valid_1:child");
        assert_eq!(format!("{run_id:?}"), "ExplainabilityRunId(\"[redacted]\")");

        assert!(ExplainabilityRunId::from_str("").is_err());
        assert!(ExplainabilityRunId::from_str(&"a".repeat(129)).is_err());
        assert!(ExplainabilityRunId::from_str("contains/slash").is_err());
        assert!(ExplainabilitySpanId::from_str("contains space").is_err());
        Ok(())
    }

    #[test]
    fn test_should_validate_ids_during_serde_round_trip() -> Result<(), Box<dyn std::error::Error>>
    {
        let id = ExplainabilitySpanId::from_str("span-123")?;
        let json = serde_json::to_string(&id)?;
        assert_eq!(json, "\"span-123\"");
        assert_eq!(serde_json::from_str::<ExplainabilitySpanId>(&json)?, id);
        assert!(serde_json::from_str::<ExplainabilitySpanId>("\"bad/value\"").is_err());
        Ok(())
    }

    #[test]
    fn test_should_round_trip_nested_envelope_without_duplicate_record_fields()
    -> Result<(), Box<dyn std::error::Error>> {
        let envelope = ExplainabilityEnvelope::new(1, sample_record()?)?;
        let value = serde_json::to_value(&envelope)?;
        assert_eq!(
            value
                .get("schema_version")
                .and_then(serde_json::Value::as_u64),
            Some(u64::from(EXPLAINABILITY_SCHEMA_VERSION))
        );
        assert_eq!(
            value.get("sequence").and_then(serde_json::Value::as_u64),
            Some(1)
        );
        let record = value.get("record").ok_or("record must be present")?;
        assert_eq!(
            record.get("run_id").and_then(serde_json::Value::as_str),
            Some("run-1")
        );
        assert_eq!(
            record.get("span_id").and_then(serde_json::Value::as_str),
            Some("span-2")
        );
        assert_eq!(
            record
                .get("parent_span_id")
                .and_then(serde_json::Value::as_str),
            Some("span-1")
        );
        assert_eq!(
            record.get("timestamp").and_then(serde_json::Value::as_str),
            Some("2026-08-03T12:30:00.000000000Z")
        );
        assert!(value.get("run_id").is_none());
        assert_eq!(
            serde_json::from_value::<ExplainabilityEnvelope>(value)?,
            envelope
        );
        Ok(())
    }

    #[test]
    fn test_should_reject_zero_sequence_and_unsupported_schema()
    -> Result<(), Box<dyn std::error::Error>> {
        assert!(matches!(
            ExplainabilityEnvelope::new(0, sample_record()?),
            Err(ExplainabilityContractError::ZeroSequence)
        ));
        let invalid = json!({
            "schema_version": 2,
            "sequence": 1,
            "record": serde_json::to_value(sample_record()?)?,
        });
        assert!(serde_json::from_value::<ExplainabilityEnvelope>(invalid).is_err());
        Ok(())
    }
}
