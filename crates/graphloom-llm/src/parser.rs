//! Parsers for GraphRAG LLM output formats.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{LlmError, Result};

/// GraphRAG tuple delimiter.
pub const TUPLE_DELIMITER: &str = "<|>";
/// GraphRAG record delimiter.
pub const RECORD_DELIMITER: &str = "##";
/// GraphRAG completion delimiter.
pub const COMPLETION_DELIMITER: &str = "<|COMPLETE|>";

/// Parsed graph extraction output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphExtraction {
    /// Entity records.
    pub entities: Vec<EntityRecord>,
    /// Relationship records.
    pub relationships: Vec<RelationshipRecord>,
}

/// Parsed entity record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityRecord {
    /// Entity title.
    pub title: String,
    /// Entity type.
    pub entity_type: String,
    /// Entity description.
    pub description: String,
    /// Source text unit id.
    pub source_id: String,
}

/// Parsed relationship record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipRecord {
    /// Source entity title.
    pub source: String,
    /// Target entity title.
    pub target: String,
    /// Relationship description.
    pub description: String,
    /// Source text unit id.
    pub source_id: String,
    /// Relationship strength.
    pub weight: f64,
}

/// Parsed claim record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimRecord {
    /// Subject entity id.
    pub subject_id: Option<String>,
    /// Object entity id.
    pub object_id: Option<String>,
    /// Claim type.
    pub claim_type: Option<String>,
    /// Claim status.
    pub status: Option<String>,
    /// Claim start date.
    pub start_date: Option<String>,
    /// Claim end date.
    pub end_date: Option<String>,
    /// Claim description.
    pub description: Option<String>,
    /// Source quote.
    pub source_text: Option<String>,
}

/// Community report JSON output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityReport {
    /// Report title.
    pub title: String,
    /// Executive summary.
    pub summary: String,
    /// Impact or importance rating.
    pub rating: f64,
    /// Rating explanation.
    #[serde(alias = "rating_explanation")]
    pub rating_explanation: String,
    /// Detailed findings.
    pub findings: Vec<CommunityFinding>,
}

/// Community report finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityFinding {
    /// Finding summary.
    pub summary: String,
    /// Finding explanation.
    pub explanation: String,
}

/// Parse GraphRAG entity and relationship tuple output.
#[must_use]
pub fn parse_graph_tuples(result: &str, source_id: &str) -> GraphExtraction {
    let mut entities = Vec::new();
    let mut relationships = Vec::new();

    for raw_record in result.split(RECORD_DELIMITER) {
        let record = trim_tuple_record(raw_record);
        if record.is_empty() || record == COMPLETION_DELIMITER {
            continue;
        }

        let fields = record.split(TUPLE_DELIMITER).collect::<Vec<_>>();
        match fields.first().copied() {
            Some("\"entity\"") if fields.len() >= 4 => entities.push(EntityRecord {
                title: clean_str(&fields[1].to_uppercase()),
                entity_type: clean_str(&fields[2].to_uppercase()),
                description: clean_str(fields[3]),
                source_id: source_id.to_owned(),
            }),
            Some("\"relationship\"") if fields.len() >= 5 => {
                relationships.push(RelationshipRecord {
                    source: clean_str(&fields[1].to_uppercase()),
                    target: clean_str(&fields[2].to_uppercase()),
                    description: clean_str(fields[3]),
                    source_id: source_id.to_owned(),
                    weight: fields
                        .last()
                        .and_then(|weight| weight.parse::<f64>().ok())
                        .unwrap_or(1.0),
                })
            }
            _ => {}
        }
    }

    GraphExtraction {
        entities,
        relationships,
    }
}

/// Parse GraphRAG claim tuple output.
#[must_use]
pub fn parse_claim_tuples(claims: &str) -> Vec<ClaimRecord> {
    let without_completion = claims
        .trim()
        .strip_suffix(COMPLETION_DELIMITER)
        .unwrap_or(claims);
    without_completion
        .split(RECORD_DELIMITER)
        .filter_map(|claim| {
            let claim = trim_claim_record(claim);
            if claim.is_empty() || claim == COMPLETION_DELIMITER {
                return None;
            }
            let fields = claim.split(TUPLE_DELIMITER).collect::<Vec<_>>();
            Some(ClaimRecord {
                subject_id: pull_field(&fields, 0),
                object_id: pull_field(&fields, 1),
                claim_type: pull_field(&fields, 2),
                status: pull_field(&fields, 3),
                start_date: pull_field(&fields, 4),
                end_date: pull_field(&fields, 5),
                description: pull_field(&fields, 6),
                source_text: pull_field(&fields, 7),
            })
        })
        .collect()
}

/// Parse a community report JSON response.
///
/// # Errors
///
/// Returns an error when strict and repaired JSON parsing both fail or when the
/// parsed value is not compatible with [`CommunityReport`].
pub fn parse_community_report(input: &str) -> Result<CommunityReport> {
    let (_, value) = try_parse_json_object(input)?;
    serde_json::from_value(value).map_err(|source| LlmError::Parse {
        kind: "community report",
        message: source.to_string(),
    })
}

/// Try to parse a GraphRAG JSON object response with limited deterministic cleanup.
///
/// # Errors
///
/// Returns an error when cleanup cannot produce a JSON object.
pub fn try_parse_json_object(input: &str) -> Result<(String, Value)> {
    if let Ok(value) = serde_json::from_str::<Value>(input)
        && value.is_object()
    {
        return Ok((input.to_owned(), value));
    }

    let mut cleaned = extract_json_object(input).unwrap_or_else(|| input.to_owned());
    cleaned = cleaned
        .replace("{{", "{")
        .replace("}}", "}")
        .replace("\"[{", "[{")
        .replace("}]\"", "}]")
        .replace('\\', " ")
        .replace("\\n", " ")
        .replace('\n', " ")
        .replace('\r', "")
        .trim()
        .to_owned();

    if let Some(stripped) = cleaned.strip_prefix("```json") {
        cleaned = stripped.to_owned();
    }
    if let Some(stripped) = cleaned.strip_suffix("```") {
        cleaned = stripped.to_owned();
    }
    cleaned = cleaned.trim().to_owned();

    let value = serde_json::from_str::<Value>(&cleaned).map_err(|source| LlmError::Parse {
        kind: "json object",
        message: source.to_string(),
    })?;
    if !value.is_object() {
        return Err(LlmError::Parse {
            kind: "json object",
            message: "parsed value is not an object".to_owned(),
        });
    }
    Ok((cleaned, value))
}

/// Extract the first balanced JSON object-like span from text.
#[must_use]
pub fn extract_json_object(input: &str) -> Option<String> {
    let start = input.find('{')?;
    let end = input.rfind('}')?;
    (end >= start).then(|| input[start..=end].to_owned())
}

fn pull_field(fields: &[&str], index: usize) -> Option<String> {
    fields.get(index).map(|field| field.trim().to_owned())
}

fn trim_tuple_record(record: &str) -> String {
    let record = record.trim();
    record
        .strip_prefix('(')
        .unwrap_or(record)
        .strip_suffix(')')
        .unwrap_or_else(|| record.strip_prefix('(').unwrap_or(record))
        .trim()
        .to_owned()
}

fn trim_claim_record(record: &str) -> String {
    let record = record.trim();
    record
        .strip_prefix('(')
        .unwrap_or(record)
        .strip_suffix(')')
        .unwrap_or_else(|| record.strip_prefix('(').unwrap_or(record))
        .trim()
        .to_owned()
}

fn clean_str(value: &str) -> String {
    html_escape::decode_html_entities(value.trim())
        .chars()
        .filter(|character| {
            let code = u32::from(*character);
            !(code <= 0x1f || (0x7f..=0x9f).contains(&code))
        })
        .collect()
}
