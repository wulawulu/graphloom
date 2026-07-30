//! Public option types for prompt tuning.

use std::path::PathBuf;

use clap::ValueEnum;

/// Document selection method for prompt tuning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DocSelectionType {
    /// Select the first `limit` chunks in document order.
    Top,
    /// Select `limit` chunks uniformly at random.
    Random,
    /// Embed a random subset, rank its positions by distance to the centroid,
    /// then select those positions from the original chunk list.
    ///
    /// The final positional mapping intentionally preserves GraphRAG 3.1.0
    /// behavior even though it does not return the sampled rows themselves.
    Auto,
    /// Select all chunks (extension point; not exposed through the GraphRAG
    /// CLI contract).
    All,
}

/// Options for generating indexing prompts.
///
/// Use [`GenerateIndexingPromptsOptions::new`] for a default configuration
/// matching GraphRAG 3.1.0 defaults, then customize with the builder methods.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct GenerateIndexingPromptsOptions {
    /// Project root directory (must contain `settings.yaml`).
    pub root: PathBuf,
    /// Maximum number of chunks to use for prompt tuning.
    pub limit: usize,
    /// Document chunk selection method.
    pub selection_method: DocSelectionType,
    /// Domain description (auto-detected if `None`).
    pub domain: Option<String>,
    /// Primary language of the input documents (auto-detected if `None`).
    pub language: Option<String>,
    /// Maximum token count for the generated entity extraction prompt.
    pub max_tokens: usize,
    /// Whether to auto-discover entity types.
    pub discover_entity_types: bool,
    /// Minimum number of examples included in the entity extraction prompt.
    pub min_examples_required: usize,
    /// Maximum number of document chunks to embed for auto selection.
    pub n_subset_max: usize,
    /// Number of chunks to retain after embedding-based selection.
    pub k: usize,
    /// Enable verbose logging.
    pub verbose: bool,
    /// Override chunk size (uses project config value if `None`).
    pub chunk_size: Option<usize>,
    /// Override chunk overlap (uses project config value if `None`).
    pub overlap: Option<usize>,
    /// Enable LLM cache.  GraphRAG 3.1.0 prompt-tune does not use cache by
    /// default.  This is a GraphLoom extension; enable only when you accept
    /// possible behaviour divergence from the reference.
    pub cache_enabled: bool,
}

impl GenerateIndexingPromptsOptions {
    /// Create options with GraphRAG 3.1.0 defaults for the given project root.
    ///
    /// # Defaults
    ///
    /// | Field | Default |
    /// |---|---|
    /// | `selection_method` | [`DocSelectionType::Random`] |
    /// | `limit` | 15 |
    /// | `max_tokens` | 2000 |
    /// | `discover_entity_types` | `true` |
    /// | `min_examples_required` | 2 |
    /// | `n_subset_max` | 300 |
    /// | `k` | 15 |
    /// | `cache_enabled` | `false` |
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            selection_method: DocSelectionType::Random,
            limit: 15,
            max_tokens: 2000,
            discover_entity_types: true,
            min_examples_required: 2,
            n_subset_max: 300,
            k: 15,
            domain: None,
            language: None,
            verbose: false,
            chunk_size: None,
            overlap: None,
            cache_enabled: false,
        }
    }

    /// Set the chunk selection method.
    #[must_use]
    pub fn with_selection_method(mut self, method: DocSelectionType) -> Self {
        self.selection_method = method;
        self
    }

    /// Set the chunk limit.
    #[must_use]
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set an explicit domain (skips auto-detection).
    #[must_use]
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// Set an explicit language (skips auto-detection).
    #[must_use]
    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Set the maximum tokens for the entity extraction prompt.
    #[must_use]
    pub fn with_max_tokens(mut self, max_tokens: usize) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Set whether to discover entity types.
    #[must_use]
    pub fn with_discover_entity_types(mut self, discover: bool) -> Self {
        self.discover_entity_types = discover;
        self
    }

    /// Set the minimum number of examples in the extraction prompt.
    #[must_use]
    pub fn with_min_examples_required(mut self, min: usize) -> Self {
        self.min_examples_required = min;
        self
    }

    /// Set the subset size for auto selection.
    #[must_use]
    pub fn with_n_subset_max(mut self, n: usize) -> Self {
        self.n_subset_max = n;
        self
    }

    /// Set k for auto selection.
    #[must_use]
    pub fn with_k(mut self, k: usize) -> Self {
        self.k = k;
        self
    }

    /// Override the project chunk size.
    #[must_use]
    pub fn with_chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = Some(size);
        self
    }

    /// Override the project chunk overlap.
    #[must_use]
    pub fn with_overlap(mut self, overlap: usize) -> Self {
        self.overlap = Some(overlap);
        self
    }

    /// Enable or disable LLM cache.
    #[must_use]
    pub fn with_cache(mut self, enabled: bool) -> Self {
        self.cache_enabled = enabled;
        self
    }
}

/// Generated indexing prompts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedIndexingPrompts {
    /// Entity and relationship extraction prompt.
    pub extract_graph: String,
    /// Entity description summarization prompt.
    pub summarize_descriptions: String,
    /// Graph-context community report generation prompt.
    pub community_report_graph: String,
}
