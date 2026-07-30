# GraphRAG Compatibility Matrix

Last reviewed: 2026-07-30

Reference baseline: Microsoft GraphRAG 3.1.0, source commit
`7fc6607edda3d387d23e52ededbf8a75b6730f97`

This document is the authoritative compatibility inventory for GraphLoom. It
describes the tested contract, not a claim that the two projects have identical
implementations, APIs, dependencies, or persisted bytes.

## Status definitions

| Status | Meaning |
|---|---|
| Compatible | The stated scope has reproducible cross-implementation or golden evidence. |
| Approved difference | The difference is intentional, documented, and accepted by the compatibility contract. |
| Unsupported | GraphLoom rejects the feature or makes no interoperability claim for it. |
| Pending validation | The implementation or claim needs broader evidence or an explicit compatibility decision. |

Evidence marked **CI gate** runs under `make test-compat`. **Offline** evidence
is deterministic and network-free but may be a narrower Rust or Python test.
**Opt-in live** evidence requires configured external models and is not part of
the default CI gate.

## Compatible

| ID | Surface | Compatible scope | Evidence |
|---|---|---|---|
| C-01 | Standard indexing | Workflow decisions and the seven standard logical Parquet tables for the pinned fixture, including schema, references, hierarchy, requests, cache use, and three managed vector collections. | CI gate: `tests/compat/test_compat.py` |
| C-02 | Incremental update | `previous`, `delta`, no-op behavior, eight merge workflows, final logical tables, request order/contracts, rebased IDs, and final vector manifests. | CI gate: `tests/compat/test_compat.py` |
| C-03 | Cross-producer update | Either implementation can consume the other implementation's seven standard Parquet tables and build consumer-native final vectors. | CI gate: `test_cross_producer_parquet_should_support_bidirectional_native_updates` |
| C-04 | Logical table interoperability | PyArrow, pandas, GraphRAG's typed `DataReader`, and GraphLoom's table reader consume the standard tables at the logical schema level. | CI gate: `tests/compat/test_compat.py`; Offline: `compat_table_reader` |
| C-05 | LLM cache protocol | GraphRAG 3.1.0 `extract_graph` cache reuse plus the separately pinned newer `79ab7c9...` key/envelope protocol golden. | CI gate: `tests/compat/test_compat.py`; `crates/graphloom-llm/tests/cache_compat.rs` |
| C-06 | Logical vector records | Collection names, IDs, dimensions, float32 values, by-ID reads, ANN reads, and bidirectional export/import through the versioned manifest. | CI gate: `tests/compat/test_query_interop.py`; Offline: `compat_vector_manifest` |
| C-07 | Basic Query | CLI contract, context, provider stages, final response, streaming, producer Parquet, and producer logical vectors in both producer/consumer directions. | CI gate: `tests/compat/test_query_compat.py`, `tests/compat/test_query_interop.py` |
| C-08 | Local Query | Local context tables, special-character handling, history, provider stages, vectors, result, and streaming in both directions. | CI gate: same Query suites |
| C-09 | Global and Dynamic Global Query | Static map/reduce and dynamic rating/traversal/map/reduce, with and without public streaming, in both directions; no vector store is required. | CI gate: same Query suites |
| C-10 | DRIFT Query | HyDE, primer, candidate/action invariants, depth, Local actions, reduce, streaming, and a shared deterministic positional trajectory. | CI gate: same Query suites; Offline trajectory golden |
| C-11 | Prompt-tune Top | Typed and untyped prompt generation, logical requests and multiplicity, selected chunk identity, tokenizer boundary, replayed responses, and all three output files byte-for-byte. | CI gate: `tests/compat/test_prompt_tune_top_reference.py` |
| C-12 | Prompt-tune Random | GraphRAG 3.1.0 selection semantics and invariants; real-model orchestration is accepted with one eligible candidate. Exact multi-candidate sample identity is AD-06. | Offline Rust tests; Opt-in live: `make prompt-tune-random-real-llm` |
| C-13 | Prompt-tune Auto | Embedding-model tokenizer, real embedding call, centroid ranking, GraphRAG positional mapping quirk, request replay, and generated prompts. Exact multi-candidate sample identity is AD-06. | Offline Rust tests; Opt-in live: `make prompt-tune-auto-real-llm` |
| C-14 | OpenAI-compatible adapters | Completion and embedding models configured with the GraphRAG `openai`, `deepseek`, and `ollama` provider names, including provider-default API bases and the validated `cl100k_base` fallback cases. Broader tokenizer mapping is V-09. | Rust integration tests; CI gate uses a deterministic OpenAI-compatible server |
| C-15 | Query read-only behavior | Query does not mutate producer Parquet, producer/bridge vectors, prompts, settings, or cache; delayed SSE proves first-delta flushing. | CI gate: `tests/compat/test_query_interop.py` |

## Approved differences

| ID | Surface | Approved difference | Rationale and boundary |
|---|---|---|---|
| AD-01 | Generated identities | Independent runs may produce different UUIDs and opaque Leiden community labels. | Comparisons use semantic identities while still validating every reference and within-producer ordinal. This does not permit missing, duplicated, or wrongly linked records. |
| AD-02 | Parquet bytes | Rust Arrow/Parquet output need not be byte-identical to Python/PyArrow output; column metadata, physical representation, and compression may differ. | Logical schemas, values, nulls, multiplicity, order where behaviorally relevant, and references remain gated. |
| AD-03 | Query request transport | GraphLoom may send explicit `stream=false` or an equivalent JSON response format where GraphRAG omits the field; non-streaming DRIFT may use a non-streaming upstream reduce instead of buffering an internal stream. | Presence-aware differences are locked in the reviewed request contract. Prompt text, model inputs, operation count, and effective public behavior remain compatible. |
| AD-04 | DRIFT random path | Production runs may select different valid follow-up subsets. | Both sides must select the required number of unique incomplete candidates, respect depth and request contracts, and pass the shared deterministic state-transition trajectory. |
| AD-05 | Prompt publication and path safety | `init` and prompt-tune publish managed files transactionally and GraphLoom rejects unsafe symlink/reparse-point or overlapping paths more strictly. | The successful managed-file contents remain compatible; stronger failure atomicity and boundary validation are intentional safety guarantees. |
| AD-06 | Prompt-tune RNG identity | Python and Rust do not promise the same PRNG or shuffle for Random or Auto sampling, so unconstrained multi-candidate runs need not select identical chunks. | Selection semantics and invariants are tested offline. Live acceptance uses one eligible candidate and must not be cited as exact multi-candidate RNG parity. |
| AD-07 | Prompt-tune structured response transport | GraphRAG passes its Python `EntityTypesResponse` type to the model abstraction; GraphLoom sends a provider-neutral request without `response_format` and validates the returned JSON on the client. GraphRAG also passes a disabled relationship JSON-object flag where GraphLoom omits it. | Both differences are checked into each Top fixture manifest. Logical messages, response content, entity types, request multiplicity, and generated prompt bytes remain mandatory. |
| AD-08 | Producer-local ordinals | Independent producers may assign different `human_readable_id` values to documents, text units, and covariates because those ordinals derive from producer-local enumeration. | Cross-producer comparison uses document/text/covariate semantics and remains multiplicity- and reference-sensitive. Retained ordinals must stay stable within one producer across update. |

## Unsupported

| ID | Surface | Current boundary |
|---|---|---|
| U-01 | Other GraphRAG releases | Full workflow compatibility is only claimed for 3.1.0. The newer cache-protocol golden is a narrow exception, not a newer-release compatibility claim. |
| U-02 | Additional model providers and identity | Required models support only the GraphRAG `openai`, `deepseek`, and `ollama` provider names. Azure, Anthropic, other LiteLLM provider names, and Azure managed identity are rejected. Unused model entries do not expand the supported provider set. |
| U-03 | Additional storage, vector, cache, and reporting providers | Remote blob storage, CosmosDB, Azure AI Search, memory/blob cache, and non-file reporting are not implemented. File input/output/reporting, JSON-or-disabled cache, and LanceDB are the supported baseline. |
| U-04 | Additional input formats | CSV, JSON, and JSONL input are not implemented; UTF-8 text files are supported. |
| U-05 | Query result cache | Query result caching is not implemented. LLM cache compatibility is a separate supported protocol. |
| U-06 | Direct cross-version LanceDB directories | Python LanceDB 0.24.3 and Rust lancedb 0.31.0 directories are not opened interchangeably. The supported contract is the logical vector manifest plus consumer-native materialization. |
| U-07 | Arbitrary GraphRAG extensions | Third-party workflows, plugins, notebooks, and private Python API surface are outside the compatibility contract unless added here with evidence. |
| U-08 | Additional model auth and retry strategies | Required models support `api_key` authentication and `exponential_backoff` retry. Other auth and retry strategies are rejected. |
| U-09 | Fast/NLP indexing | GraphRAG's `fast` and `fast-update` methods, `extract_graph_nlp` workflow, and NLP extractor configuration are not implemented. GraphLoom supports the `standard` indexing method and standard incremental update. |

## Pending validation

| ID | Surface | Known gap | Exit condition |
|---|---|---|---|
| V-01 | Full compatibility gate on Windows and macOS | Rust builds and tests run on all three CI platforms, but the Python GraphRAG cross-implementation gate runs only on Ubuntu. | Run the isolated pinned Python gate on Windows and macOS or document a reviewed platform-specific exception. |
| V-02 | Multi-candidate live Random/Auto | Live acceptance intentionally uses one eligible candidate; multi-candidate behavior is deterministic only at the invariant/unit-test layer. | Add a reproducible injected-trajectory or distribution/invariant live acceptance that does not require equal Python/Rust PRNG output. |
| V-03 | Live provider matrix | The deterministic gate validates the OpenAI-compatible protocol, but it does not call every supported provider/model combination. | Record repeatable redacted acceptance for representative OpenAI, DeepSeek, and Ollama completion/embedding configurations. |
| V-04 | Cross-implementation failure injection | Successful and no-op updates are gated; equivalent behavior at each partial table/vector/model failure boundary is not exhaustively compared. | Add fault-injection cases with explicit recovery-state contracts, or approve and document differences. |
| V-05 | Claim-extraction model errors | GraphRAG records a per-document claim extraction error and continues; GraphLoom currently fails the workflow. | Decide the desired contract, implement it if parity is required, and add a cross-implementation error fixture. |
| V-06 | Broader corpus/configuration space | The hard gate is deliberately small and deterministic; it cannot cover every prompt override, token budget, hierarchy shape, duplicate pattern, or corpus scale. | Add reduced regression fixtures whenever a real corpus exposes a new compatibility class; do not rely only on ignored debug artifacts. |
| V-07 | Equal-sized LCC tie-break | With `use_lcc`, GraphRAG keeps the first equal-sized connected component by input order; GraphLoom deliberately keeps the lexically first component for shuffle stability. The current cross-implementation fixture does not exercise this tie. | Add a pinned equal-sized-component fixture, then either reproduce GraphRAG in compatibility mode or promote the lexical rule to an approved difference with explicit downstream invariants. |
| V-08 | Model rate limits and metrics | GraphLoom parses legacy `tokens_per_minute` and `requests_per_minute` fields but does not enforce them; it ignores GraphRAG's nested `rate_limit` and `metrics` settings rather than applying them. | Either implement and cross-validate the middleware semantics, or reject non-default settings explicitly and move the boundary to Unsupported. |
| V-09 | Model-specific tokenizer mapping | GraphLoom honors explicit model `encoding_model` and otherwise uses `cl100k_base`; GraphRAG's LiteLLM tokenizer can select a model-specific encoding for known model IDs. Current cross-implementation prompt-tune evidence covers `text-embedding-3-small`/`cl100k_base` and the `ollama/bge-m3` fallback, not the full LiteLLM catalog. | Add maintained provider/model mappings or another compatible resolver, then gate representative non-`cl100k_base` chunking and token-budget boundaries across indexing, Query, and prompt tuning. |

## Maintenance rules

1. Any change to a supported workflow, request contract, schema, vector record,
   provider, input/storage type, or compatibility test must update this matrix
   in the same change.
2. A row may move to **Compatible** only with a reproducible checked-in test or
   golden. Opt-in live evidence must remain labeled as such.
3. An **Approved difference** must state what may differ and what invariant
   remains mandatory. Test normalization must never be broader than that
   boundary.
4. An **Unsupported** feature must fail explicitly; silent fallback is not
   compatibility.
5. Every **Pending validation** row needs a concrete exit condition. Closing it
   means moving or removing the row, not merely changing prose.
6. When compatibility requires preserving defective, inefficient, or
   surprising GraphRAG behavior, add or update the corresponding item in the
   [compatibility optimization backlog](optimization-opportunities.md).
7. Update the review date and pinned baseline whenever this inventory is
   audited against code and tests.

Implementation and test details are in the
[compatibility test guide](python-compatibility-testing.md),
[prompt-tuning guide](prompt-tuning.md), and
[Query record/replay guide](query-record-replay.md).
