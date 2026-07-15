# Study: GraphRAG indexing lifecycle progress

Status: Done · Owner: graphloom · Date: 2026-07-15 · Source pin: `../graphrag` tag object
`2077c4205add901e6594aced159fca81b7a6d522` (commit
`7fc6607edda3d387d23e52ededbf8a75b6730f97`, GraphRAG v3.1.0)

## Why this study

GraphLoom already reports progress from inside indexing workflows, but configuration loading, model
connectivity validation, runtime/storage initialization, and managed-file publication can still look
idle. This study traces how the GraphRAG v3.1 compatibility baseline exposes those lifecycle phases
so that GraphLoom can distinguish compatible behaviour from behaviour worth improving.

The user-provided sibling checkout is used read-only instead of adding another vendored copy. The
cited files at its current head are unchanged from the pinned v3.1.0 commit.

## Architecture map

```text
GraphRAG CLI                    GraphRAG API                  Pipeline runtime
    │                               │                              │
    │ load_config                   │                              │
    │   (no logger/callback)        │                              │
    │                               │                              │
    │ init_loggers                  │                              │
    │ validate completion models ──▶ external LLM                  │
    │ validate embedding models ───▶ external embedding service    │
    │   (post-call logs only; no live progress callback)           │
    │                               │                              │
    │ build_index ─────────────────▶│ log "Initializing..."        │
    │                               │ create pipeline              │
    │◀──────────────────────────────│ pipeline_start(names)         │
    │   print workflow list         │                              │
    │                               │ run_pipeline ───────────────▶│ create storage/cache
    │                               │                              │ load context.json
    │                               │                              │ (no phase callback)
    │                               │                              │
    │◀──────────────────────────────┼──────────────────────────────│ workflow start/progress/end
    │                               │                              │ direct output writes
    │◀──────────────────────────────│ pipeline_end(results)         │ (no publish phase)
```

GraphRAG therefore has two observability mechanisms with different coverage: standard logging for
coarse lifecycle messages and `WorkflowCallbacks` for pipeline/workflow events. It does not model
preflight or publication as progress-bearing phases.

## Hot path walkthrough

1. `index_cli` loads and parses configuration before `_run_index` initializes logging and before a
   console callback exists. A slow configuration load has no lifecycle output
   (`../graphrag/packages/graphrag/graphrag/cli/index.py:44-53,88-104`).
2. `_run_index` validates every configured completion and embedding model before constructing
   `ConsoleWorkflowCallbacks` and calling the API
   (`../graphrag/packages/graphrag/graphrag/cli/index.py:110,125-130`). Validation performs real,
   sequential connectivity requests: one completion call per completion model and one async
   embedding call per embedding model. It logs only after each successful request and exits on the
   first failure; there is no start event, spinner, per-model counter, or callback
   (`../graphrag/packages/graphrag/graphrag/index/validate_config.py:22-49`).
3. `build_index` logs `Initializing indexing pipeline...`, builds the pipeline, and only then emits
   `pipeline_start`. The adjacent source comment explicitly notes that propagating initialization
   to the CLI would improve clarity but require an API change
   (`../graphrag/packages/graphrag/graphrag/api/index.py:69-76`).
4. The console callback prints the pipeline's workflow names and each workflow's start/end. Its
   `progress` method always prints `completed / total` in place with `\r`; `verbose` controls only
   whether the completed workflow result is dumped, not whether progress is visible
   (`../graphrag/packages/graphrag/graphrag/callbacks/console_workflow_callbacks.py:21-38,44-50`).
5. After `pipeline_start`, `run_pipeline` synchronously constructs input/output storage, the table
   provider, and cache, then asynchronously reads `context.json`. None of these operations emits a
   phase or item callback (`../graphrag/packages/graphrag/graphrag/index/run/run_pipeline.py:39-48`).
   Incremental indexing also copies every previous output table without reporting copy progress
   (`../graphrag/packages/graphrag/graphrag/index/run/run_pipeline.py:55-73,182-188`).
6. The workflow loop is the actual callback boundary: it emits `workflow_start`, lets workflow code
   report item progress, then emits `workflow_end`. Stats and context writes before, between, and
   after workflows have no progress events
   (`../graphrag/packages/graphrag/graphrag/index/run/run_pipeline.py:127-151`).

## Key data structures

`Progress` is a three-field value containing an optional description, total count, and completed
count. `ProgressTicker` increments a local counter, optionally writes a standard log containing the
description and counts, and forwards the value to one callback. It has no phase identifier,
indeterminate state, elapsed time, rate, or nested task model
(`../graphrag/packages/graphrag/graphrag/logger/progress.py:16-70`).

`ConsoleWorkflowCallbacks` is intentionally a simple terminal sink. Pipeline and workflow lifecycle
methods print whole lines, while item progress overwrites one line using a carriage return. It does
not use Rich, `tqdm`, or a persistent multi-progress renderer for indexing
(`../graphrag/packages/graphrag/graphrag/callbacks/console_workflow_callbacks.py:13-50`).

## Key algorithms

The progress calculation is `round(completed / total * 100)`. The percentage is used as the field
width for a dot-padded string beginning with `completed / total`; it is a visual approximation, not
a fixed-width bar. The implementation substitutes totals of one and completed counts of zero when
the optional fields are absent
(`../graphrag/packages/graphrag/graphrag/callbacks/console_workflow_callbacks.py:44-50`).

There is no publication algorithm in the normal indexing lifecycle. Standard indexing constructs a
provider over the configured output storage and gives that provider directly to workflows; state and
stats are also written directly to that storage. Consequently GraphRAG has no isolated-generation
commit/activation phase for a progress UI to represent
(`../graphrag/packages/graphrag/graphrag/index/run/run_pipeline.py:41-48,93-105,160-179`).

## What we will adopt

- Keep pipeline/workflow start/end distinct from count-based item progress. This separation makes
  callback implementations simple and lets library callers choose their own renderer.
- Show count progress in normal CLI mode when a total is known. GraphRAG does not hide its item
  counter behind `verbose` (`../graphrag/packages/graphrag/graphrag/callbacks/console_workflow_callbacks.py:33-38,44-50`).
- Preserve structured logging alongside user-facing progress so non-interactive runs retain an
  auditable lifecycle record.

## What we will avoid

- Do not copy GraphRAG's silent connectivity wait. GraphLoom should surface the model currently
  being checked and an indeterminate running state because these are external network calls.
- Do not treat one `pipeline_start` line as coverage for storage/cache/state initialization. Those
  operations can block independently and should have lifecycle events even when no numeric total is
  available.
- Do not copy the carriage-return dot printer as the progress abstraction. It is only a console
  rendering detail and cannot represent concurrent, nested, or indeterminate work.
- Do not infer publication behaviour from GraphRAG. GraphLoom's `init` command has a real staged
  managed-file publication boundary, while current indexing writes outputs directly; progress
  should describe those actual semantics rather than inventing an index activation phase.

## Open questions

None for the comparison. Renderer selection and the exact GraphLoom lifecycle event schema belong in
the progress feature design rather than this prior-art study.
