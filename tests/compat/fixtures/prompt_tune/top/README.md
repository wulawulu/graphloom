# Prompt-tune Top compatibility fixture

This fixture records the real Microsoft GraphRAG 3.1.0 prompt-tune Top flow and
replays its responses through GraphLoom. The reference is fixed to tag
`v3.1.0`, commit `7fc6607edda3d387d23e52ededbf8a75b6730f97`.

Both `typed` and `untyped` use the same two input documents, token chunking
settings, and first three chunks. They differ only in the entity-types response:
the typed scenario returns `person` and `organization`; the untyped scenario
returns an empty array.

Each scenario contains:

- `requests.json`: complete logical message roles and content, plus byte
  lengths and SHA-256 identities.
- `responses.json`: deterministic test responses with byte evidence.
- `selected_chunks.json`: input path, document/chunk ordinal, token count,
  Top order, text bytes, and digest.
- `expected/`: the three byte-exact GraphRAG output prompts.
- `manifest.json`: source provenance, counts, hashes, and the two explicitly
  approved request-contract differences.

GraphRAG 3.1.0 reuses one mutable `CompletionMessagesBuilder` when creating its
relationship coroutines. Once `asyncio.gather` starts them, all three calls see
the same accumulated system-plus-three-user message list. The fixture therefore
records multiplicity three for one exact relationship request identity and
requires the same response bytes for each occurrence. Replay validates every
occurrence without relying on request arrival or task completion order.

## Verify

Build GraphLoom, then run the offline verifier:

```bash
cargo build -p graphloom
TARGET_DIR="$(cargo metadata --no-deps --format-version 1 |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
env -u PYTHONPATH PYTHONNOUSERSITE=1 \
  uv run --project tests/compat --locked \
  python tests/compat/prompt_tune_top_reference.py \
  --verify \
  --graphloom-bin "$TARGET_DIR/debug/graphloom"
```

`make test-compat` runs this verifier. It uses no API key and makes no network
request.

## Update

Update only from a local Git repository containing the fixed release commit.
The script uses `git archive`; it does not checkout, reset, or otherwise modify
the GraphRAG working tree.

```bash
TARGET_DIR="$(cargo metadata --no-deps --format-version 1 |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
GRAPHRAG_API_KEY=compat-test-key \
  env -u PYTHONPATH PYTHONNOUSERSITE=1 \
  uv run --project tests/compat --locked \
  python tests/compat/prompt_tune_top_reference.py \
  --update \
  --graphrag-source ../graphrag \
  --graphloom-bin "$TARGET_DIR/debug/graphloom"
```

Update prints every changed file with its old and new SHA-256, then immediately
runs the same GraphLoom replay verification. Generated JSON uses sorted keys and
one final LF; generated prompt bytes are never trimmed or normalized.
