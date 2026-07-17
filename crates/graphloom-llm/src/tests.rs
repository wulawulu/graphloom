use secrecy::ExposeSecret;

use crate::{
    ChatMessage, CompletionModel, CompletionRequest, EmbeddingModel, EmbeddingRequest,
    MockCompletionModel, MockEmbeddingModel, ModelConfig, Tokenizer, completion_cache_key,
    embedding_cache_key, embedding_request_cache_key, graphrag_cache_key, parse_claim_tuples,
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
    assert!((parsed.relationships[0].weight - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_should_parse_relationship_weight_when_completion_delimiter_is_separate() {
    let parsed = parse_graph_tuples(
        "(\"relationship\"<|>Alice<|>Bob<|>Works with Bob<|>7)##<|COMPLETE|>",
        "tu-1",
    );

    assert!((parsed.relationships[0].weight - 7.0).abs() < f64::EPSILON);
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
        temperature: Some(0.0),
        top_p: Some(1.0),
        max_tokens: Some(128),
        response_format: Some(serde_json::json!({"type": "json_object"})),
        ..CompletionRequest::new(vec![ChatMessage::user("hello")])
    };
    let embedding = EmbeddingRequest {
        dimensions: Some(3),
        ..EmbeddingRequest::new(vec!["hello".to_owned()])
    };

    assert_eq!(
        completion_cache_key("extract_graph", &config, &completion)
            .expect("completion key should hash"),
        "f572b82de883c02e40e55802097327b085960df971ffacb9c6e0844e56d09fec_v4"
    );
    assert_eq!(
        embedding_cache_key("embed", &config, &embedding).expect("embedding key should hash"),
        "c1acdf25b4b7df2452b560f79939baf569a89d280b13221dbe8bf15a3259e337_v4"
    );
}

#[test]
fn test_should_create_canonical_embedding_request_cache_keys() {
    let base = EmbeddingRequest {
        dimensions: Some(3),
        ..EmbeddingRequest::new(vec!["hello".to_owned(), "world".to_owned()])
    };
    let same = base.clone();
    let changed_input = EmbeddingRequest {
        input: vec!["hello!".to_owned(), "world".to_owned()],
        ..base.clone()
    };
    let changed_order = EmbeddingRequest {
        input: vec!["world".to_owned(), "hello".to_owned()],
        ..base.clone()
    };
    let changed_dimensions = EmbeddingRequest {
        dimensions: Some(4),
        ..base.clone()
    };

    let key = embedding_request_cache_key(&base).expect("base key");
    assert_eq!(
        key,
        embedding_request_cache_key(&same).expect("same request key")
    );
    assert_ne!(
        key,
        embedding_request_cache_key(&changed_input).expect("changed input key")
    );
    assert_ne!(
        key,
        embedding_request_cache_key(&changed_order).expect("changed order key")
    );
    assert_ne!(
        key,
        embedding_request_cache_key(&changed_dimensions).expect("changed dimensions key")
    );
}

#[test]
fn test_should_match_graphrag_cache_key_filtering_and_yaml_hash() {
    let key = graphrag_cache_key(&serde_json::json!({
        "api_key": "sk-test",
        "metrics": {},
        "messages": [{"role": "user", "content": "hello"}],
        "response_format": null,
        "timeout": 30,
    }))
    .expect("cache key should hash");

    assert_eq!(
        key,
        "98dbb7395b26e4b91598416540218dd2362f4bf67f55310c01d38ba6b555dbdd_v4",
    );
}

#[test]
fn test_should_match_graphrag_cache_key_for_multiline_message() {
    let key = graphrag_cache_key(&serde_json::json!({
        "messages": [{"role": "user", "content": "line1\nline2"}],
        "response_format": null,
    }))
    .expect("cache key should hash");

    assert_eq!(
        key,
        "39161cfaa02b838880f47181bb47f72b4047fc46e5e376f3c50a4c527926f9ce_v4",
    );
}

#[test]
fn test_should_parse_graphrag_snake_case_model_config() {
    let config: ModelConfig = serde_json::from_value(serde_json::json!({
        "model_provider": "openai",
        "model": "gpt-4o-mini",
        "auth_method": "api_key",
        "api_key": "sk-test",
        "api_base": "https://example.test/v1",
        "retry": {
            "type": "exponential_backoff",
            "max_retries": 3
        },
        "tokens_per_minute": 1000,
        "requests_per_minute": 60,
        "encoding_model": "cl100k_base"
    }))
    .expect("snake_case config should parse");

    assert_eq!(
        config.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-test")
    );
    assert_eq!(config.api_base.as_deref(), Some("https://example.test/v1"));
    assert_eq!(config.provider_type(), "openai");
    assert_eq!(config.auth_method, "api_key");
    assert_eq!(config.effective_retry_strategy(), "exponential_backoff");
    assert_eq!(config.effective_max_retries(), 3);
    assert_eq!(config.encoding_model.as_deref(), Some("cl100k_base"));
}

#[test]
fn test_should_deserialize_yaml_api_key_as_secret() {
    let config: ModelConfig = serde_yaml::from_str(
        "model_provider: openai\nmodel: gpt-test\nauth_method: api_key\napi_key: yaml-secret\n",
    )
    .expect("YAML model config should parse");

    assert_eq!(
        config.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("yaml-secret")
    );
}

#[test]
fn test_should_redact_api_key_in_serialization_and_debug() {
    let config: ModelConfig = serde_json::from_value(serde_json::json!({
        "model_provider": "openai",
        "model": "gpt-test",
        "api_key": "serialization-secret",
        "api_base": "https://models.example/v1"
    }))
    .expect("model config should parse");

    let json = serde_json::to_value(&config).expect("JSON serialization");
    let yaml = serde_yaml::to_string(&config).expect("YAML serialization");
    let debug = format!("{config:?}");

    assert_eq!(json["apiKey"], "<redacted>");
    assert_eq!(json["apiBase"], "https://models.example/v1");
    for rendered in [json.to_string(), yaml, debug] {
        assert!(!rendered.contains("serialization-secret"));
        assert!(rendered.contains("<redacted>"));
    }
}

#[test]
fn test_should_parse_legacy_model_config_fields() {
    let config: ModelConfig = serde_json::from_value(serde_json::json!({
        "type": "openai",
        "model": "gpt-4o-mini",
        "api_key": "sk-test",
        "max_retries": 4,
        "retry_strategy": "immediate"
    }))
    .expect("legacy config should parse");

    assert_eq!(config.provider_type(), "openai");
    assert_eq!(config.auth_method, "api_key");
    assert_eq!(config.effective_retry_strategy(), "immediate");
    assert_eq!(config.effective_max_retries(), 4);
}

#[test]
fn test_should_default_model_config_retries() {
    let config: ModelConfig = serde_json::from_value(serde_json::json!({
        "type": "openai",
        "model": "gpt-4o-mini",
        "api_key": "sk-test"
    }))
    .expect("config should parse");

    assert_eq!(config.effective_max_retries(), 1);
    assert_eq!(config.effective_retry_strategy(), "exponential_backoff");
}

#[test]
fn test_should_resolve_graphrag_provider_defaults() {
    let deepseek: ModelConfig = serde_json::from_value(serde_json::json!({
        "model_provider": "deepseek",
        "model": "deepseek-v4-flash",
        "api_key": "sk-test"
    }))
    .expect("deepseek config");
    let ollama: ModelConfig = serde_json::from_value(serde_json::json!({
        "model_provider": "ollama",
        "model": "bge-m3",
        "api_key": "ollama",
        "api_base": "http://localhost:11434"
    }))
    .expect("ollama config");

    deepseek
        .validate_openai_compatible("completion")
        .expect("DeepSeek should use the OpenAI-compatible transport");
    ollama
        .validate_openai_compatible("embedding")
        .expect("Ollama should use the OpenAI-compatible transport");
    assert_eq!(deepseek.effective_api_base(), "https://api.deepseek.com");
    assert_eq!(ollama.effective_api_base(), "http://localhost:11434/v1");
    assert_eq!(deepseek.effective_tokenizer_encoding(), "cl100k_base");
    assert_eq!(ollama.effective_tokenizer_encoding(), "cl100k_base");
}

#[test]
fn test_should_preserve_explicit_api_base_and_tokenizer_overrides() {
    let deepseek: ModelConfig = serde_json::from_value(serde_json::json!({
        "model_provider": "deepseek",
        "model": "deepseek-chat",
        "api_key": "sk-test",
        "api_base": "https://gateway.example/deepseek/v1",
        "encoding_model": "o200k_base"
    }))
    .expect("deepseek config");
    let ollama: ModelConfig = serde_json::from_value(serde_json::json!({
        "model_provider": "ollama",
        "model": "bge-m3",
        "api_key": "ollama",
        "api_base": "http://localhost:11434/v1/"
    }))
    .expect("ollama config");

    assert_eq!(
        deepseek.effective_api_base(),
        "https://gateway.example/deepseek/v1"
    );
    assert_eq!(deepseek.effective_tokenizer_encoding(), "o200k_base");
    assert_eq!(ollama.effective_api_base(), "http://localhost:11434/v1");
}

#[test]
fn test_should_reject_zero_model_config_retries() {
    let config: ModelConfig = serde_json::from_value(serde_json::json!({
        "type": "openai",
        "model": "gpt-4o-mini",
        "api_key": "sk-test",
        "max_retries": 0
    }))
    .expect("config should parse");

    assert!(config.validate_openai_compatible("chat").is_err());
}

#[test]
fn test_should_reject_unsupported_provider_auth_and_placeholder_key() {
    let unsupported_provider: ModelConfig = serde_json::from_value(serde_json::json!({
        "model_provider": "azure",
        "model": "gpt",
        "api_key": "sk-test"
    }))
    .expect("config");
    assert!(
        unsupported_provider
            .validate_openai_compatible("chat")
            .expect_err("azure unsupported")
            .to_string()
            .contains("openai, deepseek, and ollama")
    );

    let unsupported_auth: ModelConfig = serde_json::from_value(serde_json::json!({
        "model_provider": "openai",
        "model": "gpt",
        "auth_method": "azure_managed_identity",
        "api_key": "sk-test"
    }))
    .expect("config");
    assert!(
        unsupported_auth
            .validate_openai_compatible("chat")
            .expect_err("auth unsupported")
            .to_string()
            .contains("only api_key")
    );

    let placeholder: ModelConfig = serde_json::from_value(serde_json::json!({
        "model_provider": "openai",
        "model": "gpt",
        "auth_method": "api_key",
        "api_key": "<API_KEY>"
    }))
    .expect("config");
    assert!(placeholder.validate_openai_compatible("chat").is_err());
    assert!(!format!("{placeholder:?}").contains("<API_KEY>"));

    let empty: ModelConfig = serde_json::from_value(serde_json::json!({
        "model_provider": "openai",
        "model": "gpt",
        "auth_method": "api_key",
        "api_key": "  "
    }))
    .expect("config");
    assert!(empty.validate_openai_compatible("chat").is_err());
}

#[tokio::test]
async fn test_should_use_mock_models() {
    let completion = MockCompletionModel::new("mock", vec!["answer".to_owned()]);
    let response = completion
        .complete(CompletionRequest::new(vec![ChatMessage::user("question")]))
        .await
        .expect("mock completion should respond");
    assert_eq!(response.content().expect("content"), "answer");

    let embedding = MockEmbeddingModel::new("mock", vec![1.0, 2.0]);
    let response = embedding
        .embed(EmbeddingRequest::new(vec!["a".to_owned(), "b".to_owned()]))
        .await
        .expect("mock embedding should respond");
    assert_eq!(
        response.embeddings().collect::<Vec<_>>(),
        vec![&[1.0, 2.0][..], &[1.0, 2.0][..]]
    );
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
        auth_method: "api_key".to_owned(),
        api_key: Some("sk-test".to_owned().into()),
        api_base: None,
        organization: None,
        timeout: None,
        max_retries: 1,
        retry_strategy: None,
        retry: None,
        tokens_per_minute: None,
        requests_per_minute: None,
        encoding_model: Some("cl100k_base".to_owned()),
        call_args: std::collections::BTreeMap::new(),
    }
}
