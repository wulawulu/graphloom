# GraphLoom

GraphLoom is a Rust implementation compatible with Microsoft GraphRAG indexing
and Query behavior. The compatibility baseline is Microsoft GraphRAG 3.1.0.
Cache protocol behavior is additionally checked against pinned newer GraphRAG
source where that protocol has evolved.

## Install

```bash
cargo install --path crates/graphloom
```

For development:

```bash
cargo run -p graphloom -- --help
```

## Architecture

The `graphloom` crate is both the Rust library and the command-line binary.

- `graphloom::api` exposes programmatic indexing and Query entry points.
  `build_index` runs standard indexing and returns structured workflow output
  and pipeline stats. Read-only Query access is available through `query`,
  `query_stream`, `basic_search`, `basic_search_streaming`, `local_search`,
  `local_search_streaming`, `global_search`, `global_search_streaming`,
  `drift_search`, and `drift_search_streaming`. `build_index` always performs
  full validation and then runs workflows directly against the configured
  output, so API callers do not need to run CLI validation first. Completed
  writes are not rolled back if a later workflow fails.
- `graphloom::cli` adapts command-line arguments, console output, logging, and
  exit codes to the API indexing layer. `graphloom index` loads project
  configuration and performs CLI validation before dry-run output or indexing.
- `graphloom init` is a CLI-only project scaffold command. It writes default
  settings, `.env`, `input/`, and prompt files, but is not part of the public
  indexing API. Model names passed through `--model` and `--embedding` are
  written through structured YAML serialization rather than string replacement.

## Initialize a Project

```bash
graphloom init --root ./demo
```

This creates:

```text
demo/
├── settings.yaml
├── .env
├── input/
└── prompts/
```

The default prompts are embedded in the binary and are based on Microsoft
GraphRAG 3.1.0 prompt content under the MIT License.
GraphLoom prompt templates use Tera/Jinja double-brace syntax, such as
`{{ input_text }}`. The canonical community-report prompt is
`prompts/community_report_graph.txt` and `prompts/community_report_text.txt`.
`graphloom init` generates all 13 GraphRAG 3.1.0 managed indexing and Query
prompts. Query prompts include Basic, Local, Global, DRIFT, and question
generation templates.

`init` performs path and symlink preflight before creating directories or
writing managed files. If a project path, `input/`, `prompts/`, or managed file
target is unsafe, the command fails without leaving a partial scaffold.

## API Key

Edit:

```text
demo/.env
```

Set:

```dotenv
GRAPHRAG_API_KEY=<your API key>
```

Do not commit `.env` or API keys to git.

## Input

GraphLoom currently supports UTF-8 text input files:

```bash
echo "Alice works with Bob." > demo/input/document.txt
```

`input.file_pattern` is matched against logical storage paths that use `/` as
the separator, including on Windows. For example, `^subdir/.*\.txt$` matches
`demo/input/subdir/document.txt`.

## Index

```bash
graphloom index --root ./demo
```

This runs the full standard indexing pipeline:

```text
load_input_documents
create_base_text_units
create_final_documents
extract_graph
finalize_graph
extract_covariates
create_communities
create_final_text_units
create_community_reports
generate_text_embeddings
```

## Dry Run

```bash
graphloom index --root ./demo --dry-run
```

Dry run performs the same non-destructive prerequisite validation used before a
real index run, including required model connectivity and storage path
writability, then prints a redacted configuration summary and workflow order. It
sends one short, uncached request to each completion and embedding model required
by the active workflows, which may consume a small number of provider tokens.
It exits before runtime resources are prepared: workflows are not executed,
index output and logs are not created, model responses are not written to cache,
and LanceDB is not created, connected, or modified. Unused configured models are
not contacted. This validates non-destructive prerequisites; it does not promise
that every provider construction or workflow operation will subsequently
succeed.

## Query

All GraphRAG 3.1.0 public Query methods are available:

```bash
graphloom query --root ./demo --method basic "What are the main facts?"

graphloom query --root ./demo --method local "What happened to Alice?"

graphloom query --root ./demo --method global "What are the major themes?"

graphloom query --root ./demo --method global \
  --dynamic-community-selection \
  "What are the major themes?"

graphloom query --root ./demo --method drift \
  --streaming \
  "Explore the causes and consequences."
```

The default method is `global`. `--streaming` flushes final-answer provider
deltas as they arrive and emits one terminal newline. Intermediate context,
Global map/rating output, DRIFT actions, and usage data do not enter stdout.
Without `--verbose`, successful Query stderr is empty; stdout contains only the
answer. Query lifecycle diagnostics are appended to `logs/query.log`.

`--data <path>` overrides only the producer Parquet table location. It is
resolved from the process working directory, while LanceDB continues to use
`vector_store.db_uri` from project settings. Query is strictly read-only: it
does not write Parquet, mutate vector records, create a Query cache, or run an
index workflow.

## Skip Optional Validation

```bash
graphloom index --root ./demo --skip-validation
```

`--skip-validation` is a CLI-only escape hatch for external-resource and
optional preflight checks. It skips model configuration and connectivity,
prompt, input-existence, and tokenizer checks that may be environment-specific.
It also skips storage writability probes and optional vector validation.
It does not skip configuration parsing, workflow name checks, path safety, or
destructive-output safety. With `--dry-run`, it can print a plan for a newly
initialized project before input and credentials are available. Public Rust
callers using `graphloom::api::build_index` always get full validation.

If a future Web or application embedding needs a skip mode, it should use a
separate controlled application API rather than weakening the public
`build_index` default.

## Disable Cache

```bash
graphloom index --root ./demo --no-cache
```

`--no-cache` disables cache for the current run only. Existing cache files are
not deleted.

## Force Init

```bash
graphloom init --root ./demo --force
```

Force init overwrites `settings.yaml`, `.env`, and GraphLoom-managed default
prompt files with matching names. It does not delete `input/`, user input
files, unknown files under the project root, or extra prompt files. Managed
files are fully staged before publication; a publication failure restores every
previous managed file and removes the incomplete scaffold.

## Outputs

Successful indexing writes:

```text
demo/output/documents.parquet
demo/output/text_units.parquet
demo/output/entities.parquet
demo/output/relationships.parquet
demo/output/covariates.parquet
demo/output/communities.parquet
demo/output/community_reports.parquet
demo/output/lancedb/
demo/cache/
demo/logs/indexing-engine.log
demo/logs/query.log
```

`covariates.parquet` is written only when claim extraction is enabled. LanceDB
is prepared only when an active workflow requires vector storage, cache storage
only when cache is enabled, and log files are CLI artifacts rather than outputs
of the library APIs. `query.log` is created only when Query CLI runs.

`graphloom index` runs workflows directly against the configured output, which
matches GraphRAG's normal indexing lifecycle. Each workflow replaces the tables
it owns when those writes occur; the command does not clear the whole output
directory and does not use an isolated generation or a final publication step.
Unrelated files and tables not touched by the configured workflow list remain in
place. If a later workflow fails, outputs already written by completed work are
retained and may represent a partial run. Cache is preserved.

When `generate_text_embeddings` is active, GraphLoom resets its managed LanceDB
tables during runtime preparation before the workflow pipeline starts. Other
LanceDB tables and unrelated files under the database path are not removed.

Unified index validation checks required provider configuration and
connectivity, active vector schemas, and ordinary write access for output, logs,
enabled cache, and the active vector database. Runtime preparation begins only
after validation succeeds and constructs the configured storage, cache, table,
model, and vector providers. Vector database paths are resolved through their
existing ancestors and must not use symlink or reparse-point components to
escape the project layout.

Output and vector database locations are managed write paths: workflow-owned
tables may be replaced, and managed LanceDB tables may be reset. GraphLoom
rejects symlink or reparse-point components in both paths. Input, cache, and
logs may be symlinks, but GraphLoom follows those links and uses the real
filesystem locations for overlap checks against output and vector database
paths.
Output must be disjoint from input, cache, and reporting directories. It may
not equal, contain, or be contained by any of those resolved filesystem paths.
On Windows, path overlap and containment checks use case-insensitive Windows
path semantics, including unresolved suffixes whose capitalization differs.
Unix checks remain case-sensitive, and vector inside-output detection uses the
same platform-specific semantics.

Home-directory safety checks resolve the user home directory from `HOME`,
`USERPROFILE`, or `HOMEDRIVE` plus `HOMEPATH`, in that priority order. Output
and vector database paths may live under a normal project in the home directory,
but they must not equal the home directory or be an ancestor of it.

The final `text_units` Parquet table follows the GraphRAG 3.1.0 canonical
schema with scalar `document_id: String`. `documents.text_unit_ids` remains a
`List(String)` reverse lookup.

## GraphRAG Compatibility Status

GraphLoom's compatibility goal is behavioral first: with equivalent input,
configuration, prompts, and model responses, the standard workflows should make
the same indexing decisions and produce logically equivalent data. This stable
baseline comes before GraphLoom-specific optimizations.

The automated `make test-compat` gate runs GraphLoom and the uv-locked PyPI
GraphRAG 3.1.0 package against one deterministic OpenAI-compatible HTTP server.
It checks all seven standard Parquet tables through PyArrow, pandas, and
GraphRAG's typed `DataReader`, compares UUID-independent index semantics and
references, and verifies GraphLoom can reuse GraphRAG's `extract_graph` cache.
Cache key and payload compatibility for the newer `79ab7c9...` protocol
baseline remains a separate golden-fixture gate.

The same gate runs 20 cross-implementation Query CLI scenarios: each index
producer is consumed by the other implementation for Basic, Local, Global, and
DRIFT in streaming and non-streaming modes, plus Dynamic Global in both modes.
Producer Parquet is read directly. For Basic, Local, and DRIFT, a versioned
canonical vector manifest exports the producer's original logical `id` and
float32 `vector` records and materializes them in the consumer's native LanceDB
version. Export/import performs no embedding requests. Tests verify collection
names, IDs, dimensions, vector values, by-ID/ANN reads, provider stages,
producer context, delayed streaming flush, and read-only snapshots.

This does not mean persisted artifacts are byte-for-byte interchangeable.
GraphLoom's Rust Parquet writer and Arrow representation differ from
GraphRAG's Python/PyArrow stack, though standard tables are cross-read at the
logical schema level. Basic/Local/DRIFT cross-implementation E2E uses the
logical vector manifest to materialize producer records in the consumer-native
LanceDB version without re-embedding. It does not claim direct on-disk
compatibility between Python LanceDB 0.24.3 and Rust lancedb 0.31.0. That
physical storage gap remains a separate hardening item. See the
[compatibility test guide](docs/python-compatibility-testing.md).

One known behavioral difference remains in `extract_graph`: when one title has
multiple entity types, GraphLoom currently preserves `(title, type)` identity
instead of reproducing GraphRAG's title-only summary join. Under the
compatibility-first policy this is an implementation gap in the default mode;
the semantically stricter behavior belongs behind a future explicit
optimization mode. See the
[extract_graph output study](docs/research/study-graphrag-extract-graph-output.md).

## Current Support

Supported:

- standard indexing
- UTF-8 text input
- file storage
- JSON file cache
- OpenAI-compatible completion and embedding models configured with GraphRAG's
  `openai`, `deepseek`, or `ollama` provider names; provider defaults normalize
  DeepSeek and Ollama API bases without GraphLoom-only settings changes
- LanceDB vector storage
- Basic, Local, Global, Dynamic Global, and DRIFT through the Rust API and CLI
- provider-native streaming
- Linux, Windows, and macOS Rust CI
- tag releases published once by a dedicated Ubuntu release job after Linux and
  cross-platform build jobs and the GraphRAG compatibility gate pass

Not yet supported:

- update workflows
- prompt-tuning CLI
- Azure OpenAI or Azure managed identity
- remote blob storage, CosmosDB, or Azure AI Search
- Query result cache
- cross-version LanceDB on-disk interoperability
- CSV, JSON, or JSONL input

Settings, prompts, workflow behavior, cache protocol, logical Parquet schemas,
and vector record schemas target GraphRAG compatibility. Automated and manual
interoperability establish the current behavioral baseline; hardening LanceDB
on-disk interoperability remains follow-up work.

## License

This project is distributed under the terms of MIT.
