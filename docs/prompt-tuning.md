# Prompt tuning

GraphLoom implements the Microsoft GraphRAG 3.1.0 prompt-tuning flow through
both the `graphloom prompt-tune` CLI and the public Rust API. It reads the
project's configured text input, selects chunks, calls the configured models,
and generates three indexing prompts:

```text
extract_graph.txt
summarize_descriptions.txt
community_report_graph.txt
```

## CLI

The smallest invocation is:

```bash
graphloom prompt-tune --root ./demo
```

The project must contain a valid `settings.yaml` (or supported equivalent),
input files matching `input.file_pattern`, and the configured default completion
model. The output defaults to `prompts` below the project root.

The selection modes are:

| Mode | Behavior | Main controls |
|---|---|---|
| `top` | Select the first chunks in stable document/chunk order. | `--limit` |
| `random` | Uniformly sample chunks without replacement. | `--limit` |
| `auto` | Reproduce GraphRAG 3.1.0's embedding-centroid selection, including its positional mapping behavior. | `--n-subset-max`, `--k` |

Random is the CLI default. Common options:

```text
--domain <DOMAIN>
--language <LANGUAGE>
--selection-method <top|random|auto>
--limit <N>                    # default: 15
--n-subset-max <N>             # default: 300
--k <N>                        # default: 15
--max-tokens <N>               # default: 2000
--min-examples-required <N>    # default: 2
--chunk-size <TOKENS>          # default: 1200
--overlap <TOKENS>             # default: 100
--[no-]discover-entity-types
--output <DIRECTORY>           # default: prompts
```

`--chunk-size` and `--overlap` are command-specific overrides. For actual chunk
tokenization, GraphRAG 3.1.0 constructs the prompt-tune chunker from the
configured embedding model's tokenizer. GraphLoom does the same for Top,
Random, and Auto, even though only Auto calls the embedding API. If the model
has no known tokenizer mapping, the provider's effective compatible fallback is
used; `chunking.encoding_model` does not determine prompt-tune boundaries.

Auto deliberately preserves a GraphRAG 3.1.0 quirk. It randomly samples up to
`n_subset_max` chunks, embeds that sample, ranks sample positions by Euclidean
distance to the centroid, and then applies the ranked positional indices to the
original unsampled chunk list. It does not return the sampled rows themselves.
This behavior can look surprising, but changing it would break the fixed
compatibility baseline.

When `--domain` or `--language` is omitted, the completion model infers it.
Entity-type discovery is enabled by default and uses GraphRAG's structured JSON
request. `--no-discover-entity-types` skips that one request and uses the
untyped extraction template; it is useful for completion providers that reject
JSON Schema response formats.

The CLI disables prompt-tune caching, matching GraphRAG 3.1.0's default. It
publishes the three files transactionally. Existing targets are backed up while
staged files are renamed, and a failed publication restores the previous set.
Output directories and target files cannot be symlinks or reparse points.

## Rust API

`graphloom::api::generate_indexing_prompts` returns the generated strings and
does not write them:

```rust,no_run
use graphloom::api::{
    DocSelectionType, GenerateIndexingPromptsOptions, generate_indexing_prompts,
};

# async fn example() -> graphloom::Result<()> {
let options = GenerateIndexingPromptsOptions::new("./demo")
    .with_selection_method(DocSelectionType::Auto)
    .with_n_subset_max(300)
    .with_k(15);

let generated = generate_indexing_prompts(&options).await?;
assert!(!generated.extract_graph.is_empty());
assert!(!generated.summarize_descriptions.is_empty());
assert!(!generated.community_report_graph.is_empty());
# Ok(())
# }
```

The API defaults match GraphRAG 3.1.0: Random selection, limit 15, maximum
prompt size 2000 tokens, entity-type discovery enabled, at least two examples,
Auto subset size 300, and `k=15`. When chunk size and overlap are left unset,
the API uses the project configuration.

The API exposes `with_cache(true)` as a GraphLoom extension. The default is
false. Cache opt-in can change model-call behavior relative to the reference
and should not be used when establishing an exact compatibility baseline.

## Compatibility evidence

The offline compatibility gate contains two Top scenarios:

- typed entity discovery;
- an empty discovered type list, which takes the untyped path.

The fixture pins GraphRAG 3.1.0 at
`7fc6607edda3d387d23e52ededbf8a75b6730f97`. It compares complete logical
request bytes and multiplicity, selected chunk identities, replayed response
bytes, and the three output files byte for byte:

```bash
make test-compat
```

An opt-in real-model runner covers all three selection modes:

```bash
make prompt-tune-real-llm-check
make prompt-tune-update-debug
make prompt-tune-random-real-llm
make prompt-tune-auto-real-llm
```

Top uses the selected real completion model. Random and Auto constrain the live
case to one eligible candidate so Python and Rust RNG implementation details
cannot create a false mismatch. Multi-candidate selection invariants are
covered by deterministic offline tests. Auto also calls the configured real
embedding model in both implementations.

The runner performs live completion calls only for GraphRAG, records exact
logical response content, and replays it through GraphLoom. It then compares
request identities and generated prompts exactly. See
[`tests/compat/PROMPT_TUNE_REAL_LLM.md`](../tests/compat/PROMPT_TUNE_REAL_LLM.md)
for configuration, security, and artifact-handling details.

## Compatibility boundary

The acceptance evidence supports GraphRAG 3.1.0-compatible prompt-tune
orchestration, request construction, tokenizer selection, output assembly, and
the Top/Random/Auto modes described above. It does not claim:

- identical random samples across different RNG implementations for an
  unconstrained multi-candidate live run;
- byte-identical arbitrary model output without request-aware replay;
- compatibility with GraphRAG releases other than the pinned 3.1.0 baseline;
- equivalence when the GraphLoom-only API cache extension is enabled.
