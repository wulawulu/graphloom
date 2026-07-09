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
  output and pipeline stats. `build_index` validates configuration before it
  clears output or resets managed vector tables, so API callers do not need to
  run CLI validation first. Future query and prompt-tuning APIs will live under
  the same API layer.
- `graphloom::cli` adapts command-line arguments, console output, logging, and
  exit codes to the API. `graphloom index` loads project configuration and calls
  `graphloom::api::build_index`.
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
files, unknown files under the project root, or extra prompt files.

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

`graphloom index` is a full rebuild. After validation succeeds, it clears
validated output storage and resets GraphLoom-managed LanceDB vector indices
before running the standard pipeline. Cache is preserved.

## Current Support

Supported:

- standard indexing
- UTF-8 text input
- file storage
- JSON file cache
- OpenAI-compatible completion and embedding models
- LanceDB vector storage

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
