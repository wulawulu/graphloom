//! Stable query-identity action graph used by DRIFT.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::action::{DriftAction, DriftActionMetadata, DriftActionResponse};
use crate::query::{QueryError, Result, SearchMethod};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct DriftEdge {
    pub(super) source: usize,
    pub(super) target: usize,
    pub(super) weight: f64,
}

#[derive(Debug, Clone, Default)]
pub(super) struct DriftQueryState {
    nodes: Vec<DriftAction>,
    ids_by_query: BTreeMap<String, usize>,
    edges: Vec<DriftEdge>,
}

impl DriftQueryState {
    pub(super) fn add_root(
        &mut self,
        query: String,
        answer: String,
        score: f64,
        followups: &[String],
    ) {
        let root = self.add_or_get(query);
        if let Some(action) = self.nodes.get_mut(root) {
            action.answer = Some(answer);
            action.score = score;
        }
        for followup in followups {
            let target = self.add_or_get(followup.clone());
            self.edges.push(DriftEdge {
                source: root,
                target,
                weight: 1.0,
            });
        }
    }

    pub(super) fn incomplete_ids(&self) -> Vec<usize> {
        self.nodes
            .iter()
            .filter(|action| action.answer.is_none())
            .map(|action| action.id)
            .collect()
    }

    pub(super) fn query(&self, id: usize) -> Option<&str> {
        self.nodes.get(id).map(|action| action.query.as_str())
    }

    pub(super) fn apply(
        &mut self,
        id: usize,
        response: DriftActionResponse,
        metadata: DriftActionMetadata,
    ) -> Result<()> {
        let action = self
            .nodes
            .get_mut(id)
            .ok_or_else(|| QueryError::QueryContext {
                method: SearchMethod::Drift,
                operation: "update DRIFT action state",
                message: format!("action id {id} is absent"),
            })?;
        action.answer = response.answer;
        action.score = response.score;
        action.metadata = metadata;
        for followup in response.follow_up_queries {
            let target = self.add_or_get(followup);
            self.edges.push(DriftEdge {
                source: id,
                target,
                weight: 1.0,
            });
        }
        Ok(())
    }

    pub(super) fn reduce_answers(&self) -> Vec<&str> {
        self.nodes
            .iter()
            .filter_map(|action| action.answer.as_deref())
            .filter(|answer| !answer.is_empty())
            .collect()
    }

    pub(super) fn nodes(&self) -> &[DriftAction] {
        &self.nodes
    }

    pub(super) fn edges_in_graph_order(&self) -> Vec<&DriftEdge> {
        let mut edges = self.edges.iter().enumerate().collect::<Vec<_>>();
        edges.sort_by_key(|(position, edge)| (edge.source, *position));
        edges.into_iter().map(|(_, edge)| edge).collect()
    }

    pub(super) fn to_json(&self) -> Result<String> {
        serde_json::to_string(&json!({
            "nodes": self.nodes.iter().map(|node| json!({
                "id": node.id,
                "query": node.query,
                "answer": node.answer,
                "score": finite_json(node.score),
                "metadata": {
                    "llm_calls": node.metadata.usage.llm_calls,
                    "prompt_tokens": node.metadata.usage.prompt_tokens,
                    "output_tokens": node.metadata.usage.output_tokens,
                },
            })).collect::<Vec<_>>(),
            "edges": self.edges_in_graph_order().into_iter().map(|edge| json!({
                "source": edge.source,
                "target": edge.target,
                "weight": edge.weight,
            })).collect::<Vec<_>>(),
        }))
        .map_err(|source| QueryError::QueryParse {
            method: SearchMethod::Drift,
            operation: "serialize DRIFT action state",
            message: source.to_string(),
        })
    }

    fn add_or_get(&mut self, query: String) -> usize {
        if let Some(id) = self.ids_by_query.get(&query) {
            return *id;
        }
        let id = self.nodes.len();
        self.ids_by_query.insert(query.clone(), id);
        self.nodes.push(DriftAction::incomplete(id, query));
        id
    }
}

fn finite_json(value: f64) -> Value {
    if value.is_finite() {
        json!(value)
    } else {
        Value::Null
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_preserve_query_identity_and_multiple_edges() {
        let mut state = DriftQueryState::default();
        state.add_root(
            "root".to_owned(),
            "answer".to_owned(),
            50.0,
            &["same".to_owned(), "same".to_owned()],
        );

        assert_eq!(state.nodes.len(), 2);
        assert_eq!(state.edges.len(), 2);
        assert_eq!(state.edges[0].target, state.edges[1].target);
    }

    #[test]
    fn test_should_serialize_negative_infinity_as_valid_json_null() {
        let mut state = DriftQueryState::default();
        state.add_root(
            "root".to_owned(),
            "answer".to_owned(),
            10.0,
            &["incomplete".to_owned()],
        );
        let value: Value =
            serde_json::from_str(&state.to_json().expect("state JSON")).expect("valid JSON");

        assert!(value["nodes"][1]["score"].is_null());
    }

    #[test]
    fn test_should_treat_empty_answer_as_completed_but_exclude_it_from_reduce() {
        let mut state = DriftQueryState::default();
        state.add_root(
            "root".to_owned(),
            String::new(),
            0.0,
            &["followup".to_owned()],
        );
        state
            .apply(
                1,
                DriftActionResponse {
                    answer: Some(String::new()),
                    score: 0.0,
                    follow_up_queries: Vec::new(),
                },
                DriftActionMetadata::default(),
            )
            .expect("apply empty completed answer");

        assert!(state.incomplete_ids().is_empty());
        assert!(state.reduce_answers().is_empty());
        let value: Value =
            serde_json::from_str(&state.to_json().expect("state JSON")).expect("valid JSON");
        assert_eq!(value["nodes"][0]["answer"], "");
        assert_eq!(value["nodes"][1]["answer"], "");
    }

    #[test]
    fn test_should_reduce_truthy_answers_in_insertion_order() {
        let mut state = DriftQueryState::default();
        let none = state.add_or_get("none".to_owned());
        let empty = state.add_or_get("empty".to_owned());
        let spaces = state.add_or_get("spaces".to_owned());
        let answer = state.add_or_get("answer".to_owned());
        for (id, value) in [
            (empty, String::new()),
            (spaces, "   ".to_owned()),
            (answer, "real answer".to_owned()),
        ] {
            state
                .apply(
                    id,
                    DriftActionResponse {
                        answer: Some(value),
                        score: 0.0,
                        follow_up_queries: Vec::new(),
                    },
                    DriftActionMetadata::default(),
                )
                .expect("apply completed answer");
        }

        assert_eq!(state.incomplete_ids(), [none]);
        assert_eq!(state.reduce_answers(), ["   ", "real answer"]);
    }

    #[test]
    fn test_should_serialize_edges_in_networkx_source_node_order() {
        let mut state = DriftQueryState::default();
        state.add_root(
            "root".to_owned(),
            "answer".to_owned(),
            90.0,
            &["first".to_owned(), "second".to_owned()],
        );
        state
            .apply(
                2,
                DriftActionResponse {
                    answer: Some("second answer".to_owned()),
                    score: 80.0,
                    follow_up_queries: vec!["second child".to_owned()],
                },
                DriftActionMetadata::default(),
            )
            .expect("apply second node first");
        state
            .apply(
                1,
                DriftActionResponse {
                    answer: Some("first answer".to_owned()),
                    score: 80.0,
                    follow_up_queries: vec!["first child".to_owned()],
                },
                DriftActionMetadata::default(),
            )
            .expect("apply first node second");

        let value: Value =
            serde_json::from_str(&state.to_json().expect("state JSON")).expect("valid JSON");

        assert_eq!(
            value["edges"],
            json!([
                {"source": 0, "target": 1, "weight": 1.0},
                {"source": 0, "target": 2, "weight": 1.0},
                {"source": 1, "target": 4, "weight": 1.0},
                {"source": 2, "target": 3, "weight": 1.0},
            ])
        );
    }
}
