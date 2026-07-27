//! Domain generation for GraphRAG prompts.

use std::sync::Arc;

use graphloom_llm::{ChatMessage, CompletionModel, CompletionRequest};

use super::meta_prompts::GENERATE_DOMAIN_PROMPT;
use crate::{GraphLoomError, Result};

/// Generate a domain description from document chunks.
///
/// The `consumer` parameter establishes a stable cache operation identity
/// (e.g. `"prompt_tune.generate_domain"`).
pub(crate) async fn generate_domain(
    model: &Arc<dyn CompletionModel>,
    docs: &[String],
    consumer: &'static str,
) -> Result<String> {
    let docs_str = docs.join(" ");
    let prompt = GENERATE_DOMAIN_PROMPT.replace("{input_text}", &docs_str);

    let request = CompletionRequest::new(vec![ChatMessage::user(prompt)]);

    let response = model.complete(request).await.map_err(GraphLoomError::Llm)?;

    let content = response.content().map_err(GraphLoomError::Llm)?;

    if content.trim().is_empty() {
        return Err(GraphLoomError::InvalidData {
            workflow: consumer,
            message: "domain generation returned empty content".to_owned(),
        });
    }
    Ok(content.to_owned())
}
