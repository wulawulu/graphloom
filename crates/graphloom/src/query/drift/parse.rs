//! Structured response parsing for DRIFT primer and local actions.

use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

use super::action::DriftActionResponse;
use crate::query::{QueryError, Result, SearchMethod};

#[derive(Debug, Clone, Deserialize)]
pub(super) struct PrimerResponse {
    pub(super) intermediate_answer: String,
    pub(super) score: i64,
    pub(super) follow_up_queries: Vec<String>,
}

pub(super) fn parse_primer(input: &str) -> Result<PrimerResponse> {
    let value = parse_json_value(input).ok_or_else(|| QueryError::QueryParse {
        method: SearchMethod::Drift,
        operation: "parse DRIFT primer response",
        message: "response does not contain a valid JSON object".to_owned(),
    })?;
    serde_json::from_value(value).map_err(|source| QueryError::QueryParse {
        method: SearchMethod::Drift,
        operation: "parse DRIFT primer response",
        message: source.to_string(),
    })
}

pub(super) fn parse_action(input: &str) -> Result<DriftActionResponse> {
    let Some(value) = parse_json_value(input) else {
        tracing::warn!(method = %SearchMethod::Drift, "unable to parse DRIFT action response");
        return Ok(DriftActionResponse::fallback());
    };
    let object = value.as_object().ok_or_else(|| QueryError::QueryParse {
        method: SearchMethod::Drift,
        operation: "parse DRIFT action response",
        message: "DRIFT action response must be a JSON object".to_owned(),
    })?;
    let answer = match object.get("response") {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Null) | None => {
            tracing::warn!(method = %SearchMethod::Drift, "DRIFT action response has no answer");
            None
        }
        Some(_) => {
            return Err(QueryError::QueryParse {
                method: SearchMethod::Drift,
                operation: "parse DRIFT action response",
                message: "response must be a string or null".to_owned(),
            });
        }
    };
    let score = match object.get("score") {
        None | Some(Value::Null) => f64::NEG_INFINITY,
        Some(Value::Number(value)) => value
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or_else(|| QueryError::QueryParse {
                method: SearchMethod::Drift,
                operation: "parse DRIFT action score",
                message: "score must be a finite number".to_owned(),
            })?,
        Some(Value::String(value)) => value.parse::<f64>().map_err(|_| QueryError::QueryParse {
            method: SearchMethod::Drift,
            operation: "parse DRIFT action score",
            message: format!("score {value:?} is not numeric"),
        })?,
        Some(_) => {
            return Err(QueryError::QueryParse {
                method: SearchMethod::Drift,
                operation: "parse DRIFT action score",
                message: "score must be numeric or a numeric string".to_owned(),
            });
        }
    };
    let follow_up_queries = match object.get("follow_up_queries") {
        None | Some(Value::Null) => {
            tracing::warn!(
                method = %SearchMethod::Drift,
                "DRIFT action response has no follow-up queries"
            );
            Vec::new()
        }
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| QueryError::QueryParse {
                        method: SearchMethod::Drift,
                        operation: "parse DRIFT action follow-up queries",
                        message: "follow_up_queries must contain only strings".to_owned(),
                    })
            })
            .collect::<Result<Vec<_>>>()?,
        Some(_) => {
            return Err(QueryError::QueryParse {
                method: SearchMethod::Drift,
                operation: "parse DRIFT action follow-up queries",
                message: "follow_up_queries must be an array".to_owned(),
            });
        }
    };
    Ok(DriftActionResponse {
        answer,
        score,
        follow_up_queries,
    })
}

fn parse_json_value(input: &str) -> Option<Value> {
    json_candidates(input).find_map(|candidate| {
        serde_json::from_str(candidate)
            .ok()
            .or_else(|| repair_json(candidate).and_then(|value| serde_json::from_str(&value).ok()))
    })
}

fn repair_json(input: &str) -> Option<String> {
    let bare_key = Regex::new(r"([{,]\s*)([A-Za-z_][A-Za-z0-9_]*)(\s*:)").ok()?;
    let trailing_comma = Regex::new(r",(\s*[}\]])").ok()?;
    let quoted = bare_key.replace_all(input, "$1\"$2\"$3");
    let without_trailing = trailing_comma.replace_all(&quoted, "$1");
    Some(repair_single_quoted_strings(&without_trailing))
}

fn repair_single_quoted_strings(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for character in input.chars() {
        if escaped {
            if in_single && character == '\'' {
                output.push('\'');
            } else {
                output.push('\\');
                output.push(character);
            }
            escaped = false;
            continue;
        }
        if character == '\\' && (in_double || in_single) {
            escaped = true;
            continue;
        }
        if in_double {
            output.push(character);
            if character == '"' {
                in_double = false;
            }
        } else if in_single {
            match character {
                '\'' => {
                    output.push('"');
                    in_single = false;
                }
                '"' => output.push_str("\\\""),
                value => output.push(value),
            }
        } else {
            match character {
                '"' => {
                    output.push(character);
                    in_double = true;
                }
                '\'' => {
                    output.push('"');
                    in_single = true;
                }
                value => output.push(value),
            }
        }
    }
    if escaped {
        output.push('\\');
    }
    output
}

fn json_candidates(input: &str) -> impl Iterator<Item = &str> {
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
                    b'{' => depth = depth.saturating_add(1),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_parse_plain_fenced_prose_and_repaired_action_json() {
        for input in [
            r#"{"response":"answer","score":80,"follow_up_queries":["next"]}"#,
            r#"```json
{"response":"answer","score":"80","follow_up_queries":["next"]}
```"#,
            "prefix {response:\"answer\",score:80,follow_up_queries:[\"next\"],} suffix",
            "prefix {'response':'answer','score':'80','follow_up_queries':['next']} suffix",
        ] {
            let parsed = parse_action(input).expect("action should parse");
            assert_eq!(parsed.answer.as_deref(), Some("answer"));
            assert_eq!(parsed.score, 80.0);
            assert_eq!(parsed.follow_up_queries, ["next"]);
        }
    }

    #[test]
    fn test_should_return_action_fallback_only_for_unparsable_json() {
        let parsed = parse_action("not json").expect("fallback should be successful");

        assert!(parsed.answer.is_none());
        assert_eq!(parsed.score, f64::NEG_INFINITY);
        assert!(parsed.follow_up_queries.is_empty());
    }

    #[test]
    fn test_should_reject_invalid_present_action_fields() {
        for input in [
            r#"{"response":3,"score":80,"follow_up_queries":[]}"#,
            r#"{"response":"ok","score":[],"follow_up_queries":[]}"#,
            r#"{"response":"ok","score":80,"follow_up_queries":[3]}"#,
        ] {
            assert!(matches!(
                parse_action(input),
                Err(QueryError::QueryParse {
                    method: SearchMethod::Drift,
                    ..
                })
            ));
        }
    }

    #[test]
    fn test_should_accept_python_float_nonfinite_score_strings() {
        for (input, expected) in [
            (
                r#"{"response":"answer","score":"-inf","follow_up_queries":[]}"#,
                f64::NEG_INFINITY,
            ),
            (
                r#"{"response":"answer","score":"inf","follow_up_queries":[]}"#,
                f64::INFINITY,
            ),
        ] {
            let parsed = parse_action(input).expect("Python float string");
            assert_eq!(parsed.score, expected);
        }
        let parsed = parse_action(r#"{"response":"answer","score":"nan","follow_up_queries":[]}"#)
            .expect("Python NaN string");
        assert!(parsed.score.is_nan());
    }

    #[test]
    fn test_should_require_all_primer_fields() {
        assert!(parse_primer(r#"{"intermediate_answer":"a","score":1}"#).is_err());
        let parsed =
            parse_primer(r#"{"intermediate_answer":"a","score":1,"follow_up_queries":["q"]}"#)
                .expect("primer");
        assert_eq!(parsed.score, 1);
    }
}
