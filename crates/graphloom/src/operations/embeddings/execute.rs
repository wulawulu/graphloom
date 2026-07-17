//! Embedding execution, batching, and vector reconstitution.

use std::{collections::BTreeSet, sync::Arc};

use futures_util::{StreamExt, stream};
use graphloom_chunking::{ChunkingError, split_text_on_tokens};
use graphloom_llm::{CacheStatus, EmbeddingModel, EmbeddingRequest, EmbeddingResponse, Tokenizer};
use graphloom_vectors::VectorDocument;

use super::types::{EmbeddingBatchOutput, EmbeddingOperationConfig, EmbeddingSourceRow};
use crate::{GraphLoomError, Result};

#[derive(Debug, Clone)]
struct Snippet {
    row_index: usize,
    text: String,
    token_count: usize,
}

#[derive(Debug, Clone)]
struct ApiBatch {
    index: usize,
    snippets: Vec<Snippet>,
    token_count: usize,
}

#[derive(Debug)]
struct ApiBatchResult {
    index: usize,
    row_indices: Vec<usize>,
    embeddings: Vec<Vec<f32>>,
    request_count: usize,
    cache_hits: usize,
    cache_misses: usize,
    input_tokens: usize,
}

/// Embed source rows and reconstruct one vector per original row.
///
/// # Errors
///
/// Returns an error on invalid config, tokenizer/model failures, malformed
/// provider responses, duplicate ids, or invalid vectors.
pub(crate) async fn embed_text_rows(
    rows: &[EmbeddingSourceRow],
    config: &EmbeddingOperationConfig,
    model: Arc<dyn EmbeddingModel>,
    tokenizer: Arc<dyn Tokenizer>,
) -> Result<EmbeddingBatchOutput> {
    validate_config(config)?;
    validate_source_ids(rows, &config.embedding_name)?;
    let mut output = EmbeddingBatchOutput {
        attempted_rows: rows.len(),
        ..EmbeddingBatchOutput::default()
    };

    let snippets = prepare_snippets(rows, config, &tokenizer, &mut output)?;
    if snippets.is_empty() {
        output.skipped_rows = rows.len();
        return Ok(output);
    }

    let batches = build_batches(snippets, config);
    let concurrency = config.concurrency.max(1);
    let mut results = stream::iter(batches.into_iter().map(|batch| {
        let model = Arc::clone(&model);
        async move { execute_batch(batch, config, model).await }
    }))
    .buffer_unordered(concurrency)
    .collect::<Vec<_>>()
    .await
    .into_iter()
    .collect::<Result<Vec<_>>>()?;
    results.sort_by_key(|result| result.index);

    let vectors_by_row = collect_row_vectors(rows.len(), &results)?;
    for (row_index, vectors) in vectors_by_row.into_iter().enumerate() {
        let Some(vector) = reconstitute_vectors(vectors, config, row_index)? else {
            output.skipped_rows = output.skipped_rows.saturating_add(1);
            continue;
        };
        let row = &rows[row_index];
        output.documents.push(VectorDocument {
            id: row.id.clone(),
            vector,
        });
    }

    for result in results {
        output.request_count = output.request_count.saturating_add(result.request_count);
        output.cache_hits = output.cache_hits.saturating_add(result.cache_hits);
        output.cache_misses = output.cache_misses.saturating_add(result.cache_misses);
        output.input_tokens = output.input_tokens.saturating_add(result.input_tokens);
    }
    debug_assert_eq!(
        output.documents.len().saturating_add(output.skipped_rows),
        output.attempted_rows
    );
    Ok(output)
}

fn validate_config(config: &EmbeddingOperationConfig) -> Result<()> {
    if config.batch_size == 0 {
        return Err(invalid_data("batch_size must be greater than zero"));
    }
    if config.batch_max_tokens <= config.chunk_overlap {
        return Err(invalid_data(format!(
            "batch_max_tokens {} must be greater than chunk_overlap {}",
            config.batch_max_tokens, config.chunk_overlap
        )));
    }
    if config.expected_vector_size == 0 {
        return Err(invalid_data(
            "expected_vector_size must be greater than zero",
        ));
    }
    Ok(())
}

fn validate_source_ids(rows: &[EmbeddingSourceRow], embedding_name: &str) -> Result<()> {
    let mut ids = BTreeSet::new();
    for row in rows {
        if row.id.is_empty() {
            return Err(invalid_data(format!(
                "{embedding_name} source row id must not be empty"
            )));
        }
        if !ids.insert(row.id.as_str()) {
            return Err(invalid_data(format!(
                "{embedding_name} duplicate source id {}",
                row.id
            )));
        }
    }
    Ok(())
}

fn prepare_snippets(
    rows: &[EmbeddingSourceRow],
    config: &EmbeddingOperationConfig,
    tokenizer: &Arc<dyn Tokenizer>,
    output: &mut EmbeddingBatchOutput,
) -> Result<Vec<Snippet>> {
    let mut snippets = Vec::new();
    for (row_index, row) in rows.iter().enumerate() {
        if row.text.trim().is_empty() {
            continue;
        }
        let encode_tokenizer = Arc::clone(tokenizer);
        let decode_tokenizer = Arc::clone(tokenizer);
        let chunks = split_text_on_tokens(
            &row.text,
            config.batch_max_tokens,
            config.chunk_overlap,
            &move |text| {
                encode_tokenizer
                    .encode(text)
                    .map_err(|source| ChunkingError::Tokenizer {
                        encoding_model: "embedding".to_owned(),
                        message: source.to_string(),
                    })
            },
            &move |tokens| {
                decode_tokenizer
                    .decode(tokens)
                    .map_err(|source| ChunkingError::Tokenizer {
                        encoding_model: "embedding".to_owned(),
                        message: source.to_string(),
                    })
            },
        )?;
        if chunks.is_empty() {
            continue;
        }
        for chunk in chunks {
            let token_count = tokenizer.count(&chunk)?;
            snippets.push(Snippet {
                row_index,
                text: chunk,
                token_count,
            });
        }
    }
    output.snippet_count = snippets.len();
    Ok(snippets)
}

fn build_batches(snippets: Vec<Snippet>, config: &EmbeddingOperationConfig) -> Vec<ApiBatch> {
    let mut batches = Vec::new();
    let mut current = Vec::new();
    let mut current_tokens = 0usize;

    for snippet in snippets {
        let would_exceed_size = current.len() >= config.batch_size;
        let would_exceed_tokens = !current.is_empty()
            && current_tokens.saturating_add(snippet.token_count) > config.batch_max_tokens;
        if would_exceed_size || would_exceed_tokens {
            batches.push(ApiBatch {
                index: batches.len(),
                snippets: std::mem::take(&mut current),
                token_count: current_tokens,
            });
            current_tokens = 0;
        }
        current_tokens = current_tokens.saturating_add(snippet.token_count);
        current.push(snippet);
    }

    if !current.is_empty() {
        batches.push(ApiBatch {
            index: batches.len(),
            snippets: current,
            token_count: current_tokens,
        });
    }

    batches
}

async fn execute_batch(
    batch: ApiBatch,
    config: &EmbeddingOperationConfig,
    model: Arc<dyn EmbeddingModel>,
) -> Result<ApiBatchResult> {
    let request = EmbeddingRequest::new(
        batch
            .snippets
            .iter()
            .map(|snippet| snippet.text.clone())
            .collect(),
    );
    let mut result = ApiBatchResult {
        index: batch.index,
        row_indices: batch
            .snippets
            .iter()
            .map(|snippet| snippet.row_index)
            .collect(),
        embeddings: Vec::new(),
        request_count: 0,
        cache_hits: 0,
        cache_misses: 0,
        input_tokens: 0,
    };

    let response = model.embed(request.clone()).await?;
    match response.metadata.cache_status {
        CacheStatus::Hit => result.cache_hits = 1,
        CacheStatus::Miss => {
            result.cache_misses = 1;
            result.request_count = 1;
            result.input_tokens = batch.token_count;
        }
        CacheStatus::NotUsed => {
            result.request_count = 1;
            result.input_tokens = batch.token_count;
        }
    }

    result.embeddings = validate_response(config, batch.index, request.input.len(), &response)?;
    Ok(result)
}

fn validate_response(
    config: &EmbeddingOperationConfig,
    batch_index: usize,
    expected_count: usize,
    response: &EmbeddingResponse,
) -> Result<Vec<Vec<f32>>> {
    if response.embeddings().len() != expected_count {
        return Err(invalid_data(format!(
            "{} batch {batch_index} expected {expected_count} embeddings, got {}",
            config.embedding_name,
            response.embeddings().len()
        )));
    }
    let mut response_dimension = None;
    for (index, vector) in response.embeddings().enumerate() {
        if vector.is_empty() {
            return Err(invalid_data(format!(
                "{} batch {batch_index} vector {index} is empty",
                config.embedding_name
            )));
        }
        if vector.iter().any(|value| !value.is_finite()) {
            return Err(invalid_data(format!(
                "{} batch {batch_index} vector {index} contains non-finite value",
                config.embedding_name
            )));
        }
        if vector.iter().any(|value| value.abs() > f64::from(f32::MAX)) {
            return Err(invalid_data(format!(
                "{} batch {batch_index} vector {index} contains a value outside the f32 range",
                config.embedding_name
            )));
        }
        match response_dimension {
            Some(dimension) if dimension != vector.len() => {
                return Err(invalid_data(format!(
                    "{} batch {batch_index} inconsistent dimensions: expected {dimension}, got {}",
                    config.embedding_name,
                    vector.len()
                )));
            }
            None => response_dimension = Some(vector.len()),
            Some(_) => {}
        }
        if vector.len() != config.expected_vector_size {
            return Err(invalid_data(format!(
                "{} batch {batch_index} vector {index} expected dimension {}, got {}",
                config.embedding_name,
                config.expected_vector_size,
                vector.len()
            )));
        }
    }
    #[allow(
        clippy::cast_possible_truncation,
        reason = "provider embeddings are range-checked for finiteness and stored as f32 vectors"
    )]
    let embeddings = response
        .embeddings()
        .map(|embedding| embedding.iter().map(|value| *value as f32).collect())
        .collect();
    Ok(embeddings)
}

fn collect_row_vectors(row_count: usize, results: &[ApiBatchResult]) -> Result<Vec<Vec<Vec<f32>>>> {
    let mut vectors = vec![Vec::new(); row_count];
    for result in results {
        if result.row_indices.len() != result.embeddings.len() {
            return Err(invalid_data(format!(
                "batch {} has {} row indices but {} embeddings",
                result.index,
                result.row_indices.len(),
                result.embeddings.len()
            )));
        }
        for (row_index, vector) in result.row_indices.iter().zip(&result.embeddings) {
            let Some(row_vectors) = vectors.get_mut(*row_index) else {
                return Err(invalid_data(format!(
                    "batch {} produced out-of-range row index {}",
                    result.index, row_index
                )));
            };
            row_vectors.push(vector.clone());
        }
    }
    Ok(vectors)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "normalized finite values are bounded to [-1, 1] before conversion to the f32 store \
              format"
)]
fn reconstitute_vectors(
    vectors: Vec<Vec<f32>>,
    config: &EmbeddingOperationConfig,
    row_index: usize,
) -> Result<Option<Vec<f32>>> {
    match vectors.len() {
        0 => Ok(None),
        1 => Ok(vectors.into_iter().next()),
        count => {
            let mut sums = vec![0.0f64; config.expected_vector_size];
            for vector in vectors {
                if vector.len() != config.expected_vector_size {
                    return Err(invalid_data(format!(
                        "{} row {row_index} expected dimension {}, got {} during reconstitution",
                        config.embedding_name,
                        config.expected_vector_size,
                        vector.len()
                    )));
                }
                for (sum, value) in sums.iter_mut().zip(vector) {
                    *sum += f64::from(value);
                }
            }
            let divisor = f64::from(u32::try_from(count).map_err(|_| {
                invalid_data(format!(
                    "{} row {row_index} has too many vector fragments",
                    config.embedding_name,
                ))
            })?);
            for sum in &mut sums {
                *sum /= divisor;
            }
            let norm = sums.iter().map(|value| value * value).sum::<f64>().sqrt();
            if !norm.is_finite() || norm <= 0.0 {
                return Err(invalid_data(format!(
                    "{} row {row_index} averaged vector has invalid L2 norm {norm}",
                    config.embedding_name
                )));
            }
            let mut normalized = Vec::with_capacity(sums.len());
            for value in sums {
                let normalized_value = value / norm;
                if !normalized_value.is_finite() {
                    return Err(invalid_data(format!(
                        "{} row {row_index} normalized vector contains non-finite value",
                        config.embedding_name
                    )));
                }
                normalized.push(normalized_value as f32);
            }
            Ok(Some(normalized))
        }
    }
}

fn invalid_data(message: impl Into<String>) -> GraphLoomError {
    GraphLoomError::InvalidData {
        workflow: "generate_text_embeddings",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use graphloom_llm::{EmbeddingModel, LlmError};
    use tokio::sync::Notify;

    use super::*;

    #[derive(Debug)]
    struct CharTokenizer;

    impl Tokenizer for CharTokenizer {
        fn encode(&self, text: &str) -> graphloom_llm::Result<Vec<u32>> {
            Ok(text.chars().map(u32::from).collect())
        }

        fn decode(&self, tokens: &[u32]) -> graphloom_llm::Result<String> {
            tokens
                .iter()
                .map(|token| {
                    char::from_u32(*token).ok_or_else(|| LlmError::Tokenizer {
                        encoding_model: "char".to_owned(),
                        message: format!("invalid token {token}"),
                    })
                })
                .collect()
        }
    }

    #[derive(Debug)]
    struct RecordingEmbeddingModel {
        responses: Mutex<Vec<EmbeddingResponse>>,
        requests: Mutex<Vec<EmbeddingRequest>>,
        calls: AtomicUsize,
    }

    impl RecordingEmbeddingModel {
        fn new(responses: Vec<EmbeddingResponse>) -> Self {
            Self {
                responses: Mutex::new(responses),
                requests: Mutex::new(Vec::new()),
                calls: AtomicUsize::new(0),
            }
        }

        fn requests(&self) -> Vec<EmbeddingRequest> {
            self.requests.lock().expect("requests lock").clone()
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EmbeddingModel for RecordingEmbeddingModel {
        async fn embed(
            &self,
            request: EmbeddingRequest,
        ) -> graphloom_llm::Result<EmbeddingResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.requests
                .lock()
                .expect("requests lock")
                .push(request.clone());
            let mut responses = self.responses.lock().expect("responses lock");
            if responses.is_empty() {
                return Ok(EmbeddingResponse::vectors_for_test(
                    "recording",
                    request.input.iter().map(|_| vec![1.0, 0.0]).collect(),
                ));
            }
            Ok(responses.remove(0))
        }
    }

    fn operation_config() -> EmbeddingOperationConfig {
        EmbeddingOperationConfig {
            batch_size: 2,
            batch_max_tokens: 4,
            concurrency: 2,
            chunk_overlap: 0,
            expected_vector_size: 2,
            model_instance_name: "text_embedding".to_owned(),
            embedding_name: "text_unit_text".to_owned(),
        }
    }

    #[tokio::test]
    async fn test_should_skip_empty_text_without_calling_model() {
        let model = Arc::new(RecordingEmbeddingModel::new(Vec::new()));
        let output = embed_text_rows(
            &[EmbeddingSourceRow {
                id: "a".to_owned(),
                text: "   ".to_owned(),
            }],
            &operation_config(),
            model.clone(),
            Arc::new(CharTokenizer),
        )
        .await
        .expect("embedding should succeed");

        assert_eq!(output.attempted_rows, 1);
        assert_eq!(output.skipped_rows, 1);
        assert!(output.documents.is_empty());
        assert_eq!(model.calls(), 0);
    }

    #[tokio::test]
    async fn test_should_respect_batch_size_and_token_limit() {
        let model = Arc::new(RecordingEmbeddingModel::new(Vec::new()));
        let output = embed_text_rows(
            &[
                EmbeddingSourceRow {
                    id: "a".to_owned(),
                    text: "aa".to_owned(),
                },
                EmbeddingSourceRow {
                    id: "b".to_owned(),
                    text: "bb".to_owned(),
                },
                EmbeddingSourceRow {
                    id: "c".to_owned(),
                    text: "cc".to_owned(),
                },
            ],
            &EmbeddingOperationConfig {
                batch_max_tokens: 3,
                ..operation_config()
            },
            model.clone(),
            Arc::new(CharTokenizer),
        )
        .await
        .expect("embedding should succeed");

        assert_eq!(output.documents.len(), 3);
        assert_eq!(output.request_count, 3);
        assert_eq!(
            model
                .requests()
                .iter()
                .map(|request| request.input.len())
                .collect::<Vec<_>>(),
            vec![1, 1, 1]
        );
    }

    #[tokio::test]
    async fn test_should_reject_duplicate_source_ids_before_model_use() {
        for rows in [
            vec![
                EmbeddingSourceRow {
                    id: "duplicate".to_owned(),
                    text: "first".to_owned(),
                },
                EmbeddingSourceRow {
                    id: "duplicate".to_owned(),
                    text: "second".to_owned(),
                },
            ],
            vec![
                EmbeddingSourceRow {
                    id: "duplicate".to_owned(),
                    text: "normal".to_owned(),
                },
                EmbeddingSourceRow {
                    id: "duplicate".to_owned(),
                    text: String::new(),
                },
            ],
            vec![
                EmbeddingSourceRow {
                    id: "duplicate".to_owned(),
                    text: String::new(),
                },
                EmbeddingSourceRow {
                    id: "duplicate".to_owned(),
                    text: "normal".to_owned(),
                },
            ],
            vec![
                EmbeddingSourceRow {
                    id: "duplicate".to_owned(),
                    text: String::new(),
                },
                EmbeddingSourceRow {
                    id: "duplicate".to_owned(),
                    text: "   ".to_owned(),
                },
            ],
        ] {
            let model = Arc::new(RecordingEmbeddingModel::new(Vec::new()));

            let error = embed_text_rows(
                &rows,
                &operation_config(),
                model.clone(),
                Arc::new(CharTokenizer),
            )
            .await
            .expect_err("duplicate id should fail");

            assert!(error.to_string().contains("text_unit_text"));
            assert!(error.to_string().contains("duplicate"));
            assert_eq!(model.calls(), 0);
        }
    }

    #[derive(Debug)]
    struct OutOfOrderEmbeddingModel {
        first_started: Notify,
        second_done: Notify,
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
    }

    impl OutOfOrderEmbeddingModel {
        fn new() -> Self {
            Self {
                first_started: Notify::new(),
                second_done: Notify::new(),
                in_flight: AtomicUsize::new(0),
                max_in_flight: AtomicUsize::new(0),
            }
        }

        fn enter(&self) {
            let current = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(current, Ordering::SeqCst);
        }

        fn exit(&self) {
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
        }

        fn max_in_flight(&self) -> usize {
            self.max_in_flight.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EmbeddingModel for OutOfOrderEmbeddingModel {
        async fn embed(
            &self,
            request: EmbeddingRequest,
        ) -> graphloom_llm::Result<EmbeddingResponse> {
            self.enter();
            let input = request.input.first().cloned().unwrap_or_default();
            let vector = if input == "first" {
                self.first_started.notify_one();
                self.second_done.notified().await;
                vec![1.0, 0.0]
            } else if input == "second" {
                self.first_started.notified().await;
                self.second_done.notify_one();
                vec![0.0, 1.0]
            } else {
                vec![0.5, 0.5]
            };
            self.exit();
            Ok(EmbeddingResponse::vectors_for_test(
                "out-of-order",
                vec![vector],
            ))
        }
    }

    #[tokio::test]
    async fn test_should_restore_row_vector_mapping_after_out_of_order_batches() {
        let model = Arc::new(OutOfOrderEmbeddingModel::new());
        let rows = [
            EmbeddingSourceRow {
                id: "first-id".to_owned(),
                text: "first".to_owned(),
            },
            EmbeddingSourceRow {
                id: "second-id".to_owned(),
                text: "second".to_owned(),
            },
        ];

        let output = embed_text_rows(
            &rows,
            &EmbeddingOperationConfig {
                batch_size: 1,
                batch_max_tokens: 8,
                concurrency: 2,
                ..operation_config()
            },
            model.clone(),
            Arc::new(CharTokenizer),
        )
        .await
        .expect("embedding should succeed");

        assert_eq!(model.max_in_flight(), 2);
        assert_eq!(output.documents[0].id, "first-id");
        assert_eq!(output.documents[0].vector, vec![1.0, 0.0]);
        assert_eq!(output.documents[1].id, "second-id");
        assert_eq!(output.documents[1].vector, vec![0.0, 1.0]);
    }

    #[tokio::test]
    async fn test_should_l2_normalize_average_for_multiple_snippets() {
        let vector =
            reconstitute_vectors(vec![vec![1.0, 0.0], vec![0.0, 1.0]], &operation_config(), 0)
                .expect("reconstitute")
                .expect("vector");

        assert!((vector[0] - 0.707_106_77).abs() < 0.000_001);
        assert!((vector[1] - 0.707_106_77).abs() < 0.000_001);
    }

    #[tokio::test]
    async fn test_should_reject_zero_norm_reconstitution() {
        let error = reconstitute_vectors(
            vec![vec![1.0, 0.0], vec![-1.0, 0.0]],
            &operation_config(),
            0,
        )
        .expect_err("zero norm should fail");

        assert!(error.to_string().contains("invalid L2 norm"));
    }

    #[tokio::test]
    async fn test_should_reject_response_count_dimension_and_non_finite_values() {
        let config = operation_config();
        let count_error = validate_response(
            &config,
            7,
            2,
            &EmbeddingResponse::vectors_for_test("test", vec![vec![1.0, 0.0]]),
        )
        .expect_err("count mismatch should fail");
        assert!(count_error.to_string().contains("batch 7 expected 2"));

        let dimension_error = validate_response(
            &config,
            7,
            1,
            &EmbeddingResponse::vectors_for_test("test", vec![vec![1.0]]),
        )
        .expect_err("dimension mismatch should fail");
        assert!(dimension_error.to_string().contains("expected dimension 2"));

        let finite_error = validate_response(
            &config,
            7,
            1,
            &EmbeddingResponse::vectors_for_test("test", vec![vec![f32::NAN, 0.0]]),
        )
        .expect_err("nan should fail");
        assert!(finite_error.to_string().contains("non-finite"));

        let range_response: EmbeddingResponse = serde_json::from_value(serde_json::json!({
            "object": "list",
            "data": [{"object": "embedding", "index": 0, "embedding": [f64::MAX, 0.0]}],
            "model": "test",
            "usage": {"prompt_tokens": 0, "total_tokens": 0}
        }))
        .expect("f64 embedding response");
        let range_error = validate_response(&config, 7, 1, &range_response)
            .expect_err("out-of-range f64 should fail");
        assert!(range_error.to_string().contains("outside the f32 range"));
    }
}
