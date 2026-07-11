# GraphLoom

GraphLoom is a Rust implementation compatible with Microsoft GraphRAG indexing.
The current compatibility target is Microsoft GraphRAG 3.1.0.

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
  output and pipeline stats. `build_index` always performs full validation,
  builds into an isolated generation, and publishes that generation only after
  every workflow succeeds, so API callers do not need to run CLI validation
  first. Future query and prompt-tuning APIs will
  live under the same API layer.
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
`prompts/community_report.txt`.
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

Dry run loads and parses project configuration, performs the same full preflight
validation as indexing, prints a redacted summary, and shows the workflow list.
It does not call models, create output, create cache, create logs, connect
LanceDB, or modify the current working directory.

## Skip Optional Validation

```bash
graphloom index --root ./demo --skip-validation
```

`--skip-validation` is a CLI-only escape hatch for optional preflight checks. It
skips model, prompt, input-existence, and tokenizer checks that may be
environment-specific, but it does not skip configuration parsing, provider type
checks, workflow name checks, path safety, destructive-output safety, or runtime
provider preflight. Public Rust callers using `graphloom::api::build_index`
always get full validation.

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
demo/output/communities.parquet
demo/output/community_reports.parquet
demo/output/lancedb/
demo/cache/
demo/logs/indexing-engine.log
```

`graphloom index` is a full rebuild. After validation succeeds, it creates an
isolated output generation, resets GraphLoom-managed LanceDB indices inside that
generation, and runs the standard pipeline there. The active output and vector
database are replaced only after the complete pipeline succeeds. A pipeline or
publication failure restores the previous active generation. Cache is
preserved.

Runtime preflight validates provider construction, managed vector schemas, and
write access for output, logs, cache when enabled, and the vector database path
before any generation is created. Vector database paths are resolved through their
existing ancestors and must not use symlink or reparse-point components to
escape the project layout.

Output and vector database locations are destructive paths: output may be
recursively cleared, and managed LanceDB tables may be reset. GraphLoom rejects
symlink or reparse-point components in both paths. Input, cache, and logs may be
symlinks, but GraphLoom follows those links and uses the real filesystem
locations for overlap checks against output and vector database paths.
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

When the LanceDB database is inside the output directory, such as the default
`output/lancedb`, GraphLoom closes the preflight LanceDB connection before
clearing output, then reconnects and recreates the managed vector tables. This
keeps the lifecycle compatible with platforms that do not allow deleting files
held by an open database connection.

The final `text_units` Parquet table follows the GraphRAG 3.1.0 canonical
schema with scalar `document_id: String`. `documents.text_unit_ids` remains a
`List(String)` reverse lookup.

## Current Support

Supported:

- standard indexing
- UTF-8 text input
- file storage
- JSON file cache
- OpenAI-compatible completion and embedding models
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
- Python compatibility tests

The Parquet, settings, prompt, and LanceDB layouts are designed toward
GraphRAG 3.1.0 compatibility. Python interoperability validation is intentionally
left for a later step.

## License

This project is distributed under the terms of MIT.

See [LICENSE](LICENSE.md) for details.
