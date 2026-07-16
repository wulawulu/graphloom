# Python GraphRAG compatibility testing

GraphLoom has a reproducible cross-language compatibility gate:

```bash
make test-compat
```

The command builds the real `graphloom` binary and runs the Python tests inside
the environment locked by `tests/compat/uv.lock`. That environment installs the
published `graphrag==3.1.0` distribution directly through uv. The corresponding
source commit is `7fc6607edda3d387d23e52ededbf8a75b6730f97`; the annotated
v3.1.0 tag object is
`2077c4205add901e659fca81b7a6d522`.

The test harness starts a deterministic local OpenAI-compatible HTTP server and
runs both indexers over the same input, configuration, prompts, completion
responses, and embeddings. It then verifies:

- two checked-in UTF-8 excerpts, originally selected from the public-domain
  `../graphrag/debug/input/金瓶梅.txt` source supplied during fixture preparation,
  produce multiple chunks per document without a runtime sibling-repository dependency;
- PyArrow and pandas can read all seven standard GraphLoom Parquet tables;
- column names, column order, list types, non-empty data, unique IDs, and
  cross-table references are valid;
- GraphRAG's own `DataReader` can load every GraphLoom table;
- normalized GraphLoom and GraphRAG indexes are semantically equivalent;
- the generated graph exercises multi-level communities, non-empty child lists,
  and report-to-community hierarchy references;
- the upstream GraphRAG Global Search CLI can answer a query directly from the
  GraphLoom output tables;
- GraphLoom consumes an unmodified GraphRAG v3.1.0 `extract_graph` cache without
  issuing another extraction request.

Semantic comparison removes generated UUIDs, creation timestamps, periods,
irrelevant row ordering, and opaque numeric community cluster IDs. Community IDs
are remapped through their level and entity membership before comparing level,
parent/children, reports, and generated `Community N` titles. Non-community
`human_readable_id` values, entity and relationship content, weights, degrees,
list references, nulls, and report content remain strict.

Cache compatibility has a second, intentionally separate baseline. Cache key
canonicalization and typed payload round trips are checked against fixtures
generated from GraphRAG commit
`79ab7c9ad586856e82635264c200d8a1eb3c63d9` by
`cargo test -p graphloom-llm --test cache_compat`, which `make test-compat` also
runs. This avoids treating cache behavior from that newer protocol revision as
if it were part of the v3.1.0 core indexing contract.

The gate establishes logical Parquet interoperability, not byte-for-byte file
identity. It does not claim that GraphLoom's LanceDB directory can be opened by
GraphRAG's different LanceDB version. Local Search depends on that vector-store
boundary and remains outside this gate; Global Search is covered because it
consumes the compatible Parquet index directly.
