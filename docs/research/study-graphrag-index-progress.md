# Study: GraphRAG indexing lifecycle progress

Status: Done · Owner: graphloom · Date: 2026-07-15 · Source pin: tag object
`2077c4205add901e6594aced159fca81b7a6d522` (commit
`7fc6607edda3d387d23e52ededbf8a75b6730f97`, GraphRAG v3.1.0)

## Why this study

GraphLoom already reports progress from inside indexing workflows, but configuration loading, model
connectivity validation, runtime/storage initialization, and managed-file publication can still look
idle. This study traces how the GraphRAG v3.1 compatibility baseline exposes those lifecycle phases
so that GraphLoom can distinguish compatible behaviour from behaviour worth improving.

The research used the user-provided sibling checkout read-only. Every citation below is a durable
upstream permalink pinned to the v3.1.0 commit.

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
   ([index.py, lines 44-53 and 88-104][index-cli]).
2. `_run_index` validates every configured completion and embedding model before constructing
   `ConsoleWorkflowCallbacks` and calling the API
   ([index.py, lines 110 and 125-130][index-cli]). Validation performs real,
   sequential connectivity requests: one completion call per completion model and one async
   embedding call per embedding model. It logs only after each successful request and exits on the
   first failure; there is no start event, spinner, per-model counter, or callback
   ([validate_config.py, lines 22-49][validate-config]).
3. `build_index` logs `Initializing indexing pipeline...`, builds the pipeline, and only then emits
   `pipeline_start`. The adjacent source comment explicitly notes that propagating initialization
   to the CLI would improve clarity but require an API change
   ([API index.py, lines 69-76][api-index]).
4. The console callback prints the pipeline's workflow names and each workflow's start/end. Its
   `progress` method always prints `completed / total` in place with `\r`; `verbose` controls only
   whether the completed workflow result is dumped, not whether progress is visible
   ([console_workflow_callbacks.py, lines 21-50][console-callbacks]).
5. After `pipeline_start`, `run_pipeline` synchronously constructs input/output storage, the table
   provider, and cache, then asynchronously reads `context.json`. None of these operations emits a
   phase or item callback ([run_pipeline.py, lines 39-48][run-pipeline]).
   Incremental indexing also copies every previous output table without reporting copy progress
   ([run_pipeline.py, lines 55-73 and 182-188][run-pipeline]).
6. The workflow loop is the actual callback boundary: it emits `workflow_start`, lets workflow code
   report item progress, then emits `workflow_end`. Stats and context writes before, between, and
   after workflows have no progress events
   ([run_pipeline.py, lines 127-151][run-pipeline]).

## Key data structures

`Progress` is a three-field value containing an optional description, total count, and completed
count. `ProgressTicker` increments a local counter, optionally writes a standard log containing the
description and counts, and forwards the value to one callback. It has no phase identifier,
indeterminate state, elapsed time, rate, or nested task model
([progress.py, lines 16-70][progress]).

`ConsoleWorkflowCallbacks` is intentionally a simple terminal sink. Pipeline and workflow lifecycle
methods print whole lines, while item progress overwrites one line using a carriage return. It does
not use Rich, `tqdm`, or a persistent multi-progress renderer for indexing
([console_workflow_callbacks.py, lines 13-50][console-callbacks]).

## Key algorithms

The progress calculation is `round(completed / total * 100)`. The percentage is used as the field
width for a dot-padded string beginning with `completed / total`; it is a visual approximation, not
a fixed-width bar. The implementation substitutes totals of one and completed counts of zero when
the optional fields are absent
([console_workflow_callbacks.py, lines 44-50][console-callbacks]).

There is no publication algorithm in the normal indexing lifecycle. Standard indexing constructs a
provider over the configured output storage and gives that provider directly to workflows; state and
stats are also written directly to that storage. Consequently GraphRAG has no isolated-generation
commit/activation phase for a progress UI to represent
([run_pipeline.py, lines 41-48, 93-105, and 160-179][run-pipeline]).

## Index-to-query visibility contract

The v3.1.0 CLI relies on command sequencing rather than a storage-level publication protocol:

```text
User / operator              Standard index                    Query CLI
      │                            │                               │
      │ graphrag index ──────────▶ │ direct per-table writes       │
      │                            │ to configured output           │
      │                            │                               │
      │ success exit ◀──────────── │                               │
      │                            │                               │
      │ graphrag query ──────────────────────────────────────────▶ │
      │                            │             sequentially load required tables
      │                            │             then open configured vector stores
      │                            │                               │
      │                            │    no lock / generation / ready-manifest check
```

The documented happy path explicitly tells users to wait until the indexing pipeline is complete
and only then run a query ([getting started, lines 97-127](https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/docs/get_started.md#L97-L127)).
The CLI does not enforce that ordering. Standard indexing creates a provider directly over
`output_storage` and gives it to the workflow context
([run_pipeline.py, lines 39-49 and 92-107](https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/index/run/run_pipeline.py#L39-L107)).
The file backend opens the destination itself for writing rather than publishing a staged sibling
file ([file_storage.py, lines 98-108](https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag-storage/graphrag_storage/file_storage.py#L98-L108)).

On failure, `_run_pipeline` yields an error result without restoring earlier tables
([run_pipeline.py, lines 126-157](https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/index/run/run_pipeline.py#L126-L157)),
and the CLI converts that result into exit status 1
([index.py, lines 124-135](https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/cli/index.py#L124-L135)).
A subsequent query is not blocked by that failed status. Query setup simply opens the configured
output provider and loads each required table in sequence
([query.py, lines 374-397](https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/cli/query.py#L374-L397));
`DataReader` validates each table's shape independently, not as one index generation
([data_reader.py, lines 20-71](https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/data_model/data_reader.py#L20-L71)).
Local search then opens the configured embedding store separately
([query.py, lines 291-318](https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/api/query.py#L291-L318)).

Consequently, GraphRAG has these observable semantics:

- index completes successfully, then query starts: the intended and documented path;
- query starts while index is replacing tables: it can load a mixture of old and new tables;
- index fails after some writes, then query starts: it can consume the partial output if all
  required files still parse;
- once the Query CLI has loaded its DataFrames, later Parquet replacements do not mutate those
  in-memory frames, but vector-store access remains a separate live dependency.

No ready marker, generation pointer, or cross-command lock is consulted on either entry path. This
is not an accidental omission in GraphLoom's port: direct active-output writes reproduce GraphRAG's
normal lifecycle. A stronger online-reindexing guarantee would be a deliberate GraphLoom extension,
not a compatibility requirement.

## What we will adopt

- Keep pipeline/workflow start/end distinct from count-based item progress. This separation makes
  callback implementations simple and lets library callers choose their own renderer.
- Show count progress in normal CLI mode when a total is known. GraphRAG does not hide its item
  counter behind `verbose` ([console_workflow_callbacks.py, lines 33-50][console-callbacks]).
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

[index-cli]: https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/cli/index.py
[validate-config]: https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/index/validate_config.py
[api-index]: https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/api/index.py
[console-callbacks]: https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/callbacks/console_workflow_callbacks.py
[run-pipeline]: https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/index/run/run_pipeline.py
[progress]: https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/logger/progress.py
