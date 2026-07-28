//! Community report rating description generation.

use std::sync::Arc;

use graphloom_llm::{ChatMessage, CompletionModel, CompletionRequest};

use super::meta_prompts::GENERATE_REPORT_RATING_PROMPT;
use crate::{GraphLoomError, Result};

pub(crate) async fn generate_community_report_rating(
    model: &Arc<dyn CompletionModel>,
    domain: &str,
    persona: &str,
    docs: &[String],
    _consumer: &'static str,
) -> Result<String> {
    let docs_str = docs.join(" ");
    let prompt = GENERATE_REPORT_RATING_PROMPT
        .replace("{domain}", domain)
        .replace("{persona}", persona)
        .replace("{input_text}", &docs_str);

    let request = CompletionRequest::new(vec![ChatMessage::user(prompt)]);

    let response = model.complete(request).await.map_err(GraphLoomError::Llm)?;
    let content = response.content().map_err(GraphLoomError::Llm)?;
    Ok(content.to_owned())
}
