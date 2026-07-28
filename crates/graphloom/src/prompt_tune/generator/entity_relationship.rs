//! Entity relationship example generation for GraphRAG prompt tuning.

use std::sync::Arc;

use graphloom_llm::{ChatMessage, CompletionModel, CompletionRequest};

use super::meta_prompts::{
    ENTITY_RELATIONSHIPS_GENERATION_PROMPT, MAX_EXAMPLES,
    UNTYPED_ENTITY_RELATIONSHIPS_GENERATION_PROMPT,
};
use crate::{GraphLoomError, Result};

/// Generate entity/relationship examples from document chunks.
///
/// GraphRAG 3.1.0 uses `asyncio.gather` for concurrent requests. We replicate
/// this by spawning tasks and collecting results in input order.
pub(crate) async fn generate_entity_relationship_examples(
    model: &Arc<dyn CompletionModel>,
    persona: &str,
    entity_types: Option<&[String]>,
    docs: &[String],
    language: &str,
    consumer: &'static str,
) -> Result<Vec<String>> {
    let docs_slice: &[String] = if docs.len() > MAX_EXAMPLES {
        &docs[..MAX_EXAMPLES]
    } else {
        docs
    };

    let messages: Vec<String> = if let Some(types) = entity_types {
        let entity_types_str = types.join(", ");
        docs_slice
            .iter()
            .map(|doc| {
                ENTITY_RELATIONSHIPS_GENERATION_PROMPT
                    .replace("{entity_types}", &entity_types_str)
                    .replace("{input_text}", doc)
                    .replace("{language}", language)
            })
            .collect()
    } else {
        docs_slice
            .iter()
            .map(|doc| {
                UNTYPED_ENTITY_RELATIONSHIPS_GENERATION_PROMPT
                    .replace("{input_text}", doc)
                    .replace("{language}", language)
            })
            .collect()
    };

    // GraphRAG 3.1.0 reuses one CompletionMessagesBuilder while constructing
    // async calls. Because the coroutine bodies begin under asyncio.gather,
    // every call observes the final accumulated message list. Preserve that
    // request contract while collecting responses in producer order.
    let model = Arc::clone(model);
    let mut request_messages = Vec::with_capacity(messages.len() + 1);
    request_messages.push(ChatMessage::system(persona));
    request_messages.extend(messages.into_iter().map(ChatMessage::user));
    let request = CompletionRequest::new(request_messages);
    let handles: Vec<_> = (0..docs_slice.len())
        .map(|i| {
            let model = Arc::clone(&model);
            let request = request.clone();
            tokio::spawn(async move {
                let response = model.complete(request).await.map_err(GraphLoomError::Llm)?;
                let content = response.content().map_err(GraphLoomError::Llm)?;
                Ok::<_, GraphLoomError>((i, content.to_owned()))
            })
        })
        .collect();

    let total = handles.len();
    let mut results = vec![String::new(); total];
    for handle in handles {
        let (idx, content) = handle.await.map_err(|_join| GraphLoomError::InvalidData {
            workflow: consumer,
            message: "entity relationship example task panicked".to_owned(),
        })??;
        results[idx] = content;
    }

    Ok(results)
}
