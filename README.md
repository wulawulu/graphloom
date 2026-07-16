# GraphLoom

GraphLoom is a Rust implementation compatible with Microsoft GraphRAG indexing.
The core indexing compatibility target is Microsoft GraphRAG 3.1.0. Cache and
selected workflow interoperability are additionally checked against pinned
newer GraphRAG source where the protocol has evolved.

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

- `graphloom::api` exposes programmatic entry points. The current public API is
  `build_index`, which runs standard indexing and returns structured workflow
  output and pipeline stats. `build_index` always performs full validation and
  then runs workflows directly against the configured output, so API callers do
  not need to run CLI validation first. Completed writes are not rolled back if
  a later workflow fails. Future query and prompt-tuning APIs will live under
  the same API layer.
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
`graphloom init` currently generates only the prompt templates used by indexing
workflows. Search and query prompts will be added when their workflows are
implemented.

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
```

`covariates.parquet` is written only when claim extraction is enabled. LanceDB
is prepared only when an active workflow requires vector storage, cache storage
only when cache is enabled, and the log file is a CLI artifact rather than an
output of the library API.

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

Manual cross-implementation testing has exercised all ten standard workflows
listed above. It confirmed identical chunk boundaries under matching chunking
configuration, direct reuse of GraphRAG `extract_graph` cache entries by
GraphLoom, direct reuse of GraphLoom `create_community_reports` cache entries by
GraphRAG, and matching community results. Cache interoperability is a protocol
contract and is tested independently from Parquet or LanceDB storage bytes.

This does not mean the persisted artifacts are byte-for-byte interchangeable.
GraphLoom's Rust Parquet writer and Arrow representation differ from GraphRAG's
Python/PyArrow stack, although the logical table schema and semantics remain the
target. The two projects also use different LanceDB versions, so a LanceDB
directory created by GraphLoom is not currently promised to open in GraphRAG.
Physical Parquet parity and LanceDB on-disk compatibility are known
storage-level gaps; automated Parquet cross-reading coverage is still pending.
These gaps are tracked separately from workflow and cache behavior.

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
- Linux and Windows CI
- tag releases published once by a dedicated Ubuntu release job after Linux and
  Windows build jobs pass

Not yet supported:

- query commands
- update commands
- prompt tuning
- Azure OpenAI or Azure managed identity
- blob storage, CosmosDB, or Azure AI Search
- CSV, JSON, or JSONL input
- an automated Python/GraphRAG cross-implementation compatibility suite

Settings, prompts, workflow behavior, cache protocol, logical Parquet schemas,
and vector record schemas target GraphRAG compatibility. Manual interoperability
has established the current behavioral baseline; automating that corpus and
hardening physical Parquet/LanceDB interoperability remain follow-up work.

## License

This project is distributed under the terms of MIT.
