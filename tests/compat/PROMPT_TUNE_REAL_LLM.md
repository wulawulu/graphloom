# Prompt-tune real-LLM compatibility runner

This local-only runner fixes the reference to Microsoft GraphRAG 3.1.0,
commit `7fc6607edda3d387d23e52ededbf8a75b6730f97`. It performs:

1. GraphRAG live Top prompt tuning with the selected real completion model.
2. Exact recording of logical message roles/content and raw response content.
3. GraphLoom replay using those same responses, matched by complete message
   bytes rather than FIFO arrival order.
4. Exact comparison of selected request identities and all three output prompt
   files.

The runner uses the committed deterministic fixture inputs and Top settings.
Only the completion model configuration comes from the supplied settings file.
It never makes a second live GraphLoom model call.

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

The default output is `/prompt_tune_real_llm/typed-top` relative to the
repository. Existing runs are never overwritten. Pass `--clean` to remove only
the selected run-name directory before running again.

The ignored run directory contains the real request/response record, sanitized
temporary projects, comparison evidence, logs, generated prompts, and
`REPORT.md`. It may contain sensitive model content and must not be committed
or uploaded. API keys, authorization headers, and raw provider envelopes are
not recorded.

If identical concurrent GraphRAG request bytes receive different model
responses, the runner fails instead of assigning responses using arrival order.
This preserves request-aware replay semantics.
