use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use bytes::Bytes;
use graphloom_cache::{Cache, JsonCache, MemoryCache};
use graphloom_llm::{
    CacheStatus, CachedCompletionModel, CachedEmbeddingModel, CachedModelResult, ChatMessage,
    CompletionModel, CompletionRequest, CompletionResponse, EmbeddingModel, EmbeddingRequest,
    EmbeddingResponse, MockCompletionModel, ModelConfig, OpenAiCompletionModel,
    OpenAiEmbeddingModel,
};
use graphloom_storage::FileStorage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, method, path},
};

const CACHE_KEY_CASES: &str = include_str!("fixtures/graphrag/cache_keys/cases.json");
const CACHE_KEY_YAML: &str = include_str!("fixtures/graphrag/cache_keys/expected.yaml.json");
const CACHE_KEY_VALUES: &str = include_str!("fixtures/graphrag/cache_keys/expected_keys.json");

const COMPLETION_FIXTURE: &str = include_str!(
    "fixtures/graphrag/completion/\
     04ad9d031321cd7dc75fc1b9a3341700367369b97ec0168c51a16a1e076a1e3e_v4"
);
const EMBEDDING_FIXTURE: &str = include_str!(
    "fixtures/graphrag/embedding/\
     90c0451b04544c23e0c20382f46fb97d894a1ee4ff4a85f020be5635afb81bc7_v4"
);

#[test]
fn test_should_match_graphrag_completion_and_embedding_key_goldens() {
    let cases = [
        (
            include_str!("fixtures/graphrag/completion_request.json"),
            include_str!("fixtures/graphrag/completion_expected_key.txt"),
        ),
        (
            include_str!("fixtures/graphrag/embedding_request.json"),
            include_str!("fixtures/graphrag/embedding_expected_key.txt"),
        ),
    ];
    for (request, expected) in cases {
        let kwargs: Value = serde_json::from_str(request).expect("request fixture");
        assert_eq!(
            graphloom_llm::graphrag_cache_key(&kwargs).expect("cache key"),
            expected.trim()
        );
    }
}

#[derive(Debug, Deserialize)]
struct CacheKeyCase {
    name: String,
    kwargs: Value,
}

#[test]
fn test_should_match_pyyaml_golden_corpus() {
    let cases: Vec<CacheKeyCase> = serde_json::from_str(CACHE_KEY_CASES).expect("cases");
    let expected_yaml: std::collections::BTreeMap<String, String> =
        serde_json::from_str(CACHE_KEY_YAML).expect("YAML goldens");
    let expected_keys: std::collections::BTreeMap<String, String> =
        serde_json::from_str(CACHE_KEY_VALUES).expect("key goldens");

    for case in cases {
        let yaml = graphloom_llm::graphrag_cache_yaml(&case.kwargs).expect("cache YAML");
        let key = graphloom_llm::graphrag_cache_key(&case.kwargs).expect("cache key");
        let expected_yaml = expected_yaml.get(&case.name).expect("case YAML");
        let expected_key = expected_keys.get(&case.name).expect("case key");
        let first_difference = first_differing_byte(expected_yaml, &yaml);
        assert_eq!(
            (&yaml, &key),
            (expected_yaml, expected_key),
            "case {}\nexpected YAML: {expected_yaml:?}\nactual YAML: {yaml:?}\nfirst differing \
             byte: {first_difference:?}\nexpected key: {expected_key}\nactual key: {key}",
            case.name,
        );
    }
}

fn first_differing_byte(expected: &str, actual: &str) -> Option<usize> {
    expected
        .bytes()
        .zip(actual.bytes())
        .position(|(expected, actual)| expected != actual)
        .or_else(|| (expected.len() != actual.len()).then_some(expected.len().min(actual.len())))
}

fn assert_cache_key_cases(names: &[&str]) {
    let cases: Vec<CacheKeyCase> = serde_json::from_str(CACHE_KEY_CASES).expect("cases");
    let expected_yaml: std::collections::BTreeMap<String, String> =
        serde_json::from_str(CACHE_KEY_YAML).expect("YAML goldens");
    let expected_keys: std::collections::BTreeMap<String, String> =
        serde_json::from_str(CACHE_KEY_VALUES).expect("key goldens");
    for name in names {
        let case = cases
            .iter()
            .find(|case| case.name == *name)
            .expect("named case");
        assert_eq!(
            graphloom_llm::graphrag_cache_yaml(&case.kwargs).expect("YAML"),
            *expected_yaml.get(*name).expect("expected YAML"),
            "YAML mismatch for {name}",
        );
        assert_eq!(
            graphloom_llm::graphrag_cache_key(&case.kwargs).expect("key"),
            *expected_keys.get(*name).expect("expected key"),
            "key mismatch for {name}",
        );
    }
}

#[test]
fn test_should_match_long_plain_ascii_pyyaml() {
    assert_cache_key_cases(&["long-plain-ascii", "long-string-in-array"]);
}

#[test]
fn test_should_match_long_nested_ascii_pyyaml() {
    assert_cache_key_cases(&["long-string-in-nested-object", "long-single-quoted-ascii"]);
}

#[test]
fn test_should_match_long_multiline_ascii_pyyaml() {
    assert_cache_key_cases(&["long-multiline-ascii"]);
}

#[test]
fn test_should_match_yaml_indicator_goldens() {
    assert_cache_key_cases(&[
        "leading-hash",
        "leading-pipe",
        "leading-greater-than",
        "leading-dash-space",
        "leading-colon-space",
        "leading-bracket",
        "leading-brace",
        "document-start",
        "document-end",
    ]);
}

#[test]
fn test_should_match_yaml_indicator_edge_cases() {
    assert_cache_key_cases(&[
        "leading-double-quote",
        "leading-single-quote",
        "leading-closing-bracket",
        "leading-closing-brace",
        "leading-comma",
        "leading-percent",
        "anchor-without-space",
        "alias-without-space",
        "tag-without-space",
        "document-start-exact",
        "document-end-exact",
    ]);
}

#[test]
fn test_should_match_long_scalars_with_repeated_spaces() {
    let names = [
        "long-plain-with-double-spaces",
        "long-plain-with-triple-spaces",
        "long-single-quoted-with-double-spaces",
        "long-string-with-alignment-spaces",
        "long-string-with-leading-continuation-spaces",
        "long-sequence-string-with-double-spaces",
        "long-nested-string-with-double-spaces",
    ];
    assert_cache_key_cases(&names);

    let cases: Vec<CacheKeyCase> = serde_json::from_str(CACHE_KEY_CASES).expect("cases");
    let expected_yaml: std::collections::BTreeMap<String, String> =
        serde_json::from_str(CACHE_KEY_YAML).expect("YAML goldens");
    for name in names {
        let case = cases
            .iter()
            .find(|case| case.name == name)
            .expect("named case");
        let actual_yaml = graphloom_llm::graphrag_cache_yaml(&case.kwargs).expect("actual YAML");
        let expected_decoded: Value =
            serde_yaml::from_str(expected_yaml.get(name).expect("expected YAML"))
                .expect("decode expected YAML");
        let actual_decoded: Value = serde_yaml::from_str(&actual_yaml).expect("decode actual YAML");
        assert_eq!(
            expected_decoded, case.kwargs,
            "expected semantics for {name}"
        );
        assert_eq!(actual_decoded, case.kwargs, "actual semantics for {name}");
    }
}

#[test]
fn test_should_match_pyyaml_float_exponents() {
    assert_cache_key_cases(&[
        "float-positive-one-digit-exponent",
        "float-negative-one-digit-exponent",
        "float-positive-two-digit-exponent",
        "float-negative-two-digit-exponent",
        "float-large-positive-exponent",
        "float-small-negative-exponent",
    ]);
}

#[test]
fn test_should_match_real_extract_graph_request_key() {
    assert_cache_key_cases(&["real-extract-graph-request"]);
}

#[test]
fn test_should_match_clean_multiline_ascii_scalars() {
    assert_cache_key_cases(&[
        "long-clean-multiline-ascii",
        "long-clean-multiline-with-blank-lines",
        "long-clean-multiline-with-single-quote",
    ]);
}

#[test]
fn test_should_match_clean_multiline_ascii_in_messages() {
    assert_cache_key_cases(&["long-clean-multiline-in-message"]);
}

#[test]
fn test_should_match_clean_multiline_ascii_in_nested_values() {
    assert_cache_key_cases(&[
        "long-clean-multiline-in-array",
        "long-clean-multiline-in-nested-object",
    ]);
}

#[test]
fn test_should_quote_pyyaml_implicit_null_scalars() {
    assert_cache_key_cases(&[
        "implicit-null-tilde",
        "implicit-null-null-lower",
        "implicit-null-null-upper",
        "implicit-null-null-title",
    ]);
}

#[test]
fn test_should_quote_pyyaml_implicit_float_scalars() {
    assert_cache_key_cases(&[
        "implicit-positive-inf",
        "implicit-positive-inf-with-plus",
        "implicit-negative-inf",
        "implicit-nan",
        "implicit-upper-inf",
        "implicit-upper-nan",
        "implicit-leading-dot-float",
        "implicit-trailing-dot-float",
        "implicit-sexagesimal-float",
        "implicit-signed-float",
        "implicit-exponent-float",
    ]);
}

#[test]
fn test_should_quote_pyyaml_implicit_integer_scalars() {
    assert_cache_key_cases(&[
        "implicit-hex",
        "implicit-binary",
        "implicit-octal",
        "implicit-sexagesimal-integer",
        "implicit-leading-zero",
        "implicit-signed-integer",
    ]);
}

#[test]
fn test_should_quote_pyyaml_timestamp_scalars() {
    assert_cache_key_cases(&[
        "implicit-date",
        "implicit-datetime",
        "implicit-datetime-z",
        "implicit-datetime-timezone",
        "implicit-time-like",
    ]);
}

#[test]
fn test_should_preserve_yaml_semantics_for_all_golden_cases() {
    let cases: Vec<CacheKeyCase> = serde_json::from_str(CACHE_KEY_CASES).expect("cases");
    let expected_yaml: std::collections::BTreeMap<String, String> =
        serde_json::from_str(CACHE_KEY_YAML).expect("YAML goldens");

    for case in cases {
        let expected_decoded: Value =
            serde_yaml::from_str(expected_yaml.get(&case.name).expect("expected YAML"))
                .expect("decode expected YAML");
        let actual_yaml = graphloom_llm::graphrag_cache_yaml(&case.kwargs).expect("actual YAML");
        let actual_decoded: Value = serde_yaml::from_str(&actual_yaml).expect("decode actual YAML");
        assert_eq!(
            actual_decoded, expected_decoded,
            "semantic mismatch for {}",
            case.name,
        );
    }
}

#[test]
fn test_should_match_multiline_trailing_newline_scalars() {
    assert_cache_key_cases(&[
        "multiline-trailing-newline",
        "multiline-trailing-two-newlines",
        "multiline-trailing-three-newlines",
        "long-multiline-trailing-newline",
        "long-multiline-trailing-two-newlines",
        "multiline-only-newline",
        "multiline-only-two-newlines",
    ]);
}

#[test]
fn test_should_match_multiline_trailing_newlines_in_nested_values() {
    assert_cache_key_cases(&[
        "message-content-trailing-newline",
        "array-multiline-trailing-newline",
        "nested-multiline-trailing-newline",
    ]);
}

#[test]
fn test_should_quote_pyyaml_merge_and_value_scalars() {
    assert_cache_key_cases(&["implicit-merge", "implicit-value"]);
}

#[test]
fn test_should_match_pyyaml_named_escape_replacements() {
    let names = [
        "escape-null",
        "escape-bell",
        "escape-backspace",
        "escape-tab",
        "escape-newline",
        "escape-vertical-tab",
        "escape-form-feed",
        "escape-carriage-return",
        "escape-escape",
        "escape-double-quote",
        "escape-backslash",
        "escape-next-line",
        "escape-non-breaking-space",
        "escape-line-separator",
        "escape-paragraph-separator",
    ];
    assert_cache_key_cases(&names);

    let expected: std::collections::BTreeMap<String, String> =
        serde_json::from_str(CACHE_KEY_YAML).expect("YAML goldens");
    for (name, replacement) in [
        ("escape-null", "\\0"),
        ("escape-bell", "\\a"),
        ("escape-backspace", "\\b"),
        ("escape-vertical-tab", "\\v"),
        ("escape-form-feed", "\\f"),
        ("escape-escape", "\\e"),
    ] {
        assert!(
            expected
                .get(name)
                .expect("expected YAML")
                .contains(replacement),
            "missing named replacement {replacement} for {name}",
        );
    }
}

#[test]
fn test_should_match_pyyaml_numeric_escape_codes() {
    assert_cache_key_cases(&["escape-delete", "escape-c1-control"]);
}

#[test]
fn test_should_escape_non_breaking_space_like_pyyaml() {
    assert_cache_key_cases(&["escape-non-breaking-space"]);
    let expected: std::collections::BTreeMap<String, String> =
        serde_json::from_str(CACHE_KEY_YAML).expect("YAML goldens");
    assert!(
        expected
            .get("escape-non-breaking-space")
            .expect("expected YAML")
            .contains("\\_")
    );
}

#[test]
fn test_should_force_ascii_control_characters_into_double_quotes() {
    let names = [
        "escape-null",
        "escape-bell",
        "escape-backspace",
        "escape-tab",
        "escape-vertical-tab",
        "escape-form-feed",
        "escape-carriage-return",
        "escape-escape",
        "escape-delete",
    ];
    assert_cache_key_cases(&names);
    let cases: Vec<CacheKeyCase> = serde_json::from_str(CACHE_KEY_CASES).expect("cases");
    for name in names {
        let case = cases
            .iter()
            .find(|case| case.name == name)
            .expect("named case");
        assert!(
            graphloom_llm::graphrag_cache_yaml(&case.kwargs)
                .expect("actual YAML")
                .starts_with("value: \""),
            "control scalar must be double quoted for {name}",
        );
    }
}

#[test]
fn test_should_match_pyyaml_explicit_mapping_keys() {
    assert_cache_key_cases(&[
        "empty-mapping-key",
        "multiline-mapping-key",
        "trailing-newline-mapping-key",
        "only-newline-mapping-key",
        "two-newlines-mapping-key",
        "long-mapping-key",
        "explicit-key-empty-object",
        "explicit-key-empty-array",
        "explicit-key-nested-object",
        "explicit-key-nested-array",
    ]);
}

#[test]
fn test_should_match_explicit_keys_in_json_schema() {
    assert_cache_key_cases(&[
        "empty-json-schema-property-name",
        "multiline-json-schema-property-name",
        "trailing-newline-json-schema-property-name",
        "long-json-schema-property-name",
    ]);
}

#[test]
fn test_should_match_explicit_keys_in_tool_parameters() {
    assert_cache_key_cases(&[
        "empty-tool-parameter-name",
        "multiline-tool-parameter-name",
        "long-tool-parameter-name",
    ]);
}

#[test]
fn test_should_match_explicit_keys_inside_sequences() {
    assert_cache_key_cases(&[
        "sequence-mapping-empty-key",
        "sequence-mapping-multiline-key",
        "sequence-mapping-long-key",
        "sequence-mapping-mixed-simple-explicit",
        "sequence-mapping-explicit-nested-object",
        "sequence-mapping-explicit-array",
    ]);
}

#[test]
fn test_should_match_pyyaml_simple_key_length_boundary() {
    assert_cache_key_cases(&[
        "mapping-key-at-simple-limit",
        "mapping-key-over-simple-limit",
        "nested-mapping-key-at-simple-limit",
        "nested-mapping-key-over-simple-limit",
        "long-unicode-key-at-simple-limit",
        "long-unicode-key-over-simple-limit",
        "long-control-escaped-key",
    ]);
}

#[test]
fn test_should_account_for_explicit_key_indicator_columns() {
    assert_cache_key_cases(&[
        "explicit-long-plain-key-with-break-space",
        "explicit-long-single-quoted-key-with-break-space",
        "explicit-long-double-quoted-key-near-width",
        "long-control-escaped-key",
    ]);
}

#[test]
fn test_should_use_real_column_for_double_quoted_explicit_keys() {
    assert_cache_key_cases(&[
        "explicit-long-double-quoted-key-near-width",
        "long-control-escaped-key",
        "explicit-double-quoted-key-wrap-before-boundary",
        "explicit-double-quoted-key-wrap-at-boundary",
        "explicit-double-quoted-key-wrap-after-boundary",
        "nested-explicit-double-quoted-key-wrap",
        "sequence-explicit-double-quoted-key-wrap",
    ]);
}

#[test]
fn test_should_wrap_nested_explicit_keys_at_pyyaml_columns() {
    assert_cache_key_cases(&[
        "nested-explicit-long-key-with-break-space",
        "sequence-explicit-long-key-with-break-space",
        "json-schema-explicit-long-property-with-break-space",
    ]);
}

#[test]
fn test_should_preserve_sort_order_with_explicit_keys() {
    assert_cache_key_cases(&["mixed-explicit-key-sort-order"]);
}

#[test]
fn test_should_preserve_yaml_semantics_for_explicit_keys() {
    let names = [
        "empty-mapping-key",
        "multiline-mapping-key",
        "explicit-key-empty-object",
        "explicit-key-empty-array",
        "explicit-key-nested-object",
        "explicit-key-nested-array",
        "sequence-mapping-mixed-simple-explicit",
    ];
    let cases: Vec<CacheKeyCase> = serde_json::from_str(CACHE_KEY_CASES).expect("cases");
    let expected_yaml: std::collections::BTreeMap<String, String> =
        serde_json::from_str(CACHE_KEY_YAML).expect("YAML goldens");

    for name in names {
        let case = cases
            .iter()
            .find(|case| case.name == name)
            .expect("named case");
        let expected: Value = serde_yaml::from_str(expected_yaml.get(name).expect("expected YAML"))
            .expect("decode expected YAML");
        let actual_yaml = graphloom_llm::graphrag_cache_yaml(&case.kwargs).expect("actual YAML");
        let actual: Value = serde_yaml::from_str(&actual_yaml).expect("decode actual YAML");
        assert_eq!(actual, expected, "semantic mismatch for {name}");
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Outer<R> {
    result: CachedModelResult<R>,
}

#[test]
fn test_should_decode_and_round_trip_real_graphrag_completion_cache_fixture() {
    let original: Value = serde_json::from_str(COMPLETION_FIXTURE).expect("fixture JSON");
    let typed: Outer<CompletionResponse> =
        serde_json::from_value(original.clone()).expect("canonical completion cache");

    assert!(
        typed
            .result
            .response
            .content()
            .expect("content")
            .contains("西门庆")
    );
    assert!(typed.result.response.reasoning_content().is_some());
    assert_eq!(
        typed
            .result
            .response
            .first_choice()
            .and_then(|choice| choice.finish_reason.as_deref()),
        Some("stop")
    );
    assert!(!typed.result.metrics.is_empty());
    assert_eq!(
        serde_json::to_value(typed).expect("encoded fixture"),
        original
    );
}

#[test]
fn test_should_decode_and_round_trip_real_graphrag_embedding_cache_fixture() {
    let original: Value = serde_json::from_str(EMBEDDING_FIXTURE).expect("fixture JSON");
    let typed: Outer<EmbeddingResponse> =
        serde_json::from_value(original.clone()).expect("canonical embedding cache");

    assert!(!typed.result.response.data.is_empty());
    assert!(!typed.result.response.data[0].embedding.is_empty());
    assert!(!typed.result.metrics.is_empty());
    assert_eq!(
        serde_json::to_value(typed).expect("encoded fixture"),
        original
    );
}

#[tokio::test]
async fn test_should_hit_graphrag_completion_payload_without_provider_call() {
    let fixture: Outer<CompletionResponse> =
        serde_json::from_str(COMPLETION_FIXTURE).expect("fixture");
    let request = CompletionRequest::new(vec![ChatMessage::user("fixture request")]);
    let key = graphloom_llm::completion_request_cache_key(&request).expect("key");
    let cache = Arc::new(MemoryCache::new());
    cache
        .set(
            &key,
            Bytes::from(serde_json::to_vec(&fixture.result).expect("payload")),
        )
        .await
        .expect("seed cache");
    let inner = Arc::new(CountingCompletionModel::default());
    let cached = CachedCompletionModel::new(inner.clone(), cache);

    let response = cached.complete(request).await.expect("cache hit");

    assert!(response.content().expect("content").contains("西门庆"));
    assert_eq!(response.metadata.cache_status, CacheStatus::Hit);
    assert_eq!(inner.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn test_should_cache_completion_miss_and_invalidate_legacy_payload() {
    let request = CompletionRequest::new(vec![ChatMessage::user("cache me")]);
    let key = graphloom_llm::completion_request_cache_key(&request).expect("key");
    let cache = Arc::new(MemoryCache::new());
    cache
        .set(&key, Bytes::from_static(br#"{"content":"legacy"}"#))
        .await
        .expect("legacy seed");
    let inner = Arc::new(CountingCompletionModel::default());
    let cached = CachedCompletionModel::new(inner.clone(), cache.clone());

    let first = cached.complete(request.clone()).await.expect("cache miss");
    let second = cached.complete(request).await.expect("cache hit");

    assert_eq!(first.metadata.cache_status, CacheStatus::Miss);
    assert_eq!(second.metadata.cache_status, CacheStatus::Hit);
    assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
    let stored = cache.get(&key).await.expect("cache read").expect("entry");
    let value: Value = serde_json::from_slice(&stored).expect("new payload");
    assert!(value.get("response").is_some());
    assert_eq!(
        value.get("metrics"),
        Some(&Value::Object(Default::default()))
    );
}

#[tokio::test]
async fn test_should_cache_embedding_miss_then_hit() {
    let request = EmbeddingRequest::new(vec!["cache me".to_owned()]);
    let cache = Arc::new(MemoryCache::new());
    let inner = Arc::new(CountingEmbeddingModel::default());
    let cached = CachedEmbeddingModel::new(inner.clone(), cache);

    let first = cached.embed(request.clone()).await.expect("cache miss");
    let second = cached.embed(request).await.expect("cache hit");

    assert_eq!(first.metadata.cache_status, CacheStatus::Miss);
    assert_eq!(second.metadata.cache_status, CacheStatus::Hit);
    assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_should_hit_graphrag_embedding_payload_without_provider_call() {
    let fixture: Outer<EmbeddingResponse> =
        serde_json::from_str(EMBEDDING_FIXTURE).expect("fixture");
    let request = EmbeddingRequest::new(vec!["fixture request".to_owned()]);
    let key = graphloom_llm::embedding_request_cache_key(&request).expect("key");
    let cache = Arc::new(MemoryCache::new());
    cache
        .set(
            &key,
            Bytes::from(serde_json::to_vec(&fixture.result).expect("payload")),
        )
        .await
        .expect("seed cache");
    let inner = Arc::new(CountingEmbeddingModel::default());
    let cached = CachedEmbeddingModel::new(inner.clone(), cache);

    let response = cached.embed(request).await.expect("cache hit");

    assert!(!response.data.is_empty());
    assert_eq!(response.metadata.cache_status, CacheStatus::Hit);
    assert_eq!(inner.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn test_should_call_provider_repeatedly_without_cache_middleware() {
    let request = CompletionRequest::new(vec![ChatMessage::user("uncached")]);
    let inner = CountingCompletionModel::default();

    inner.complete(request.clone()).await.expect("first call");
    inner.complete(request).await.expect("second call");

    assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_should_bypass_cache_for_boolean_mock_response_and_streaming() {
    let cache = Arc::new(MemoryCache::new());
    let inner = Arc::new(CountingCompletionModel::default());
    let cached = CachedCompletionModel::new(inner.clone(), cache);
    let mut streaming = CompletionRequest::new(vec![ChatMessage::user("streaming")]);
    streaming.stream = Some(true);
    let mut mocked = CompletionRequest::new(vec![ChatMessage::user("mocked")]);
    mocked
        .extra
        .insert("mock_response".to_owned(), Value::Bool(true));

    cached
        .complete(streaming.clone())
        .await
        .expect("stream one");
    cached.complete(streaming).await.expect("stream two");
    cached.complete(mocked.clone()).await.expect("mock one");
    cached.complete(mocked).await.expect("mock two");

    assert_eq!(inner.calls.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn test_should_bypass_cache_for_string_mock_response_without_pollution() {
    let cache = Arc::new(MemoryCache::new());
    let inner = Arc::new(CountingCompletionModel::default());
    let cached = CachedCompletionModel::new(inner.clone(), cache.clone());
    let mut mocked = CompletionRequest::new(vec![ChatMessage::user("same request")]);
    mocked.extra.insert(
        "mock_response".to_owned(),
        Value::String("mock answer".to_owned()),
    );
    let real = CompletionRequest::new(vec![ChatMessage::user("same request")]);
    let key = graphloom_llm::completion_request_cache_key(&real).expect("real key");

    cached.complete(mocked.clone()).await.expect("mock one");
    cached.complete(mocked).await.expect("mock two");
    assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
    assert!(!cache.has(&key).await.expect("cache inspection"));

    cached.complete(real.clone()).await.expect("real miss");
    cached.complete(real).await.expect("real hit");
    assert_eq!(inner.calls.load(Ordering::SeqCst), 3);
    assert!(cache.has(&key).await.expect("cache inspection"));
}

#[tokio::test]
async fn test_should_bypass_embedding_cache_for_truthy_mock_response() {
    let cache = Arc::new(MemoryCache::new());
    let inner = Arc::new(CountingEmbeddingModel::default());
    let cached = CachedEmbeddingModel::new(inner.clone(), cache.clone());
    let mut mocked = EmbeddingRequest::new(vec!["same request".to_owned()]);
    mocked.extra.insert(
        "mock_response".to_owned(),
        Value::String("mock embedding".to_owned()),
    );
    let real = EmbeddingRequest::new(vec!["same request".to_owned()]);
    let key = graphloom_llm::embedding_request_cache_key(&real).expect("real key");

    cached.embed(mocked.clone()).await.expect("mock one");
    cached.embed(mocked).await.expect("mock two");
    assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
    assert!(!cache.has(&key).await.expect("cache inspection"));

    cached.embed(real.clone()).await.expect("real miss");
    cached.embed(real).await.expect("real hit");
    assert_eq!(inner.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn test_should_not_bypass_cache_for_empty_mock_response() {
    let cache = Arc::new(MemoryCache::new());
    let inner = Arc::new(CountingCompletionModel::default());
    let cached = CachedCompletionModel::new(inner.clone(), cache);
    let mut request = CompletionRequest::new(vec![ChatMessage::user("empty mock")]);
    request
        .extra
        .insert("mock_response".to_owned(), Value::String(String::new()));

    cached.complete(request.clone()).await.expect("miss");
    cached.complete(request).await.expect("hit");

    assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn test_should_reject_completion_extra_reserved_key() {
    let mut request = CompletionRequest::new(vec![ChatMessage::user("reserved")]);
    request
        .extra
        .insert("messages".to_owned(), serde_json::json!([]));

    assert!(matches!(
        request.validate(),
        Err(graphloom_llm::LlmError::InvalidRequest { .. })
    ));
}

#[test]
fn test_should_reject_embedding_extra_reserved_key() {
    let mut request = EmbeddingRequest::new(vec!["reserved".to_owned()]);
    request
        .extra
        .insert("input".to_owned(), serde_json::json!([]));

    assert!(matches!(
        request.validate(),
        Err(graphloom_llm::LlmError::InvalidRequest { .. })
    ));
}

fn openai_test_config(api_base: String, model: &str) -> ModelConfig {
    serde_json::from_value(serde_json::json!({
        "model_provider": "openai",
        "model": model,
        "api_key": "test-key",
        "api_base": api_base,
        "max_retries": 1
    }))
    .expect("test model config")
}

fn generated_cache_root(kind: &str) -> (Option<tempfile::TempDir>, std::path::PathBuf) {
    if let Some(root) = std::env::var_os("GRAPHLOOM_GENERATED_CACHE_ROOT") {
        let root = std::path::PathBuf::from(root);
        let root = if root.is_absolute() {
            root
        } else {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(root)
        };
        return (None, root.join(kind));
    }
    let temp = tempfile::tempdir().expect("cache tempdir");
    let root = temp.path().join(kind);
    (Some(temp), root)
}

#[tokio::test]
async fn test_should_write_real_graphrag_completion_cache_file() {
    let (_temp, root) = generated_cache_root("completion");
    let storage = Arc::new(FileStorage::new(&root).expect("file storage"));
    let cache = JsonCache::new(storage)
        .child("extract_graph")
        .expect("cache namespace");
    let inner = Arc::new(CountingCompletionModel::default());
    let model = CachedCompletionModel::new(inner, cache);
    let request: CompletionRequest =
        serde_json::from_str(include_str!("fixtures/graphrag/completion_request.json"))
            .expect("completion request");
    let key = graphloom_llm::completion_request_cache_key(&request).expect("key");

    model.complete(request).await.expect("cache miss");

    let path = root.join("extract_graph").join(&key);
    let stored: Value = serde_json::from_slice(&tokio::fs::read(&path).await.expect("cache file"))
        .expect("cache JSON");
    assert!(stored["result"]["response"].is_object());
    assert_eq!(stored["result"]["metrics"], serde_json::json!({}));
    assert_eq!(
        path.file_name().and_then(std::ffi::OsStr::to_str),
        Some(key.as_str())
    );
}

#[tokio::test]
async fn test_should_write_real_graphrag_embedding_cache_file() {
    let (_temp, root) = generated_cache_root("embedding");
    let storage = Arc::new(FileStorage::new(&root).expect("file storage"));
    let cache = JsonCache::new(storage)
        .child("embed_text")
        .expect("cache namespace");
    let inner = Arc::new(CountingEmbeddingModel::default());
    let model = CachedEmbeddingModel::new(inner, cache);
    let request: EmbeddingRequest =
        serde_json::from_str(include_str!("fixtures/graphrag/embedding_request.json"))
            .expect("embedding request");
    let key = graphloom_llm::embedding_request_cache_key(&request).expect("key");

    model.embed(request).await.expect("cache miss");

    let path = root.join("embed_text").join(&key);
    let stored: Value = serde_json::from_slice(&tokio::fs::read(&path).await.expect("cache file"))
        .expect("cache JSON");
    assert!(stored["result"]["response"].is_object());
    assert_eq!(stored["result"]["metrics"], serde_json::json!({}));
    assert_eq!(
        path.file_name().and_then(std::ffi::OsStr::to_str),
        Some(key.as_str())
    );
}

#[tokio::test]
async fn test_should_send_completion_extra_fields_in_http_body() {
    let server = MockServer::start().await;
    let expected = serde_json::json!({
        "messages": [{"role": "user", "content": "extra"}],
        "model": "chat-test",
        "stream": false,
        "reasoning_effort": "high",
        "provider_options": {"thinking": true}
    });
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_json(expected))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(CompletionResponse::text_for_test("chat-test", "answer")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let model =
        OpenAiCompletionModel::new("chat", openai_test_config(server.uri(), "chat-test"), 1)
            .expect("model");
    let mut request = CompletionRequest::new(vec![ChatMessage::user("extra")]);
    request.extra.insert(
        "reasoning_effort".to_owned(),
        Value::String("high".to_owned()),
    );
    request.extra.insert(
        "provider_options".to_owned(),
        serde_json::json!({"thinking": true}),
    );

    assert_eq!(
        model
            .complete(request)
            .await
            .expect("completion")
            .content()
            .expect("content"),
        "answer"
    );
}

#[tokio::test]
async fn test_should_send_embedding_extra_fields_in_http_body() {
    let server = MockServer::start().await;
    let expected = serde_json::json!({
        "input": ["extra"],
        "encoding_format": "float",
        "model": "embed-test",
        "provider_option": "value"
    });
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .and(body_json(expected))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            EmbeddingResponse::vectors_for_test("embed-test", vec![vec![1.0, 2.0]]),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let model = OpenAiEmbeddingModel::new(
        "embedding",
        openai_test_config(server.uri(), "embed-test"),
        1,
    )
    .expect("model");
    let mut request = EmbeddingRequest::new(vec!["extra".to_owned()]);
    request.encoding_format = Some("float".to_owned());
    request.extra.insert(
        "provider_option".to_owned(),
        Value::String("value".to_owned()),
    );

    let response = model.embed(request).await.expect("embedding");
    assert_eq!(response.data.len(), 1);
}

const CLIENT_ONLY_EXTRA_FIELDS: &[&str] = &[
    "mock_response",
    "api_key",
    "api_base",
    "base_url",
    "api_version",
    "timeout",
    "drop_params",
    "metrics",
    "stream_options",
    "azure_ad_token_provider",
];

#[tokio::test]
async fn test_should_reject_completion_client_only_extra_fields() {
    let server = MockServer::start().await;
    let model =
        OpenAiCompletionModel::new("chat", openai_test_config(server.uri(), "chat-test"), 1)
            .expect("model");

    for field in CLIENT_ONLY_EXTRA_FIELDS {
        let mut request = CompletionRequest::new(vec![ChatMessage::user("client only")]);
        request.extra.insert(
            (*field).to_owned(),
            Value::String("sensitive-value".to_owned()),
        );
        let error = model.complete(request).await.expect_err("field must fail");
        assert!(matches!(
            &error,
            graphloom_llm::LlmError::InvalidRequest { .. }
        ));
        assert!(error.to_string().contains(field));
        assert!(!error.to_string().contains("sensitive-value"));
    }

    assert_eq!(
        server
            .received_requests()
            .await
            .expect("received requests")
            .len(),
        0
    );
}

#[tokio::test]
async fn test_should_reject_embedding_client_only_extra_fields() {
    let server = MockServer::start().await;
    let model = OpenAiEmbeddingModel::new(
        "embedding",
        openai_test_config(server.uri(), "embed-test"),
        1,
    )
    .expect("model");

    for field in CLIENT_ONLY_EXTRA_FIELDS {
        let mut request = EmbeddingRequest::new(vec!["client only".to_owned()]);
        request.extra.insert(
            (*field).to_owned(),
            Value::String("sensitive-value".to_owned()),
        );
        let error = model.embed(request).await.expect_err("field must fail");
        assert!(matches!(
            &error,
            graphloom_llm::LlmError::InvalidRequest { .. }
        ));
        assert!(error.to_string().contains(field));
        assert!(!error.to_string().contains("sensitive-value"));
    }

    assert_eq!(
        server
            .received_requests()
            .await
            .expect("received requests")
            .len(),
        0
    );
}

#[tokio::test]
async fn test_should_not_send_mock_response_to_openai_provider() {
    let server = MockServer::start().await;
    let completion =
        OpenAiCompletionModel::new("chat", openai_test_config(server.uri(), "chat-test"), 1)
            .expect("completion model");
    let embedding = OpenAiEmbeddingModel::new(
        "embedding",
        openai_test_config(server.uri(), "embed-test"),
        1,
    )
    .expect("embedding model");
    let mut completion_request = CompletionRequest::new(vec![ChatMessage::user("mock")]);
    completion_request.extra.insert(
        "mock_response".to_owned(),
        Value::String("mock answer".to_owned()),
    );
    let mut embedding_request = EmbeddingRequest::new(vec!["mock".to_owned()]);
    embedding_request.extra.insert(
        "mock_response".to_owned(),
        Value::String("mock answer".to_owned()),
    );

    assert!(matches!(
        completion.complete(completion_request).await,
        Err(graphloom_llm::LlmError::InvalidRequest { .. })
    ));
    assert!(matches!(
        embedding.embed(embedding_request).await,
        Err(graphloom_llm::LlmError::InvalidRequest { .. })
    ));
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("received requests")
            .len(),
        0
    );
}

#[tokio::test]
async fn test_should_not_expose_api_key_value_in_error() {
    let server = MockServer::start().await;
    let model =
        OpenAiCompletionModel::new("chat", openai_test_config(server.uri(), "chat-test"), 1)
            .expect("model");
    let mut request = CompletionRequest::new(vec![ChatMessage::user("secret")]);
    request.extra.insert(
        "api_key".to_owned(),
        Value::String("sk-never-log-this".to_owned()),
    );

    let error = model
        .complete(request)
        .await
        .expect_err("api_key must fail");
    assert!(error.to_string().contains("api_key"));
    assert!(!error.to_string().contains("sk-never-log-this"));
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("received requests")
            .len(),
        0
    );
}

async fn seed_completion_cache(cache: &MemoryCache, request: &CompletionRequest) {
    let key = graphloom_llm::completion_request_cache_key(request).expect("completion key");
    let payload = CachedModelResult {
        response: CompletionResponse::text_for_test("cached", "cached answer"),
        metrics: Default::default(),
    };
    cache
        .set(
            &key,
            Bytes::from(serde_json::to_vec(&payload).expect("completion payload")),
        )
        .await
        .expect("seed completion cache");
}

async fn seed_embedding_cache(cache: &MemoryCache, request: &EmbeddingRequest) {
    let key = graphloom_llm::embedding_request_cache_key(request).expect("embedding key");
    let payload = CachedModelResult {
        response: EmbeddingResponse::vectors_for_test("cached", vec![vec![1.0, 2.0]]),
        metrics: Default::default(),
    };
    cache
        .set(
            &key,
            Bytes::from(serde_json::to_vec(&payload).expect("embedding payload")),
        )
        .await
        .expect("seed embedding cache");
}

#[tokio::test]
async fn test_should_reject_completion_client_only_fields_before_cache_lookup() {
    let server = MockServer::start().await;
    let inner = Arc::new(
        OpenAiCompletionModel::new("chat", openai_test_config(server.uri(), "chat-test"), 1)
            .expect("model"),
    );
    let cache = Arc::new(MemoryCache::new());
    let normal = CompletionRequest::new(vec![ChatMessage::user("cached request")]);
    seed_completion_cache(&cache, &normal).await;
    let cached = CachedCompletionModel::new(inner.clone(), cache);

    for field in CLIENT_ONLY_EXTRA_FIELDS
        .iter()
        .copied()
        .filter(|field| *field != "mock_response")
    {
        let mut request = normal.clone();
        request.extra.insert(
            field.to_owned(),
            Value::String("sensitive-value".to_owned()),
        );
        let direct_error = inner
            .validate_request(&request)
            .expect_err("provider preflight must fail");
        let error = cached
            .complete(request)
            .await
            .expect_err("field must fail before hit");
        assert!(matches!(
            &error,
            graphloom_llm::LlmError::InvalidRequest { .. }
        ));
        assert!(error.to_string().contains(field));
        assert!(!error.to_string().contains("sensitive-value"));
        assert_eq!(error.to_string(), direct_error.to_string());
    }
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("received requests")
            .len(),
        0
    );
}

#[tokio::test]
async fn test_should_reject_embedding_client_only_fields_before_cache_lookup() {
    let server = MockServer::start().await;
    let inner = Arc::new(
        OpenAiEmbeddingModel::new(
            "embedding",
            openai_test_config(server.uri(), "embed-test"),
            1,
        )
        .expect("model"),
    );
    let cache = Arc::new(MemoryCache::new());
    let normal = EmbeddingRequest::new(vec!["cached request".to_owned()]);
    seed_embedding_cache(&cache, &normal).await;
    let cached = CachedEmbeddingModel::new(inner.clone(), cache);

    for field in CLIENT_ONLY_EXTRA_FIELDS
        .iter()
        .copied()
        .filter(|field| *field != "mock_response")
    {
        let mut request = normal.clone();
        request.extra.insert(
            field.to_owned(),
            Value::String("sensitive-value".to_owned()),
        );
        let direct_error = inner
            .validate_request(&request)
            .expect_err("provider preflight must fail");
        let error = cached
            .embed(request)
            .await
            .expect_err("field must fail before hit");
        assert!(matches!(
            &error,
            graphloom_llm::LlmError::InvalidRequest { .. }
        ));
        assert!(error.to_string().contains(field));
        assert!(!error.to_string().contains("sensitive-value"));
        assert_eq!(error.to_string(), direct_error.to_string());
    }
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("received requests")
            .len(),
        0
    );
}

#[tokio::test]
async fn test_should_reject_mock_response_before_cached_openai_call() {
    let server = MockServer::start().await;
    let inner = Arc::new(
        OpenAiCompletionModel::new("chat", openai_test_config(server.uri(), "chat-test"), 1)
            .expect("model"),
    );
    let cache = Arc::new(MemoryCache::new());
    let normal = CompletionRequest::new(vec![ChatMessage::user("cached request")]);
    seed_completion_cache(&cache, &normal).await;
    let cached = CachedCompletionModel::new(inner, cache.clone());
    let mut request = normal.clone();
    request.extra.insert(
        "mock_response".to_owned(),
        Value::String("mock answer".to_owned()),
    );

    assert!(matches!(
        cached.complete(request).await,
        Err(graphloom_llm::LlmError::InvalidRequest { .. })
    ));
    assert!(
        cache
            .has(&graphloom_llm::completion_request_cache_key(&normal).expect("key"))
            .await
            .expect("cache inspection")
    );
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("received requests")
            .len(),
        0
    );
}

#[tokio::test]
async fn test_should_allow_mock_response_for_mock_model_and_bypass_cache() {
    let cache = Arc::new(MemoryCache::new());
    let normal = CompletionRequest::new(vec![ChatMessage::user("cached request")]);
    seed_completion_cache(&cache, &normal).await;
    let inner = Arc::new(MockCompletionModel::new(
        "mock",
        vec!["mock answer".to_owned()],
    ));
    let cached = CachedCompletionModel::new(inner, cache);
    let mut request = normal;
    request.extra.insert(
        "mock_response".to_owned(),
        Value::String("mock answer".to_owned()),
    );

    let response = cached.complete(request).await.expect("mock response");
    assert_eq!(response.content().expect("content"), "mock answer");
    assert_eq!(response.metadata.cache_status, CacheStatus::NotUsed);
}

#[derive(Debug, Default)]
struct CountingCompletionModel {
    calls: AtomicUsize,
}

#[async_trait]
impl CompletionModel for CountingCompletionModel {
    async fn complete(
        &self,
        _request: CompletionRequest,
    ) -> graphloom_llm::Result<CompletionResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(CompletionResponse::text_for_test("counting", "answer"))
    }
}

#[derive(Debug, Default)]
struct CountingEmbeddingModel {
    calls: AtomicUsize,
}

#[async_trait]
impl EmbeddingModel for CountingEmbeddingModel {
    async fn embed(&self, request: EmbeddingRequest) -> graphloom_llm::Result<EmbeddingResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(EmbeddingResponse::vectors_for_test(
            "counting",
            vec![vec![1.0, 2.0]; request.input.len()],
        ))
    }
}
