# Python GraphRAG compatibility testing

GraphLoom has a reproducible cross-language gate:

```bash
make test-compat
```

The gate builds the real `graphloom` binary and the test-only
`compat_vector_manifest` example, then runs the published `graphrag==3.1.0`
distribution from `tests/compat/.venv`. The fixed GraphRAG source commit is
`7fc6607edda3d387d23e52ededbf8a75b6730f97`; the annotated v3.1.0 tag object is
`2077c4205add901e6594aced159fca81b7a6d522`. Tests reject editable installs and
neighboring source checkouts.

## Compatibility contract

GraphLoom separates compatibility into four layers:

1. **Workflow behavior:** workflow order, prompts, parsing, graph/community
   decisions, and Query orchestration.
2. **Protocol interoperability:** cache namespaces, canonical requests, keys,
   and response envelopes.
3. **Logical data interoperability:** table schemas, references, vector
   collection names, record IDs, dimensions, and vector values.
4. **Physical storage interoperability:** Parquet writer/Arrow representation
   and direct LanceDB on-disk directory access.

The first three layers are hard gates for Phase 1 and Phase 2. The fourth is a
separate storage-hardening boundary. The pinned Python environment uses
LanceDB 0.24.3 and PyArrow 22.0.0; the Rust workspace uses lancedb 0.31.0,
Lance 8.0.0, and Arrow 58.3.0.

The interoperability suite validates logical vector records through a
version-neutral manifest and consumer-native LanceDB materialization. It does
not claim direct on-disk compatibility between the pinned Python and Rust
LanceDB versions.

## Paired producer indexes

A session-scoped fixture starts a deterministic local OpenAI-compatible HTTP
server and creates one GraphLoom index and one GraphRAG index from the same two
checked-in UTF-8 documents. The fixture produces multiple documents and text
units, entities, relationships, claims, multi-level communities, reports, and
all three vector collections. Both indexers use the same four-dimensional,
content-derived embedding responses.

The existing Phase 1 gates remain intact:

- PyArrow and pandas read all seven standard GraphLoom Parquet tables;
- GraphRAG's typed `DataReader` reads those tables;
- column order, logical types, nulls, references, and hierarchy are checked;
- UUID-independent semantic records from both indexes are compared;
- GraphRAG Global Search consumes GraphLoom Parquet;
- GraphLoom consumes an unmodified GraphRAG v3.1.0 `extract_graph` cache;
- the newer pinned cache-protocol fixture remains a separate test.

Producer Parquet is never copied or transformed for Query. Consumer commands
pass the producer's original `output` directory through `--data`.

## Canonical vector manifest

`tests/compat/vector_manifest.py` and
`crates/graphloom-vectors/examples/compat_vector_manifest.rs` implement a
test-only logical vector bridge. It is not part of the user CLI or production
Query runtime.

The stable JSON shape is:

```json
{
  "format_version": 1,
  "collections": [
    {
      "name": "community_full_content",
      "dimension": 4,
      "records": [
        {
          "id": "producer-record-id",
          "vector": [1.0, 0.25, 0.5, 0.75]
        }
      ]
    }
  ]
}
```

The manifest contains exactly these formal collections, in stable order:

```text
community_full_content
entity_description
text_unit_text
```

Records are sorted by ID. Validation rejects unknown/missing collections,
unsupported versions, empty or duplicate IDs, unsorted records, zero or mixed
dimensions, and non-finite values. The shared logical schema is `id` plus the
complete float32 `vector`; timestamp expansion columns are physical store
metadata and are not consumed by Query.

### Producer export

- GraphRAG export uses the pinned Python LanceDB client to read its actual
  producer tables.
- GraphLoom export uses `LanceDbVectorStore` through the public `VectorStore`
  `ids`, `get_by_id`, and ANN methods.

Neither export reads Parquet to reconstruct vectors. Provider recorder offsets
before and after export must be identical.

### Consumer import

- GraphRAG records are imported into a new Rust-native database through
  `LanceDbVectorStore::ensure_index` and `VectorStore::upsert_documents`.
- GraphLoom records are imported into a new Python-native database through
  GraphRAG's `create_vector_store`, `create_index`, and `load_documents`.

Imports refuse a non-empty destination. They preserve every producer ID and
float32 vector bit pattern and neither filter nor add records. Round-trip
exports compare collection names, counts, ID sets, dimensions, and float32
values. By-ID reads and ANN probes cover every collection; a producer record
must appear in top-k with GraphRAG-compatible score `1.0` for its own vector.
Recorder offsets also prove import makes no HTTP request.

Generated entity UUID values legitimately differ between the two independent
index runs. Each manifest is therefore checked strictly against its own
producer Parquet foreign keys; cross-producer entity vectors are compared by
the semantically equivalent entity title. Content-addressed text-unit and
community-report IDs compare directly.

## Consumer views

Each consumer gets a native project view containing its own `settings.yaml` and
prompt syntax. Its `vector_store.db_uri` points to the consumer-native bridge
database. Query combines:

```text
Parquet: producer output, read directly
Vectors: producer logical records in the consumer-native LanceDB version
Prompts: consumer-native project view
```

This is a **logical vector interoperability bridge**, not a physical LanceDB
migration. It does not run an index or embedding workflow.

## Query matrix and recorder

`tests/compat/test_query_interop.py` runs 20 real CLI scenarios:

```text
2 producer/consumer directions
× 4 methods (Basic, Local, Global, DRIFT)
× streaming on/off
= 16

2 directions
× Dynamic Global
× streaming on/off
= 4
```

Four additional direct Global/Dynamic Global smokes point each consumer at a
nonexistent vector URI and verify that no LanceDB directory is opened or
created.

The local provider exposes real `POST /v1/embeddings` and
`POST /v1/chat/completions` routes, JSON completions, structured responses,
SSE with two non-empty deltas and `[DONE]`, usage, model names, batch
embeddings, bounded concurrent handling, and a secret-free request recorder.
Each scenario analyzes only requests after its own offset.

Assertions cover:

- Basic: one query embedding, `Sources` context, and final completion;
- Local: one query embedding plus `Reports`, `Entities`, `Relationships`, and
  `Sources`;
- Global: map and reduce with no embedding;
- Dynamic Global: rating, map, and reduce with no embedding;
- DRIFT: HyDE completion, expanded-query embedding, structured primer, Local
  action, and final reduce.

Only the final provider response enters public streaming output. Two additional
delayed-SSE tests—one for each consumer—hold the server after the first delta,
observe that delta in the real CLI pipe before process completion, then release
the remaining delta and `[DONE]`.

## Read-only proof

Before the Query matrix, tests snapshot producer Parquet file sets, hashes,
sizes, and mtimes; producer vector logical state; both bridge databases; and
consumer settings/prompts. After all queries, the same snapshots must match.
No consumer cache may appear and no vector row may be added, replaced, or
reset. Query-specific log files are allowed.

## Running focused checks

```bash
cargo build -p graphloom
cargo build -p graphloom-vectors --example compat_vector_manifest

TARGET_DIR="$(cargo metadata --no-deps --format-version 1 | \
  python -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"

env -u PYTHONPATH \
GRAPHLOOM_BIN="$TARGET_DIR/debug/graphloom" \
GRAPHLOOM_VECTOR_MANIFEST_BIN="$TARGET_DIR/debug/examples/compat_vector_manifest" \
uv run --project tests/compat --locked \
pytest -vv tests/compat/test_query_interop.py
```

Run all Python/Rust compatibility checks, including Ruff and cache goldens, with
`make test-compat`.

## Known physical storage gap

The suite does not require either LanceDB version to open the other version's
directory. It also does not require Parquet files to be byte-identical.
Physical hardening may later evaluate a jointly supported LanceDB version,
explicit offline conversion tooling, and additional Arrow writer conformance.
No future work should silently migrate a database during Query or make Query
write to producer artifacts.
