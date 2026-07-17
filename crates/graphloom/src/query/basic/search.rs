//! Basic Search completion orchestration.

use std::{collections::BTreeMap, sync::Arc, time::Instant};

use futures_util::{StreamExt, stream};
use graphloom_llm::{ChatMessage, CompletionRequest, CompletionStream, Tokenizer};
use serde::Serialize;

use super::super::{
    BasicQueryRuntime, QueryError, QueryEvent, QueryEventStream, QueryResult, QueryUsage,
    QueryUsageCategory, Result, SearchMethod,
};

#[derive(Debug, Serialize)]
struct BasicPromptContext<'a> {
    context_data: &'a str,
    response_type: &'a str,
}

struct BasicStreamState {
    provider: CompletionStream,
    context: super::super::QueryContext,
    response: String,
    started: Instant,
    prompt_tokens: usize,
    build_context_usage: QueryUsageCategory,
    tokenizer: Arc<dyn Tokenizer>,
    callbacks: Arc<dyn super::super::QueryCallbacks>,
    completion_model_id: String,
    phase: BasicStreamPhase,
}

#[derive(Debug, Clone, Copy)]
enum BasicStreamPhase {
    Context,
    Tokens,
    Completed,
}

pub(crate) async fn basic_search(
    runtime: BasicQueryRuntime,
    query: &str,
    response_type: &str,
) -> Result<QueryResult> {
    let mut events = basic_search_streaming(runtime, query, response_type).await?;
    while let Some(event) = events.next().await {
        if let QueryEvent::Completed(result) = event? {
            return Ok(result);
        }
    }
    Err(QueryError::QueryCompletion {
        method: SearchMethod::Basic,
        operation: "aggregate Basic Search stream",
        model: "unknown".to_owned(),
        source: Box::new(graphloom_llm::LlmError::InvalidResponse {
            model_instance: "unknown".to_owned(),
            operation: "query stream",
            message: "stream ended without a completed event".to_owned(),
        }),
    })
}

pub(crate) async fn basic_search_streaming(
    runtime: BasicQueryRuntime,
    query: &str,
    response_type: &str,
) -> Result<QueryEventStream> {
    let started = Instant::now();
    let built = runtime.basic_context.build(query).await?;
    let context_text = match &built.context.text {
        super::super::QueryContextText::Text(value) => value.as_str(),
        _ => {
            return Err(QueryError::QueryContext {
                method: SearchMethod::Basic,
                operation: "read Basic Search context text",
                message: "Basic Search requires one context string".to_owned(),
            });
        }
    };
    let rendered = runtime
        .prompt
        .bind(&BasicPromptContext {
            context_data: context_text,
            response_type,
        })
        .and_then(|prompt| prompt.render())
        .map_err(|source| QueryError::QueryPrompt {
            method: SearchMethod::Basic,
            operation: "render Basic Search prompt",
            prompt: "basic_search_system_prompt.txt",
            source: Box::new(source),
        })?;
    let prompt_tokens = runtime
        .basic_context
        .tokenizer
        .count(&rendered)
        .map_err(|source| QueryError::QueryContext {
            method: SearchMethod::Basic,
            operation: "count Basic Search prompt tokens",
            message: source.to_string(),
        })?;
    let mut request = CompletionRequest::new(vec![
        ChatMessage::system(rendered),
        ChatMessage::user(query),
    ]);
    request
        .apply_call_args(&runtime.completion_config.call_args)
        .and_then(|()| {
            request.stream = Some(true);
            request.validate()
        })
        .map_err(|source| QueryError::InvalidQueryConfig {
            method: SearchMethod::Basic,
            operation: "build Basic Search completion request",
            message: source.to_string(),
        })?;
    runtime.callbacks.on_context(&built.context);
    let provider = runtime
        .completion_model
        .stream(request)
        .await
        .map_err(|source| QueryError::QueryCompletion {
            method: SearchMethod::Basic,
            operation: "start Basic Search completion stream",
            model: runtime.completion_model_id.clone(),
            source: Box::new(source),
        })?;
    let state = BasicStreamState {
        provider,
        context: built.context,
        response: String::new(),
        started,
        prompt_tokens,
        build_context_usage: built.usage,
        tokenizer: Arc::clone(&runtime.basic_context.tokenizer),
        callbacks: runtime.callbacks,
        completion_model_id: runtime.completion_model_id,
        phase: BasicStreamPhase::Context,
    };
    Ok(Box::pin(stream::unfold(Some(state), |state| async move {
        next_event(state).await
    })))
}

async fn next_event(
    state: Option<BasicStreamState>,
) -> Option<(Result<QueryEvent>, Option<BasicStreamState>)> {
    let mut state = state?;
    match state.phase {
        BasicStreamPhase::Context => {
            state.phase = BasicStreamPhase::Tokens;
            Some((Ok(QueryEvent::Context(state.context.clone())), Some(state)))
        }
        BasicStreamPhase::Tokens => loop {
            match state.provider.next().await {
                Some(Ok(chunk)) => {
                    let content = chunk
                        .choices
                        .first()
                        .and_then(|choice| choice.delta.content.as_deref())
                        .unwrap_or_default();
                    if content.is_empty() {
                        continue;
                    }
                    state.response.push_str(content);
                    state.callbacks.on_llm_new_token(content);
                    return Some((Ok(QueryEvent::Token(content.to_owned())), Some(state)));
                }
                Some(Err(source)) => {
                    let error = QueryError::QueryCompletion {
                        method: SearchMethod::Basic,
                        operation: "consume Basic Search completion stream",
                        model: state.completion_model_id.clone(),
                        source: Box::new(source),
                    };
                    return Some((Err(error), None));
                }
                None => {
                    state.phase = BasicStreamPhase::Completed;
                    return Some(completed_event(state));
                }
            }
        },
        BasicStreamPhase::Completed => Some(completed_event(state)),
    }
}

fn completed_event(state: BasicStreamState) -> (Result<QueryEvent>, Option<BasicStreamState>) {
    let output_tokens = match state.tokenizer.count(&state.response) {
        Ok(value) => value,
        Err(source) => {
            return (
                Err(QueryError::QueryCompletion {
                    method: SearchMethod::Basic,
                    operation: "count Basic Search output tokens",
                    model: state.completion_model_id,
                    source: Box::new(source),
                }),
                None,
            );
        }
    };
    let usage = QueryUsage::from_categories(BTreeMap::from([
        ("build_context".to_owned(), state.build_context_usage),
        (
            "response".to_owned(),
            QueryUsageCategory {
                llm_calls: 1,
                prompt_tokens: state.prompt_tokens,
                output_tokens,
            },
        ),
    ]));
    let result = QueryResult {
        response: state.response,
        context: state.context,
        elapsed: state.started.elapsed(),
        usage,
    };
    (Ok(QueryEvent::Completed(result)), None)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use graphloom_llm::{
        CompletionChunk, CompletionModel, CompletionResponse, EmbeddingModel, MockEmbeddingModel,
        ModelConfig,
    };
    use graphloom_vectors::{
        LanceDbVectorStore, VectorDocument, VectorIndexSchema, VectorStore, VectorStoreConfig,
    };
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::{
        BasicSearchConfig,
        prompts::{PromptKind, PromptRepository},
        query::{
            BasicQueryRuntime, QueryCallbacks, QueryContext, QueryContextText, TextUnit,
            basic::BasicContextBuilder,
        },
    };

    #[derive(Debug, Default)]
    struct ByteTokenizer;

    impl Tokenizer for ByteTokenizer {
        fn encode(&self, text: &str) -> graphloom_llm::Result<Vec<u32>> {
            Ok(text
                .as_bytes()
                .iter()
                .map(|value| u32::from(*value))
                .collect())
        }

        fn decode(&self, tokens: &[u32]) -> graphloom_llm::Result<String> {
            let bytes = tokens
                .iter()
                .map(|value| u8::try_from(*value))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|source| graphloom_llm::LlmError::Tokenizer {
                    encoding_model: "bytes".to_owned(),
                    message: source.to_string(),
                })?;
            String::from_utf8(bytes).map_err(|source| graphloom_llm::LlmError::Tokenizer {
                encoding_model: "bytes".to_owned(),
                message: source.to_string(),
            })
        }
    }

    #[derive(Debug)]
    struct RecordingCompletion {
        requests: Arc<Mutex<Vec<CompletionRequest>>>,
        order: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl CompletionModel for RecordingCompletion {
        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> graphloom_llm::Result<CompletionResponse> {
            self.requests.lock().expect("requests").push(request);
            Ok(CompletionResponse::text_for_test(
                "completion",
                "first answer",
            ))
        }

        async fn stream(
            &self,
            request: CompletionRequest,
        ) -> graphloom_llm::Result<CompletionStream> {
            self.requests.lock().expect("requests").push(request);
            self.order
                .lock()
                .expect("order")
                .push("completion_start".to_owned());
            Ok(Box::pin(futures_util::stream::iter([
                Ok(CompletionChunk::text_for_test("completion", "first ", None)),
                Ok(CompletionChunk::text_for_test(
                    "completion",
                    "answer",
                    Some("stop".to_owned()),
                )),
            ])))
        }
    }

    #[derive(Debug)]
    struct RecordingCallback {
        order: Arc<Mutex<Vec<String>>>,
    }

    impl QueryCallbacks for RecordingCallback {
        fn on_context(&self, _context: &QueryContext) {
            self.order.lock().expect("order").push("context".to_owned());
        }

        fn on_llm_new_token(&self, token: &str) {
            self.order
                .lock()
                .expect("order")
                .push(format!("token:{token}"));
        }
    }

    fn text_unit(id: &str, short_id: &str, text: &str) -> TextUnit {
        TextUnit {
            id: id.to_owned(),
            short_id: short_id.to_owned(),
            text: text.to_owned(),
            entity_ids: Vec::new(),
            relationship_ids: Vec::new(),
            covariate_ids: Vec::new(),
            n_tokens: None,
            document_id: None,
        }
    }

    async fn runtime(
        tempdir: &TempDir,
        requests: Arc<Mutex<Vec<CompletionRequest>>>,
        order: Arc<Mutex<Vec<String>>>,
    ) -> BasicQueryRuntime {
        let mut vector_config = VectorStoreConfig::default();
        vector_config.db_uri = tempdir.path().join("lancedb").display().to_string();
        vector_config.vector_size = 2;
        let schema = VectorIndexSchema::for_embedding_name("text_unit_text", 2);
        let vector_store = Arc::new(
            LanceDbVectorStore::connect(&vector_config)
                .await
                .expect("connect LanceDB"),
        );
        vector_store.ensure_index(&schema).await.expect("index");
        vector_store
            .upsert_documents(
                &schema,
                &[
                    VectorDocument {
                        id: "B".to_owned(),
                        vector: vec![0.25, 0.75],
                    },
                    VectorDocument {
                        id: "A".to_owned(),
                        vector: vec![0.20, 0.70],
                    },
                ],
            )
            .await
            .expect("vectors");
        let tokenizer: Arc<dyn Tokenizer> = Arc::new(ByteTokenizer);
        let embedding_model: Arc<dyn EmbeddingModel> =
            Arc::new(MockEmbeddingModel::new("embedding", vec![0.25, 0.75]));
        let completion_model: Arc<dyn CompletionModel> = Arc::new(RecordingCompletion {
            requests,
            order: Arc::clone(&order),
        });
        let mut completion_config = serde_json::from_value::<ModelConfig>(json!({
            "model_provider": "openai",
            "model": "recording",
            "api_key": "secret",
            "call_args": {
                "temperature": 0.2,
                "top_p": 0.8,
                "n": 1,
                "max_tokens": 100,
                "max_completion_tokens": 120,
                "seed": 42,
                "stop": ["END"],
                "presence_penalty": 0.1,
                "frequency_penalty": 0.2,
                "response_format": {"type": "json_object"},
                "stream": false,
                "parallel_tool_calls": false
            }
        }))
        .expect("completion config");
        completion_config.encoding_model = Some("cl100k_base".to_owned());
        let prompt = PromptRepository::new(tempdir.path())
            .load(PromptKind::BasicSearch, None)
            .await
            .expect("prompt");
        BasicQueryRuntime {
            basic_context: BasicContextBuilder {
                config: BasicSearchConfig {
                    k: 2,
                    max_context_tokens: usize::MAX,
                    ..BasicSearchConfig::default()
                },
                text_units: vec![text_unit("A", "0", "first"), text_unit("B", "1", "second")],
                embedding_model,
                embedding_model_id: "embedding".to_owned(),
                vector_store,
                vector_schema: schema,
                tokenizer,
            },
            completion_model,
            completion_model_id: "completion".to_owned(),
            completion_config,
            prompt,
            callbacks: Arc::new(RecordingCallback { order }),
        }
    }

    #[tokio::test]
    async fn test_should_stream_context_tokens_and_completed_result_with_exact_request() {
        let tempdir = TempDir::new().expect("tempdir");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let order = Arc::new(Mutex::new(Vec::new()));
        let runtime = runtime(&tempdir, Arc::clone(&requests), Arc::clone(&order)).await;

        let mut events = basic_search_streaming(runtime, "What happened?", "Short Answer")
            .await
            .expect("stream");
        let mut kinds = Vec::new();
        let mut completed = None;
        while let Some(event) = events.next().await {
            match event.expect("event") {
                QueryEvent::Context(context) => {
                    let QueryContextText::Text(text) = context.text else {
                        panic!("expected context text");
                    };
                    assert_eq!(text, "id|text\n0|first\n1|second\n");
                    kinds.push("context");
                }
                QueryEvent::Token(token) => {
                    assert!(token == "first " || token == "answer");
                    kinds.push("token");
                }
                QueryEvent::Completed(result) => {
                    kinds.push("completed");
                    completed = Some(result);
                }
            }
        }
        assert_eq!(kinds, ["context", "token", "token", "completed"]);
        assert_eq!(
            order.lock().expect("order").as_slice(),
            [
                "context",
                "completion_start",
                "token:first ",
                "token:answer"
            ]
        );

        let requests = requests.lock().expect("requests");
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.stream, Some(true));
        assert_eq!(request.temperature, Some(0.2));
        assert_eq!(request.top_p, Some(0.8));
        assert_eq!(request.n, Some(1));
        assert_eq!(request.max_tokens, Some(100));
        assert_eq!(request.max_completion_tokens, Some(120));
        assert_eq!(request.seed, Some(42));
        assert_eq!(request.stop, Some(json!(["END"])));
        assert_eq!(request.presence_penalty, Some(0.1));
        assert_eq!(request.frequency_penalty, Some(0.2));
        assert_eq!(
            request.response_format,
            Some(json!({"type": "json_object"}))
        );
        assert_eq!(request.extra["parallel_tool_calls"], false);
        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.messages[0].role, graphloom_llm::ChatRole::System);
        assert!(
            request.messages[0]
                .content
                .contains("id|text\n0|first\n1|second\n")
        );
        assert!(request.messages[0].content.contains("Short Answer"));
        assert_eq!(request.messages[1].role, graphloom_llm::ChatRole::User);
        assert_eq!(request.messages[1].content.as_str(), "What happened?");
        assert!(!request.messages[1].content.contains("id|text"));

        let result = completed.expect("completed result");
        assert_eq!(result.response, "first answer");
        assert_eq!(result.usage.llm_calls, 1);
        assert_eq!(result.usage.categories["build_context"].llm_calls, 0);
        assert_eq!(result.usage.categories["response"].llm_calls, 1);
        assert_eq!(result.usage.output_tokens, "first answer".len());
    }
}
