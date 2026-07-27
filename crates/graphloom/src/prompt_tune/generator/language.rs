//! Language detection for GraphRAG prompts.

use std::sync::Arc;

use graphloom_llm::{ChatMessage, CompletionModel, CompletionRequest};

use super::meta_prompts::DETECT_LANGUAGE_PROMPT;
use crate::{GraphLoomError, Result};

/// Detect the primary language of the input documents.
pub(crate) async fn detect_language(
    model: &Arc<dyn CompletionModel>,
    docs: &[String],
    consumer: &'static str,
) -> Result<String> {
    let docs_str = docs.join(" ");
    let prompt = DETECT_LANGUAGE_PROMPT.replace("{input_text}", &docs_str);

    let request = CompletionRequest::new(vec![ChatMessage::user(prompt)]);

    let response = model.complete(request).await.map_err(GraphLoomError::Llm)?;
    let content = response.content().map_err(GraphLoomError::Llm)?;
    let trimmed = content.trim().to_owned();

    if trimmed.is_empty() {
        return Err(GraphLoomError::InvalidData {
            workflow: consumer,
            message: "language detection returned empty content".to_owned(),
        });
    }
    Ok(trimmed)
}
