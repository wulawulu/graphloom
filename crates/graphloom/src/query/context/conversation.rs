//! GraphRAG-compatible conversation history.

use std::sync::Arc;

use graphloom_llm::Tokenizer;
use serde::{Deserialize, Serialize};

use super::ContextTable;
use crate::query::{QueryError, Result, SearchMethod};

const MAX_CONVERSATION_TURNS: usize = 1_024;
const MAX_CONVERSATION_CONTENT_BYTES: usize = 65_536;

/// Role of one conversation turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ConversationRole {
    /// System-authored context.
    System,
    /// User question.
    User,
    /// Assistant answer.
    Assistant,
}

impl ConversationRole {
    const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

/// One conversation turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ConversationTurn {
    /// Turn author.
    pub role: ConversationRole,
    /// Turn text.
    pub content: String,
}

/// Conversation history supplied to Local Search.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ConversationHistory {
    /// Turns in original chronological input order.
    pub turns: Vec<ConversationTurn>,
}

#[derive(Debug)]
pub(crate) struct ConversationContext {
    pub(crate) text: String,
    pub(crate) table: ContextTable,
}

impl ConversationHistory {
    pub(crate) fn validate(&self) -> std::result::Result<(), String> {
        if self.turns.len() > MAX_CONVERSATION_TURNS {
            return Err(format!(
                "conversation_history has {} turns; maximum is {MAX_CONVERSATION_TURNS}",
                self.turns.len()
            ));
        }
        if let Some((index, turn)) = self
            .turns
            .iter()
            .enumerate()
            .find(|(_, turn)| turn.content.len() > MAX_CONVERSATION_CONTENT_BYTES)
        {
            return Err(format!(
                "conversation_history turn {index} has {} content bytes; maximum is \
                 {MAX_CONVERSATION_CONTENT_BYTES}",
                turn.content.len()
            ));
        }
        Ok(())
    }

    /// Return recent user questions in GraphRAG mapping order.
    ///
    /// GraphRAG scans the input backwards, so the newest selected question is
    /// first. A zero maximum has the upstream meaning of no limit.
    #[must_use]
    pub fn recent_user_questions(&self, max_turns: usize) -> Vec<&str> {
        self.turns
            .iter()
            .rev()
            .filter(|turn| turn.role == ConversationRole::User)
            .map(|turn| turn.content.as_str())
            .take(if max_turns == 0 {
                usize::MAX
            } else {
                max_turns
            })
            .collect()
    }

    pub(crate) fn mapping_query(&self, query: &str, max_turns: usize) -> String {
        format!(
            "{query}\n{}",
            self.recent_user_questions(max_turns).join("\n")
        )
    }

    pub(crate) fn build_user_context(
        &self,
        tokenizer: &Arc<dyn Tokenizer>,
        max_turns: usize,
        max_tokens: usize,
    ) -> Result<ConversationContext> {
        let mut table = ContextTable::new(["turn", "content"], Vec::new());
        if !self
            .turns
            .iter()
            .any(|turn| turn.role == ConversationRole::User)
        {
            return Ok(ConversationContext {
                text: String::new(),
                table,
            });
        }
        let limit = if max_turns == 0 {
            usize::MAX
        } else {
            max_turns
        };
        for turn in self
            .turns
            .iter()
            .filter(|turn| turn.role == ConversationRole::User)
            .take(limit)
        {
            let row = vec![
                ConversationRole::User.as_str().to_owned(),
                turn.content.clone(),
            ];
            let mut trial = table.clone();
            trial.push(row);
            let trial_text = trial.render_csv_section(
                "Conversation History",
                SearchMethod::Local,
                "render conversation history candidate",
            )?;
            if count(
                tokenizer,
                &trial_text,
                "count conversation history candidate",
            )? > max_tokens
            {
                break;
            }
            table = trial;
        }
        let text = if table.is_empty() {
            "-----Conversation History-----\n\n".to_owned()
        } else {
            table.render_csv_section(
                "Conversation History",
                SearchMethod::Local,
                "render conversation history",
            )?
        };
        Ok(ConversationContext { text, table })
    }
}

fn count(tokenizer: &Arc<dyn Tokenizer>, text: &str, operation: &'static str) -> Result<usize> {
    tokenizer
        .count(text)
        .map_err(|source| QueryError::QueryContext {
            method: SearchMethod::Local,
            operation,
            message: source.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_reject_unbounded_history_collection_and_content() {
        let oversized_collection = ConversationHistory {
            turns: (0..=MAX_CONVERSATION_TURNS)
                .map(|_| ConversationTurn {
                    role: ConversationRole::User,
                    content: "question".to_owned(),
                })
                .collect(),
        };
        assert!(
            oversized_collection
                .validate()
                .expect_err("too many turns")
                .contains("maximum")
        );

        let oversized_content = ConversationHistory {
            turns: vec![ConversationTurn {
                role: ConversationRole::User,
                content: "q".repeat(MAX_CONVERSATION_CONTENT_BYTES + 1),
            }],
        };
        assert!(
            oversized_content
                .validate()
                .expect_err("turn too long")
                .contains("content bytes")
        );
    }
}
