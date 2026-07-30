# Study: GraphRAG `extract_graph` output semantics

Status: Done · Owner: graphloom · Date: 2026-07-14 · Source:
`../graphrag` @ `79ab7c9ad586856e82635264c200d8a1eb3c63d9`

> **Decision update (2026-07-16):** The causal analysis and semantic costs in
> this study remain valid, but the earlier decision to preserve GraphLoom's
> stricter semantics by default was superseded by the compatibility-first
> principle. Default mode must reproduce the pinned GraphRAG behavior.
> Preserving `(title, type)` identity may only become an explicit optimized
> mode with independent cache and artifact validation. That mode split is not
> currently implemented.

## Executive summary

GraphLoom and the referenced GraphRAG version consumed the same LLM cache
responses, yet their `entities.parquet` rows originally differed. This was not
a cache miss or different graph extraction. The divergence occurred after
extraction and description summarization: GraphRAG merged entity summaries back
into extracted entities using only `title`.

When one title is recognized as multiple entity types, this merge creates a
Cartesian product. Every summary for the title joins to every typed entity row
for that title, including summaries produced for other types. GraphLoom's
stricter implementation avoided this second join and kept each summary attached
to the typed entity row that produced it.

That stricter design maintained stronger invariants:

- `type`, `description`, `text_unit_ids`, and `frequency` stayed associated;
- each extracted `(title, type)` group produced exactly one row;
- summaries could not cross entity types or increase entity cardinality;
- cardinality grew linearly, rather than quadratically, with type count.

Exact GraphRAG output compatibility nevertheless requires reproducing the
incomplete join key and its inconsistent rows. Cache compatibility and output
behavior remain independently testable, but compatibility-first requires both
in default mode. Stronger entity invariants belong in an explicitly named
optimization mode.

## Scope and comparison method

The comparison used files generated from the same debug corpus:

| File | GraphLoom | GraphRAG |
| --- | ---: | ---: |
| `entities.parquet` rows | 370 | 386 |
| Unique entity titles | 362 | 362 |
| `relationships.parquet` rows | 1,107 | 1,107 |

Exact inspected artifacts:

| File | Size | SHA-256 |
| --- | ---: | --- |
| GraphLoom `entities.parquet` | 78,640 bytes | `ab27b1f0c3bcf2d8aad13da5a9db9cb8a970e3e384c83c9ef09d419a26f4719d` |
| GraphRAG `entities.parquet` | 59,371 bytes | `391c9ed462cbfa500a09ea87a18155246cb796deb0200695f4ff1a32aaefe38b` |
| GraphLoom `relationships.parquet` | 113,046 bytes | `0c862026e2daa2610d942b40763e8f1cb492f3fe3613b43b6d1196d772535d1e` |
| GraphRAG `relationships.parquet` | 79,564 bytes | `ed991be709e2ade4ec0c4778679271239fe9645859805c5b31a3902a1ee858fe` |

The logical comparison decoded Parquet, compared complete row multisets, and
normalized Arrow representation differences. Row order was initially ignored:
GraphLoom's `BTreeMap` sorted keys, while Pandas
`groupby(..., sort=False)` preserves first occurrence. That was sufficient for
relationship multisets but not the full pipeline. `create_communities` keeps
the last duplicate undirected edge, so relationship order changes final edge
weights and communities. GraphLoom now preserves Pandas first-occurrence group
order. Semantic comparison may still ignore column order and compatible Arrow
physical types.

The comparison established:

1. All 1,107 relationship rows matched on `source`, `target`, `description`,
   `text_unit_ids`, and `weight`.
2. Both entity tables contained the same 362 unique titles.
3. Every one of GraphLoom's 370 complete entity rows appeared in GraphRAG.
4. GraphRAG had exactly 16 extra rows, all cross-type summary combinations.

Matching relationships and a complete entity-row subset strongly show that
both workflows consumed the same extraction and summarization results. Different
LLM responses would normally change relationships or base groups, not only
produce these join combinations.

## The eight affected titles

Eight titles had both `GEO` and `ORGANIZATION` groups after extraction:

| Title | GraphLoom rows | GraphRAG rows | Extra GraphRAG rows |
| --- | ---: | ---: | ---: |
| 东平府 | 2 | 4 | 2 |
| 守备府 | 2 | 4 | 2 |
| 张大户家 | 2 | 4 | 2 |
| 报恩寺 | 2 | 4 | 2 |
| 李家 | 2 | 4 | 2 |
| 清河县 | 2 | 4 | 2 |
| 玉皇庙 | 2 | 4 | 2 |
| 王婆茶坊 | 2 | 4 | 2 |
| **Total** | **16** | **32** | **16** |

Other titles had one typed group, for which a title-only join and a one-to-one
join have the same cardinality.

## Concrete example: `守备府`

After LLM extraction, both implementations group by `(title, type)`. The two
groups have different evidence, frequency, and final summaries:

| Title | Type | Correct summary association | Frequency |
| --- | --- | --- | ---: |
| 守备府 | `GEO` | A government residence where Ximen Qing sends a favor | 2 |
| 守备府 | `ORGANIZATION` | A local military institution supplying guards to move a dowry | 1 |

Each group also has distinct `text_unit_ids`. Those IDs and frequencies are
evidence for a typed group, not merely for the title string.

Summarization produces one description per group, but GraphRAG's temporary
summary table retains only:

| Title | Summary |
| --- | --- |
| 守备府 | Government residence and favor-delivery location |
| 守备府 | Local military institution supplying guards |

The distinguishing `type` has been lost. Joining this table back to two
extracted rows by `title` yields:

```text
2 extracted rows × 2 summary rows = 4 output rows
```

| Output type | Attached description | Result |
| --- | --- | --- |
| `GEO` | Location summary | Correct |
| `GEO` | Military-institution summary | Incorrect cross-type combination |
| `ORGANIZATION` | Location summary | Incorrect cross-type combination |
| `ORGANIZATION` | Military-institution summary | Correct |

The cross rows are not harmless duplicates: `type`, `text_unit_ids`, and
`frequency` come from one group while `description` comes from another.

## GraphRAG algorithm and defect

GraphRAG initially groups correctly:

```python
all_entities.groupby(["title", "type"], sort=False)
```

Source:
`../graphrag/packages/graphrag/graphrag/index/operations/extract_graph/extract_graph.py:104-115`.

Summarization iterates every grouped row but drops `type`:

```python
node_descriptions = [
    {
        "title": result.id,
        "description": result.description,
    }
    for result in node_results
]
```

Source:
`../graphrag/packages/graphrag/graphrag/index/operations/summarize_descriptions/summarize_descriptions.py:59-66`.

The workflow then joins only on `title`:

```python
extracted_entities.drop(columns=["description"], inplace=True)
entities = extracted_entities.merge(entity_summaries, on="title", how="left")
```

Source:
`../graphrag/packages/graphrag/graphrag/index/workflows/extract_graph.py:183-184`.

With `k` typed groups for one title, each side has `k` rows and the merge emits
`k²`; only `k` preserve the correct association. The remaining `k²-k` are cross
combinations. Here eight titles had `k=2`, adding 16 rows.

The initial `(title, type)` grouping is necessary because one name can have
different interpretations and evidence. The defect is dropping `type` from
summary identity and then joining on a weaker key.

## GraphLoom's stricter algorithm and invariant

GraphLoom also groups raw rows by `(title, entity_type)`:

```rust
let key = (row.title.clone(), row.entity_type.clone());
```

Source: `crates/graphloom/src/operations/graph/merge.rs:7-22`.

In the stricter summarization design, each asynchronous operation owns a full
`EntityRow` and constructs its result from that same row:

```rust
SummarizedEntityRow {
    title: row.title,
    entity_type: row.entity_type,
    description,
    text_unit_ids: row.text_unit_ids,
    frequency: row.frequency,
}
```

Source: `crates/graphloom/src/operations/graph/summarize.rs:90-106`.

No DataFrame rejoin is needed. Input indices travel with tasks and restore order
after completion, so concurrency cannot mix results
(`summarize.rs:111-120`).

The invariant is:

```text
(title, type) identifies one aggregate group; description, text_unit_ids,
and frequency in that row must all belong to the same group.
```

For `n` aggregate rows, this design always returns `n` summarized rows.

## Why the stricter design is better

### 1. Correct evidence provenance

`text_unit_ids` support a particular typed group. Attaching another type's
summary makes the description unsupported by the IDs stored in that row.

### 2. Classification matches meaning

Type determines interpretation. One name may denote both a place and an
organization; their descriptions must not cross.

### 3. Stable cardinality

Summarization transforms values; it should not expand the graph. The stricter
design preserves group identity and count structurally.

### 4. No quadratic amplification

For `k` types, the stricter design emits `k` rows while GraphRAG emits `k²`.

### 5. No unnecessary lossy round trip

GraphRAG converts typed rows to an untyped summary table and later tries to
recover identity. Keeping typed domain objects throughout is safer.

### 6. Consistency with the upstream data model

GraphRAG itself defines identity as `(title, type)` before summarization. The
stricter design retains that identity afterward.

## Physical Parquet differences

Even equal logical rows can differ physically:

- GraphLoom writes Arrow `string_view` and `large_list<string_view>` while the
  reference uses `string` and `list<string>`.
- Column order differs in the historical artifacts.
- Historical row order differed before GraphLoom adopted first-occurrence
  grouping for downstream community compatibility.
- GraphRAG includes Pandas schema metadata; GraphLoom does not.
- Writers, encodings, and compression produce different sizes and hashes.

Byte identity is therefore neither expected nor a sound semantic criterion.
Distinguish:

1. **Cache compatibility:** identical requests can reuse GraphRAG cache entries.
2. **Logical-field compatibility:** equivalent rows have compatible schemas and
   meaning.
3. **Artifact replication:** exact row order, physical Arrow types, metadata,
   and even anomalous reference behavior.

GraphLoom default mode now reproduces the title-only summary join, closing this
logical `entities.parquet` gap. Physical Parquet replication remains out of
scope. Relationships do not have the same issue because both summarization and
join use the full `("source", "target")` identity.

## Compatibility decision (updated 2026-07-16)

Default compatibility mode reproduces the pinned GraphRAG title-only join,
including cardinality, complete field combinations, left-row order, summary
match order, and `finalize_graph`'s later first-title retention. This accepts an
upstream semantic flaw only to establish a directly comparable baseline.

The earlier one-input-to-one-output `(title, type)` behavior remains a future
optimization. No strategy enum or configuration switch currently exposes it.
Any future mode needs an explicit name, isolated validation, and documented
artifact differences.

Compatibility checks should verify:

- equal relationship multisets after physical normalization;
- GraphRAG-compatible semantic relationship order because keep-last duplicate
  undirected edges make order affect weights;
- equal default entity-row multisets, including the four-row Cartesian
  regression;
- correct typed associations and no cross-type descriptions in optimized mode;
- one compatible cache protocol for both modes, with comparisons labeling the
  selected artifact mode.

Byte-for-byte Parquet replication remains unnecessary. Decoded behavior and
records are the contract; writers, Arrow metadata, and compression are separate
physical-storage concerns.

## Relationship to the cache interoperability study

Cache format and keys are documented in the
[GraphRAG v4 LLM cache interoperability study](study-graphrag-llm-cache.md).
Cache interoperability proves that GraphLoom receives the same model responses;
it cannot prove compatibility of later DataFrame transformations. Default mode
must validate those transformations separately, while semantic correction
belongs to an explicit optimization mode rather than the cache protocol.
