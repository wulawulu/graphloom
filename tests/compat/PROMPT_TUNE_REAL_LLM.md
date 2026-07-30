# Prompt-tune real-LLM compatibility runner

This local-only runner fixes the reference to Microsoft GraphRAG 3.1.0,
commit `7fc6607edda3d387d23e52ededbf8a75b6730f97`. It supports Top, Random, and Auto
selection and performs:

1. GraphRAG live prompt tuning in the selected Top, Random, or Auto mode with
   the configured real models.
   Byte-identical concurrent requests are single-flighted to one provider call.
2. Exact recording of logical message roles/content and raw response content.
3. GraphLoom replay using those same responses, matched by complete message
   bytes rather than FIFO arrival order.
4. Exact comparison of selected request identities and all three output prompt
   files.

By default, the runner uses the committed deterministic fixture inputs and
settings. Input pattern, selection, chunking, and Auto parameters can be
selected explicitly. Completion and embedding model configurations come from
the supplied settings file. GraphLoom never makes a second live completion
call; Auto uses the configured real embedding model in each implementation.

## Validate without network access

```bash
env -u PYTHONPATH PYTHONNOUSERSITE=1 \
  uv run --project tests/compat --locked \
  python tests/compat/prompt_tune_real_llm.py \
  --check \
  --settings /path/to/settings.yaml \
  --env-file /path/to/.env \
  --graphrag-source ../graphrag
```

Validation resolves the configured credential but prints only the provider and
model name. It does not write a run directory.

## Run the acceptance

Build GraphLoom, then explicitly opt into the network call:

```bash
cargo build -p graphloom
TARGET_DIR="$(cargo metadata --no-deps --format-version 1 |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
env -u PYTHONPATH PYTHONNOUSERSITE=1 \
  uv run --project tests/compat --locked \
  python tests/compat/prompt_tune_real_llm.py \
  --run \
  --settings /path/to/settings.yaml \
  --env-file /path/to/.env \
  --graphrag-source ../graphrag \
  --graphloom-bin "$TARGET_DIR/debug/graphloom" \
  --run-name typed-top
```

To exercise the existing update-debug input with a model that does not support
GraphRAG's JSON Schema entity-type request, use the repository target:

```bash
make prompt-tune-real-llm-check
make prompt-tune-update-debug RUN_NAME=update-debug-top
make prompt-tune-random-real-llm
make prompt-tune-auto-real-llm
```

`--no-discover-entity-types` is a real GraphRAG CLI/API mode: it uses the
configured default entity types and skips only the structured-output discovery
request. Use it only when the selected provider rejects JSON Schema response
formats.

GraphRAG 3.1.0 prompt-tune creates its chunker with the configured embedding
model's tokenizer, not `chunking.encoding_model`. GraphLoom follows the same
rule, including for Top and Random selection where the embedding API itself is
not called. For the update-debug `ollama/bge-m3` configuration, both
implementations therefore use LiteLLM's compatible `cl100k_base` fallback while
preserving the fixture's original `chunking.encoding_model: o200k_base`.

Random and Auto use `first.txt` as one 1,000-token candidate chunk. This makes
the selected chunk unique instead of pretending that Python's pandas RNG and
Rust's RNG can share an implementation-independent seed. Multi-candidate
selection algorithms remain covered by deterministic offline tests; the live
acceptances validate each mode's real orchestration, completion requests, and
byte-exact generated prompts. Auto additionally exercises the live
`ollama/bge-m3` embedding path on both sides.

The generic `prompt-tune-real-llm-run` target accepts `SETTINGS`, `ENV_FILE`,
`GRAPHRAG_SOURCE`, `SELECTION_METHOD`, `INPUT_DIR`, `INPUT_FILE_PATTERN`,
`LIMIT`, `CHUNK_SIZE`, `OVERLAP`, `ENCODING_MODEL`, `N_SUBSET_MAX`, `K`, and
`RUN_NAME` overrides. Set `CLEAN=1` only when intentionally replacing that
bounded run-name directory.

The default output is `/prompt_tune_real_llm/typed-top` relative to the
repository. Existing runs are never overwritten. Pass `--clean` to remove only
the selected run-name directory before running again.

The ignored run directory contains the real request/response record, sanitized
temporary projects, comparison evidence, logs, generated prompts, and
`REPORT.md`. It may contain sensitive model content and must not be committed
or uploaded. API keys, authorization headers, and raw provider envelopes are
not recorded.

GraphRAG 3.1.0 reuses one mutable message builder for concurrent relationship
examples, so several logical calls can carry identical request bytes.
Single-flight recording gives those calls one shared live response, preserving
request-aware replay semantics without assigning responses by arrival order.
