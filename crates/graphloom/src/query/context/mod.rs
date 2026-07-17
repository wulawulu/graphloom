//! Shared context-table and conversation-history support.

mod conversation;
mod csv;

pub use conversation::{ConversationHistory, ConversationRole, ConversationTurn};
pub(crate) use csv::ContextTable;
