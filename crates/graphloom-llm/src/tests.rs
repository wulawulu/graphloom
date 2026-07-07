use serde::Serialize;

use crate::{
    ChatMessage, CompletionModel, CompletionRequest, DefaultPrompt, EmbeddingModel,
    EmbeddingRequest, MockCompletionModel, MockEmbeddingModel, ModelConfig, PromptLoader,
    Tokenizer, completion_cache_key, embedding_cache_key, parse_claim_tuples,
    parse_community_report, parse_graph_tuples,
};

#[test]
fn test_should_parse_graphrag_graph_tuples() {
    let parsed = parse_graph_tuples(
        "(\"entity\"<|>Alice<|>person<|>A researcher)##(\"relationship\"<|>Alice<|>Bob<|>Works \
         with Bob<|>7)<|COMPLETE|>",
        "tu-1",
    );

    assert_eq!(parsed.entities[0].title, "ALICE");
    assert_eq!(parsed.entities[0].entity_type, "PERSON");
    assert_eq!(parsed.relationships[0].source, "ALICE");
    assert_eq!(parsed.relationships[0].target, "BOB");
    assert_eq!(parsed.relationships[0].weight, 1.0);
}

#[test]
fn test_should_parse_relationship_weight_when_completion_delimiter_is_separate() {
    let parsed = parse_graph_tuples(
        "(\"relationship\"<|>Alice<|>Bob<|>Works with Bob<|>7)##<|COMPLETE|>",
        "tu-1",
    );

    assert_eq!(parsed.relationships[0].weight, 7.0);
}

#[test]
fn test_should_parse_graphrag_claim_tuples_with_missing_fields() {
    let parsed = parse_claim_tuples(
        "(ALICE<|>BOB<|>COMPLIANCE<|>TRUE<|>2024-01-01<|>2024-01-01<|>Desc<|>Quote)##\
         (CAROL<|>NONE)<|COMPLETE|>",
    );

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].subject_id.as_deref(), Some("ALICE"));
    assert_eq!(parsed[0].source_text.as_deref(), Some("Quote"));
    assert_eq!(parsed[1].claim_type, None);
}

#[test]
fn test_should_parse_repaired_community_report_json_frame() {
    let report = parse_community_report(
        "```json\n{\"title\":\"T\",\"summary\":\"S\",\"rating\":5,\"rating_explanation\":\"R\",\"\
         findings\":[{\"summary\":\"A\",\"explanation\":\"B\"}]}\n```",
    )
    .expect("community report should parse");

    assert_eq!(report.title, "T");
    assert_eq!(report.findings[0].summary, "A");
}

#[test]
fn test_should_create_deterministic_cache_keys() {
    let config = model_config();
    let completion = CompletionRequest {
        messages: vec![ChatMessage::user("hello")],
        temperature: Some(0.0),
        top_p: Some(1.0),
        max_tokens: Some(128),
        response_format: Some("json_object".to_owned()),
        cache_namespace: Some("extract_graph".to_owned()),
    };
    let embedding = EmbeddingRequest {
        input: vec!["hello".to_owned()],
        dimensions: Some(3),
        cache_namespace: Some("text_unit_text_embedding".to_owned()),
    };

    assert_eq!(
        completion_cache_key("extract_graph", &config, &completion)
            .expect("completion key should hash"),
        completion_cache_key("extract_graph", &config, &completion)
            .expect("completion key should hash")
    );
    assert_eq!(
        embedding_cache_key("embed", &config, &embedding).expect("embedding key should hash"),
        embedding_cache_key("embed", &config, &embedding).expect("embedding key should hash")
    );
}

#[test]
fn test_should_parse_graphrag_snake_case_model_config() {
    let config: ModelConfig = serde_json::from_value(serde_json::json!({
        "type": "openai",
        "model": "gpt-4o-mini",
        "api_key": "sk-test",
        "api_base": "https://example.test/v1",
        "max_retries": 3,
        "retry_strategy": "exponential_backoff",
        "tokens_per_minute": 1000,
        "requests_per_minute": 60,
        "encoding_model": "cl100k_base"
    }))
    .expect("snake_case config should parse");

    assert_eq!(config.api_key.as_deref(), Some("sk-test"));
    assert_eq!(config.api_base.as_deref(), Some("https://example.test/v1"));
    assert_eq!(config.max_retries, Some(3));
    assert_eq!(config.encoding_model.as_deref(), Some("cl100k_base"));
}

#[tokio::test]
async fn test_should_render_default_prompt_with_tera_values() {
    #[derive(Debug, Serialize)]
    struct Values<'a> {
        entity_types: &'a str,
        input_text: &'a str,
    }

    let loader = PromptLoader::new(".");
    let rendered = loader
        .render(
            DefaultPrompt::ExtractGraph,
            None,
            &Values {
                entity_types: "PERSON",
                input_text: "Alice met Bob.",
            },
        )
        .await
        .expect("prompt should render");

    assert!(rendered.contains("Entity_types: PERSON"));
    assert!(rendered.contains("Text: Alice met Bob."));
}

#[tokio::test]
async fn test_should_use_mock_models() {
    let completion = MockCompletionModel::new("mock", vec!["answer".to_owned()]);
    let response = completion
        .complete(CompletionRequest {
            messages: vec![ChatMessage::user("question")],
            temperature: None,
            top_p: None,
            max_tokens: None,
            response_format: None,
            cache_namespace: None,
        })
        .await
        .expect("mock completion should respond");
    assert_eq!(response.content, "answer");

    let embedding = MockEmbeddingModel::new("mock", vec![1.0, 2.0]);
    let response = embedding
        .embed(EmbeddingRequest {
            input: vec!["a".to_owned(), "b".to_owned()],
            dimensions: None,
            cache_namespace: None,
        })
        .await
        .expect("mock embedding should respond");
    assert_eq!(response.embeddings, vec![vec![1.0, 2.0], vec![1.0, 2.0]]);
}

#[test]
fn test_should_tokenize_with_tiktoken() {
    let tokenizer =
        crate::TiktokenTokenizer::new("cl100k_base").expect("cl100k_base should be supported");
    let tokens = tokenizer.encode("hello").expect("text should encode");
    assert!(!tokens.is_empty());
    assert_eq!(
        tokenizer.decode(&tokens).expect("tokens should decode"),
        "hello"
    );
}

fn model_config() -> ModelConfig {
    ModelConfig {
        provider_type: "openai".to_owned(),
        model: "gpt-4o-mini".to_owned(),
        api_key: Some("sk-test".to_owned()),
        api_base: None,
        organization: None,
        timeout: None,
        max_retries: Some(1),
        retry_strategy: None,
        tokens_per_minute: None,
        requests_per_minute: None,
        encoding_model: Some("cl100k_base".to_owned()),
    }
}
