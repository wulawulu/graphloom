//! Persona generation for GraphRAG prompts.

use std::sync::Arc;

use graphloom_llm::{ChatMessage, CompletionModel, CompletionRequest};

use super::meta_prompts::{DEFAULT_TASK, GENERATE_PERSONA_PROMPT};
use crate::{GraphLoomError, Result};

pub(crate) async fn generate_persona(
    model: &Arc<dyn CompletionModel>,
    domain: &str,
    consumer: &'static str,
) -> Result<String> {
    let formatted_task = DEFAULT_TASK.replace("{domain}", domain);
    let prompt = GENERATE_PERSONA_PROMPT.replace("{sample_task}", &formatted_task);

    let request = CompletionRequest::new(vec![ChatMessage::user(prompt)]);

    let response = model.complete(request).await.map_err(GraphLoomError::Llm)?;
    let content = response.content().map_err(GraphLoomError::Llm)?;
    if content.trim().is_empty() {
        return Err(GraphLoomError::InvalidData {
            workflow: consumer,
            message: "persona generation returned empty content".to_owned(),
        });
    }
    Ok(content.to_owned())
}
