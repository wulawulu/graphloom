//! Document chunk selection methods for prompt tuning.

use std::sync::Arc;

use graphloom_llm::EmbeddingModel;
use rand::{Rng, seq::SliceRandom};

use super::options::DocSelectionType;
use crate::GraphLoomError;

/// A chunk identity, stable across process invocations for deterministic methods.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChunkIdentity {
    /// Document identity (e.g. file path or title).
    pub document_id: Arc<str>,
    /// Chunk text content.
    pub chunk_text: Arc<str>,
    /// Token count.
    pub token_count: usize,
    /// Chunk ordinal within the document (0-based).
    pub chunk_ordinal: usize,
}

const DEFAULT_LIMIT: usize = 15;

/// Select chunks from the full candidate set based on the requested method.
pub(crate) async fn select_chunks(
    chunks: Vec<ChunkIdentity>,
    method: DocSelectionType,
    limit: usize,
    n_subset_max: usize,
    k: usize,
    embedding_model: Option<&(Arc<dyn EmbeddingModel>, usize)>,
) -> Result<Vec<ChunkIdentity>, GraphLoomError> {
    if chunks.is_empty() {
        return Err(GraphLoomError::MissingInput {
            message: "no document chunks available for prompt tuning".to_owned(),
        });
    }

    match method {
        DocSelectionType::All => Ok(chunks),
        DocSelectionType::Top => {
            let effective = apply_limit_fallback(limit, chunks.len());
            Ok(select_top(chunks, effective))
        }
        DocSelectionType::Random => {
            let effective = apply_limit_fallback(limit, chunks.len());
            // GraphRAG: pandas .sample(n=limit) errors if limit > len
            select_random(chunks, effective, &mut rand::thread_rng())
        }
        DocSelectionType::Auto => {
            let (model, batch_size) =
                embedding_model.ok_or_else(|| GraphLoomError::InvalidModel {
                    model_id: "embedding_model".to_owned(),
                    message: "auto selection method requires an embedding model".to_owned(),
                })?;
            // GraphRAG Auto does NOT use limit; selection is controlled by n_subset_max and k
            select_auto(chunks, n_subset_max, k, model.as_ref(), *batch_size).await
        }
    }
}

/// GraphRAG 3.1.0 loader fallback: if `limit <= 0` or `limit > chunk_count`, use 15.
///
/// The public API validates its `PositiveInt` equivalent before reaching this
/// internal rule, so zero remains here only to mirror the loader in isolation.
fn apply_limit_fallback(limit: usize, chunk_count: usize) -> usize {
    if limit == 0 || limit > chunk_count {
        DEFAULT_LIMIT
    } else {
        limit
    }
}

pub(crate) fn select_top(chunks: Vec<ChunkIdentity>, limit: usize) -> Vec<ChunkIdentity> {
    let end = limit.min(chunks.len());
    chunks.into_iter().take(end).collect()
}

/// Random selection. GraphRAG uses `pandas.DataFrame.sample(n=limit)` which
/// raises `ValueError` when `limit > len(chunks)`. We replicate this.
pub(crate) fn select_random(
    chunks: Vec<ChunkIdentity>,
    limit: usize,
    rng: &mut impl Rng,
) -> Result<Vec<ChunkIdentity>, GraphLoomError> {
    if limit > chunks.len() {
        return Err(GraphLoomError::InvalidData {
            workflow: "prompt_tune",
            message: format!(
                "random selection limit ({limit}) exceeds chunk count ({})",
                chunks.len()
            ),
        });
    }
    let mut indices: Vec<usize> = (0..chunks.len()).collect();
    indices.shuffle(rng);
    Ok(indices
        .into_iter()
        .take(limit)
        .map(|i| chunks[i].clone())
        .collect())
}

/// GraphRAG 3.1.0 auto selection.
///
/// 1. Randomly sample up to `n_subset_max` chunks
/// 2. Embed the sampled chunks
/// 3. Compute centroid of embeddings
/// 4. Compute Euclidean distances to centroid
/// 5. Argsort distances, select `k` nearest
/// 6. Apply nearest positional indices to the **original** chunk set
///
/// GraphRAG does NOT short-circuit on `chunks.len() <= limit`.
pub(crate) async fn select_auto(
    chunks: Vec<ChunkIdentity>,
    n_subset_max: usize,
    k: usize,
    embedding_model: &dyn EmbeddingModel,
    batch_size: usize,
) -> Result<Vec<ChunkIdentity>, GraphLoomError> {
    if k == 0 {
        return Err(GraphLoomError::InvalidData {
            workflow: "prompt_tune",
            message: "k must be > 0 for auto selection".to_owned(),
        });
    }
    if n_subset_max == 0 {
        return Err(GraphLoomError::InvalidData {
            workflow: "prompt_tune",
            message: "n_subset_max must be > 0 for auto selection".to_owned(),
        });
    }

    // Sample up to n_subset_max chunks for embedding
    let sample_size = n_subset_max.min(chunks.len());
    let sample_texts: Vec<&str> = {
        let mut rng = rand::thread_rng();
        let mut indices: Vec<usize> = (0..chunks.len()).collect();
        indices.shuffle(&mut rng);
        indices
            .into_iter()
            .take(sample_size)
            .map(|i| chunks[i].chunk_text.as_ref())
            .collect()
    };

    // Embed sampled chunks
    let embeddings = embed_texts(embedding_model, &sample_texts, batch_size).await?;

    // Verify response
    if embeddings.len() != sample_texts.len() {
        return Err(GraphLoomError::InvalidData {
            workflow: "prompt_tune",
            message: format!(
                "embedding returned {} vectors but {} were requested",
                embeddings.len(),
                sample_texts.len()
            ),
        });
    }

    // Centroid and distances
    let center = compute_centroid(&embeddings);
    let distances = distances_to_center(&embeddings, &center);
    let nearest_indices = argsort_f64(&distances);

    // GraphRAG 3.1.0: apply positional indices to ORIGINAL chunk set
    let selected_size = k.min(nearest_indices.len());
    let result: Vec<ChunkIdentity> = nearest_indices
        .into_iter()
        .take(selected_size)
        .filter_map(|idx| chunks.get(idx).cloned())
        .collect();

    Ok(result)
}

async fn embed_texts(
    model: &dyn EmbeddingModel,
    texts: &[&str],
    batch_size: usize,
) -> Result<Vec<Vec<f64>>, GraphLoomError> {
    let mut all_embeddings = Vec::with_capacity(texts.len());

    for batch in texts.chunks(batch_size) {
        let request =
            graphloom_llm::EmbeddingRequest::new(batch.iter().map(|t| t.to_string()).collect());
        let response = model.embed(request).await.map_err(GraphLoomError::Llm)?;

        // Validate response dimensions
        if batch.len() != response.data.len() {
            return Err(GraphLoomError::InvalidData {
                workflow: "prompt_tune",
                message: format!(
                    "embedding batch response mismatch: requested {} got {}",
                    batch.len(),
                    response.data.len()
                ),
            });
        }

        for data in &response.data {
            if data.embedding.is_empty() {
                return Err(GraphLoomError::InvalidData {
                    workflow: "prompt_tune",
                    message: "embedding returned empty vector".to_owned(),
                });
            }
            // Validate no NaN/Infinity
            for &v in &data.embedding {
                if v.is_nan() || v.is_infinite() {
                    return Err(GraphLoomError::InvalidData {
                        workflow: "prompt_tune",
                        message: "embedding contains NaN or Infinity".to_owned(),
                    });
                }
            }
            all_embeddings.push(data.embedding.clone());
        }
    }

    Ok(all_embeddings)
}

fn compute_centroid(embeddings: &[Vec<f64>]) -> Vec<f64> {
    if embeddings.is_empty() {
        return Vec::new();
    }
    let dim = embeddings[0].len();
    let mut center = vec![0.0_f64; dim];
    for embedding in embeddings {
        for (i, &value) in embedding.iter().enumerate() {
            center[i] += value;
        }
    }
    for value in &mut center {
        *value /= embeddings.len() as f64;
    }
    center
}

fn euclidean_distance(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let diff = x - y;
            diff * diff
        })
        .sum::<f64>()
        .sqrt()
}

fn distances_to_center(embeddings: &[Vec<f64>], center: &[f64]) -> Vec<f64> {
    embeddings
        .iter()
        .map(|e| euclidean_distance(e, center))
        .collect()
}

/// Stable argsort matching NumPy `np.argsort`. NaN values sort to end.
fn argsort_f64(values: &[f64]) -> Vec<usize> {
    let mut indexed: Vec<(usize, f64)> = values.iter().copied().enumerate().collect();
    indexed.sort_by(|(i, a), (j, b)| match (a.is_nan(), b.is_nan()) {
        (true, true) => i.cmp(j),
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        (false, false) => a
            .partial_cmp(b)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| i.cmp(j)),
    });
    indexed.into_iter().map(|(i, _)| i).collect()
}

// ---------------------------------------------------------------------------
// GraphRAG Python-format literal handling
// ---------------------------------------------------------------------------

/// Escape braces exactly as GraphRAG does before Python `.format()` assembly.
pub fn escape_python_format_literal(input: &str) -> String {
    input.replace('{', "{{").replace('}', "}}")
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use rand::SeedableRng;
    use rand_xorshift::XorShiftRng;

    use super::*;

    fn chunk(id: &str, text: &str) -> ChunkIdentity {
        ChunkIdentity {
            document_id: Arc::from(id),
            chunk_text: Arc::from(text),
            token_count: text.split_whitespace().count(),
            chunk_ordinal: 0,
        }
    }

    // ---- limit fallback ----

    #[test]
    fn test_limit_0_falls_back_to_15() {
        assert_eq!(apply_limit_fallback(0, 20), 15);
    }

    #[test]
    fn test_limit_gt_chunk_count_falls_back_to_15() {
        assert_eq!(apply_limit_fallback(50, 20), 15);
    }

    #[test]
    fn test_limit_within_range_keeps_value() {
        assert_eq!(apply_limit_fallback(10, 20), 10);
    }

    // ---- top ----

    #[test]
    fn test_top_selects_first_n() {
        let chunks: Vec<_> = (0..10)
            .map(|i| chunk(&format!("doc{i}"), &format!("text {i}")))
            .collect();
        let selected = select_top(chunks, 3);
        assert_eq!(selected.len(), 3);
        assert_eq!(selected[0].document_id.as_ref(), "doc0");
        assert_eq!(selected[1].document_id.as_ref(), "doc1");
        assert_eq!(selected[2].document_id.as_ref(), "doc2");
    }

    #[test]
    fn test_top_limit_exceeds_chunk_count() {
        let chunks: Vec<_> = (0..5)
            .map(|i| chunk(&format!("doc{i}"), &format!("text {i}")))
            .collect();
        let selected = select_top(chunks, 100);
        assert_eq!(selected.len(), 5);
    }

    // ---- random ----

    #[test]
    fn test_random_selects_n_distinct_chunks() {
        let chunks: Vec<_> = (0..20)
            .map(|i| chunk(&format!("doc{i}"), &format!("text {i}")))
            .collect();
        let mut rng = XorShiftRng::seed_from_u64(42);
        let selected = select_random(chunks, 5, &mut rng).expect("random selection");
        assert_eq!(selected.len(), 5);
        let ids: std::collections::HashSet<_> =
            selected.iter().map(|c| c.document_id.as_ref()).collect();
        assert_eq!(ids.len(), 5);
    }

    #[test]
    fn test_random_limit_exceeds_chunk_count_errors() {
        let chunks: Vec<_> = (0..3)
            .map(|i| chunk(&format!("doc{i}"), &format!("text {i}")))
            .collect();
        let mut rng = XorShiftRng::seed_from_u64(42);
        let err = select_random(chunks, 10, &mut rng).expect_err("should error");
        assert!(err.to_string().contains("exceeds chunk count"));
    }

    // ---- centroid / distance ----

    #[test]
    fn test_centroid_computation_f64() {
        let embeddings = vec![vec![1.0, 0.0], vec![0.0, 1.0], vec![1.0, 1.0]];
        let center = compute_centroid(&embeddings);
        let expected_x: f64 = 2.0 / 3.0;
        let expected_y: f64 = 2.0 / 3.0;
        assert!((center[0] - expected_x).abs() < 0.001);
        assert!((center[1] - expected_y).abs() < 0.001);
    }

    #[test]
    fn test_euclidean_distance_f64() {
        let a = vec![0.0, 0.0];
        let b = vec![3.0, 4.0];
        assert!((euclidean_distance(&a, &b) - 5.0).abs() < 0.001);
    }

    // ---- argsort ----

    #[test]
    fn test_argsort_stable_tie_breaking() {
        let values = vec![3.0, 1.0, 2.0, 1.0];
        let indices = argsort_f64(&values);
        assert_eq!(indices[0], 1);
        assert_eq!(indices[1], 3);
        assert_eq!(indices[2], 2);
        assert_eq!(indices[3], 0);
    }

    #[test]
    fn test_argsort_handles_nan() {
        let values = vec![3.0, f64::NAN, 1.0, f64::NAN];
        let indices = argsort_f64(&values);
        assert!(!values[indices[0]].is_nan());
        assert!(!values[indices[1]].is_nan());
        assert!(values[indices[2]].is_nan());
        assert!(values[indices[3]].is_nan());
    }

    // ---- GraphRAG Python-format escaping ----

    #[test]
    fn test_escape_python_format_literal_doubles_every_brace() {
        let cases = [
            (r#"{"name":"Alice"}"#, r#"{{"name":"Alice"}}"#),
            ("{{ user_supplied }}", "{{{{ user_supplied }}}}"),
            ("{% if value %}", "{{% if value %}}"),
            (r#"\frac{a}{b}"#, r#"\frac{{a}}{{b}}"#),
        ];

        for (input, expected) in cases {
            assert_eq!(escape_python_format_literal(input), expected);
        }
    }
}
