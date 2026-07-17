//! Robust Global map JSON extraction and typed point parsing.

use serde_json::Value;

use crate::query::QueryUsageCategory;

/// One relevant point emitted by a Global map analyst.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MapPoint {
    /// Point description.
    pub answer: String,
    /// GraphRAG importance score.
    pub score: i64,
}

/// Typed result of one Global map batch.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MapSearchResult {
    /// Original context batch index.
    pub batch_index: usize,
    /// Unmodified provider response.
    pub raw_response: String,
    /// Parsed points, or the GraphRAG score-zero fallback.
    pub points: Vec<MapPoint>,
    /// Exact map context supplied for this analyst.
    pub context: String,
    /// Usage attributable to this map call.
    pub usage: QueryUsageCategory,
}

pub(super) fn parse_map_points(input: &str) -> Vec<MapPoint> {
    let Some(object) = first_json_object(input) else {
        return fallback();
    };
    let Ok(value) = serde_json::from_str::<Value>(object) else {
        return fallback();
    };
    let Some(points) = value.get("points").and_then(Value::as_array) else {
        return fallback();
    };
    if points.is_empty() {
        return fallback();
    }
    points
        .iter()
        .filter_map(|point| {
            let answer = point.get("description")?.as_str()?.to_owned();
            let score = python_int(point.get("score")?)?;
            Some(MapPoint { answer, score })
        })
        .collect()
}

pub(super) fn first_json_object(input: &str) -> Option<&str> {
    json_object_candidates(input).find(|object| serde_json::from_str::<Value>(object).is_ok())
}

pub(super) fn first_balanced_json_object(input: &str) -> Option<&str> {
    json_object_candidates(input).next()
}

fn json_object_candidates(input: &str) -> impl Iterator<Item = &str> {
    let bytes = input.as_bytes();
    bytes
        .iter()
        .enumerate()
        .filter_map(|(index, byte)| (*byte == b'{').then_some(index))
        .filter_map(move |start| {
            let mut depth = 0_usize;
            let mut in_string = false;
            let mut escaped = false;
            for (offset, byte) in bytes[start..].iter().copied().enumerate() {
                if in_string {
                    if escaped {
                        escaped = false;
                    } else if byte == b'\\' {
                        escaped = true;
                    } else if byte == b'"' {
                        in_string = false;
                    }
                    continue;
                }
                match byte {
                    b'"' => in_string = true,
                    b'{' => depth += 1,
                    b'}' => {
                        depth = depth.checked_sub(1)?;
                        if depth == 0 {
                            return input.get(start..=start + offset);
                        }
                    }
                    _ => {}
                }
            }
            None
        })
}

pub(super) fn python_int(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(value) => Some(i64::from(*value)),
        Value::Number(value) => value.as_i64().or_else(|| {
            value.as_f64().and_then(|value| {
                let truncated = value.trunc();
                (truncated.is_finite()
                    && truncated >= i64::MIN as f64
                    && truncated < -(i64::MIN as f64))
                    .then_some(truncated as i64)
            })
        }),
        Value::String(value) => value.trim().parse().ok(),
        _ => None,
    }
}

fn fallback() -> Vec<MapPoint> {
    vec![MapPoint {
        answer: String::new(),
        score: 0,
    }]
}

#[cfg(test)]
mod tests {
    use super::{MapPoint, first_json_object, parse_map_points};

    #[test]
    fn test_should_parse_plain_fenced_and_prose_json() {
        for input in [
            r#"{"points":[{"description":"A","score":2}]}"#,
            "```json\n{\"points\":[{\"description\":\"A\",\"score\":\"2\"}]}\n```",
            "Before {\"points\":[{\"description\":\"A\",\"score\":2.9}]} after",
        ] {
            assert_eq!(
                parse_map_points(input),
                vec![MapPoint {
                    answer: "A".to_owned(),
                    score: 2,
                }]
            );
        }
    }

    #[test]
    fn test_should_scan_nested_objects_braces_and_escaped_quotes() {
        let input = r#"prefix {"meta":{"text":"a } { and \"quoted\""},"points":[{"description":"B","score":1}]} suffix"#;
        assert_eq!(
            first_json_object(input),
            Some(
                r#"{"meta":{"text":"a } { and \"quoted\""},"points":[{"description":"B","score":1}]}"#
            )
        );
        assert_eq!(parse_map_points(input)[0].answer, "B");
    }

    #[test]
    fn test_should_fallback_for_missing_wrong_or_malformed_points() {
        for input in [
            "{}",
            r#"{"points":null}"#,
            r#"{"points":{}}"#,
            r#"{"points":[]}"#,
            "not json",
        ] {
            assert_eq!(
                parse_map_points(input),
                vec![MapPoint {
                    answer: String::new(),
                    score: 0,
                }]
            );
        }
    }

    #[test]
    fn test_should_skip_invalid_balanced_braces_before_first_valid_object() {
        assert_eq!(
            parse_map_points(
                r#"note {not-json} then {"points":[{"description":"valid","score":4}]}"#
            ),
            vec![MapPoint {
                answer: "valid".to_owned(),
                score: 4,
            }]
        );
    }

    #[test]
    fn test_should_skip_invalid_elements_and_apply_python_int_semantics() {
        assert_eq!(
            parse_map_points(
                r#"{"points":[
                    {"description":"integer","score":"-2"},
                    {"description":"float","score":3.9},
                    {"description":"bool","score":true},
                    {"description":"bad","score":"2.5"},
                    {"score":4},
                    {"description":5,"score":4}
                ]}"#
            ),
            vec![
                MapPoint {
                    answer: "integer".to_owned(),
                    score: -2,
                },
                MapPoint {
                    answer: "float".to_owned(),
                    score: 3,
                },
                MapPoint {
                    answer: "bool".to_owned(),
                    score: 1,
                },
            ]
        );
    }
}
