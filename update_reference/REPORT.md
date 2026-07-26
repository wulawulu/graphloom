# GraphLoom / Microsoft GraphRAG update reference compatibility report

Date: 2026-07-26
GraphLoom baseline: `64a81e7b6c2bf1e238bfa1ff6e92579dabf3b3bb`
GraphRAG: `79ab7c9ad586856e82635264c200d8a1eb3c63d9`
(`graphrag` 3.1.0)

## Result

**Final classification:** under this configuration and fixture, GraphLoom's
native update and the GraphRAG 3.1.0 initial-index → GraphLoom update handoff
are canonically semantically compatible. The strict
Arrow/UUID/JSON-order/Float32 differences listed below remain.

This result does not claim byte-identical or Arrow-schema-identical storage,
does not cover the reverse GraphLoom → GraphRAG handoff, and does not generalize
to arbitrary models, data sets, or configurations.

The three experiment phases are retained independently:

1. GraphLoom originally rejected a cached completion with present-empty
   `content`.
2. After fixing completion semantics, LL reached community reports and exposed
   an empty-description request-key difference.
3. After preserving empty descriptions exactly like GraphRAG, LL and RG both
   completed all 18 update workflows offline with zero misses.

RR final vs LL final and RR final vs RG final both pass all seven canonical
table comparisons and all three semantic LanceDB comparisons. Both reproduce
GraphRAG's known orphan-reference multisets exactly after stable identity
mapping. Strict storage remains unequal and is reported rather than erased.
All initial Parquet and vector IDs are retained through update in RR, LL, and
RG, with zero retained `human_readable_id` changes.

## Fixture and effective configuration

Local experiment directories (not committed):

- Cache builder:
  `../graphrag/update_reference/cache_builder`
- Frozen cache:
  `../graphrag/update_reference/reference_cache`
- RR:
  `../graphrag/update_reference/lanes/RR`
- LL:
  `update_reference/lanes/LL`
- RG:
  `update_reference/lanes/RG`
- Auxiliary fresh lanes: `RF` under GraphRAG and `GF` under GraphLoom.

`effective_config_gate.py` exits 0 and records the full comparison in
`artifacts/effective-config.json`. It verifies explicit provider/model/auth/
retry/call arguments, concurrency, input, token chunking, all storage paths,
cache, 1024-dimensional LanceDB, embedding batching and collections, graph
extraction, summarization, claims, clustering, community reports, snapshots,
and prompt equivalence.

The stale three-workflow GraphRAG override was removed. Both engines resolve:

```text
index:
load_input_documents, create_base_text_units, create_final_documents,
extract_graph, finalize_graph, extract_covariates, create_communities,
create_final_text_units, create_community_reports, generate_text_embeddings

update:
load_update_documents, the nine standard post-loader workflows,
update_final_documents, update_entities_relationships, update_text_units,
update_covariates, update_communities, update_community_reports,
update_text_embeddings, update_clean_state
```

`extract_claims.enabled` is `true` on both sides. All 13 prompt files are
equivalent after the repository's established Tera-to-`str.format` conversion.
The source prompt bytes cannot be identical because GraphLoom requires Tera
`{{ variable }}` and GraphRAG requires Python `{variable}` syntax; likewise
GraphRAG's environment loader requires `$$` for a literal regex terminator.
The effective-config gate records this syntax-only conversion, and identical
cache keys/hits prove the rendered requests are identical.

Before formal runs, completion and embedding endpoints were changed on both
sides to `127.0.0.1:9`; `NO_PROXY/no_proxy` explicitly covers loopback and
validation was skipped. Thus a miss could not reach a provider.

## Input proof

The retained split is:

| Batch | File | Bytes |
|---|---|---:|
| initial | `金瓶梅_第一至五回.txt` | 116,319 |
| update | `金瓶梅_第六至十回.txt` | 80,161 |

Both engines use byte-identical copies. Concatenating them in that order yields
196,480 bytes and SHA-256
`3b46a1b8c225fc663d85a3fdc9948f069d9ba912120fda2fc1b627f63af16a24`,
exactly matching each original `debug/input/金瓶梅.txt`. The boundary preserves
whole chapters and produces a related but independently chunked update
document.

## GraphRAG-only reference cache construction

Commands:

```text
rtk .venv/bin/graphrag index \
  --root ../graphrag/update_reference/cache_builder --dry-run --cache
exit 0

rtk .venv/bin/graphrag index \
  --root ../graphrag/update_reference/cache_builder --cache
exit 0

rtk .venv/bin/graphrag update \
  --root ../graphrag/update_reference/cache_builder --cache
attempt 1 exit 1 after operator interrupt of a stalled local-proxy connection

rtk .venv/bin/graphrag update \
  --root ../graphrag/update_reference/cache_builder --cache
attempt 2 exit 0
```

The first update attempt stopped after the proxy connection ceased progressing
and retained its valid GraphRAG-written entries. The initial output had not
entered merge workflows. The second official update restarted from the same
initial state, naturally hit those entries, generated the remainder, and
completed all 18 workflows. No GraphLoom process wrote reference cache data.

Cache evolution:

| Point | Entries |
|---|---:|
| old `debug` seed | 837 |
| after builder initial | 981 |
| after interrupted update attempt | 1,329 |
| frozen final | 1,547 |

Frozen cache:

- Size: 59,841,404 bytes
- Manifest SHA-256:
  `840704ca8ea5295903f639e59cc4ed5607ce4ab02b4d15f0f1ddd0208cd1b1fb`
- Namespaces:
  `community_reporting=179`, `extract_claims=164`,
  `extract_graph=164`, `summarize_descriptions=904`,
  `text_embedding=136`

Builder initial metrics:

- Completion: 419 attempted, 296 hits, 123 misses
- Embedding: 32 attempted, 9 hits, 23 misses

Successful update-attempt metrics:

- Completion: 508 attempted, 348 hits, 160 misses
- Embedding: 85 attempted, 25 hits, 60 misses

Across both update attempts, the cache gained exactly 566 unique entries.
GraphRAG writes aggregate request metrics only at normal completion, so the
interrupted attempt has exact before/after entry counts and progress logs but
no final attempted-request counter. This limitation is recorded rather than
inventing a count.

## Pre-fix formal lane commands and cache evidence

```text
# RR initial / update
rtk env NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost \
  .venv/bin/graphrag index --root .../lanes/RR --skip-validation --cache
exit 0
rtk env NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost \
  .venv/bin/graphrag update --root .../lanes/RR --skip-validation --cache
exit 0

# LL initial / update
rtk env NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost \
  RUST_LOG=graphloom_llm=debug,graphloom=info \
  target/debug/graphloom index --root update_reference/lanes/LL \
  --skip-validation
exit 0
rtk env ... target/debug/graphloom update \
  --root update_reference/lanes/LL --skip-validation
exit 1

# RG handoff update
rtk env ... target/debug/graphloom update \
  --root update_reference/lanes/RG --skip-validation
exit 1
```

Cache results:

| Lane/stage | Completion hits/misses | Embedding hits/misses | Manifest |
|---|---:|---:|---|
| RR initial | 418 / 0 | 31 / 0 | unchanged |
| LL initial | 418 / 0 | 31 / 0 | unchanged |
| RR update | 507 / 0 | 84 / 0 | unchanged |
| LL update before failure | 45 total hits / 0 misses | not reached | unchanged |
| RG update before failure | 26 total hits / 0 misses | not reached | unchanged |

Every before/after cache manifest equals the frozen manifest. LL and RG report
no `cache miss` line. The difference in hit count before failure is only
concurrent scheduling.

## Initial comparison: RR vs LL

Both outputs contain all seven tables with identical row counts:

| Table | Rows |
|---|---:|
| documents | 1 |
| text_units | 34 |
| entities | 186 |
| relationships | 571 |
| communities | 46 |
| community_reports | 46 |
| covariates | 582 |

Canonical semantic comparison passes for every table, including stable
cross-table UUID mapping, IDs and references, list fields, text, ranks,
weights, hierarchy, and claims. Duplicate/orphan validation passes on both.

Strict storage comparison does not pass:

- GraphLoom uses Arrow `large_string`/`large_list`; GraphRAG uses
  `string`/`list`.
- `documents.raw_data` is GraphLoom `large_string` versus GraphRAG `null`.
- `community_reports.findings` has large versus regular children and reversed
  struct child order.
- Runtime UUIDs and their list references differ; GraphRAG and GraphLoom
  generate them independently.
- `full_content_json` property serialization order differs.

These strict differences remain in the JSON; none is deleted by semantic
normalization.

All three LanceDB logical table schemas, dimensions, and row counts match.
Strict IDs/vectors differ because entity UUIDs are random and some cached
embedding values land one Float32 rounding step apart. After entity UUIDs are
mapped by title, all IDs align and the maximum absolute vector delta is
`1.3447952251777195e-08`, below the explicit `1e-07` tolerance. The initial
semantic vector gate passes.

## Pre-fix update result and handoff blocker

RR recognizes one new document and completes. Final RR state:

| Table | Rows | Change from initial |
|---|---:|---:|
| documents | 2 | +1 |
| text_units | 58 | +24 |
| entities | 346 | +160 |
| relationships | 1,089 | +518 |
| communities | 94 | +48 |
| community_reports | 93 | +47 |
| covariates | 949 | +367 |

RR vector rows are 93 community reports, 346 entity descriptions, and 58 text
units, all dimension 1024.

The RR initial output copied into RG is byte-identical:
`a6d871c4b208cca3bfd4d6f2c0ee6c64f4b039cded36977955cfb49e066eea51`.
RG successfully reads the GraphRAG documents, text units, Parquet schemas, and
LanceDB and reaches `extract_graph`. Opening the GraphRAG LanceDB adds three
root-level Lance manifest bookkeeping files but changes no copied data file.
Thus the failure is not an inability to read GraphRAG initial output.

LL and RG fail on:

```text
extract_graph/
3383d9acf3268cea81c21a6621978f296b9acef5555c3dfe0e6c603d2298c68d_v4
```

That cache object has exactly one completion choice with `content: ""` and
18,311 characters of provider `reasoning_content`. It is the only completion
entry among all four completion namespaces with absent/empty content.
GraphRAG deliberately uses the empty `content`; it does not substitute
`reasoning_content`.

Relevant code:

- GraphLoom:
  `crates/graphloom-llm/src/types/completion.rs:99-112` filters empty content
  and returns `InvalidResponse`.
- GraphRAG:
  `packages/graphrag-llm/graphrag_llm/types/types.py:97-101` returns
  `message.content or ""`.
- GraphRAG extraction:
  `packages/graphrag/graphrag/index/operations/extract_graph/graph_extractor.py:93-109`
  appends that empty string and continues.

Since LL and RG produce no final update result, `RR final vs LL final` and
`RR final vs RG final` are explicitly marked not performed. Partial outputs
are retained as failure evidence and are not mislabeled as finals.

## Integrity and auxiliary fresh comparison

The RR final self-audit finds GraphRAG's own update output contains:

- 249 orphan `text_units.relationship_ids`
- 119 orphan `communities.entity_ids`
- 79 orphan `communities.relationship_ids`

This is reference behavior of GraphRAG's incremental result, not a GraphLoom
difference. A future successful GraphLoom update should be compared against the
same violation set.

Both auxiliary full-single-document fresh lanes run offline with zero misses:
GraphRAG has 782 completion plus 55 embedding hits; GraphLoom has the same 837
hits. GraphLoom fresh and GraphRAG fresh are canonically equal, including
semantic vectors, with the same strict storage differences as initial.

GraphRAG update is not equal to GraphRAG fresh single-document index:

| Table | RR update | GraphRAG fresh |
|---|---:|---:|
| documents | 2 | 1 |
| text_units | 58 | 57 |
| entities | 346 | 362 |
| relationships | 1,089 | 1,107 |
| communities | 94 | 89 |
| community_reports | 93 | 84 |
| covariates | 949 | 1,010 |

This is expected input-document/chunking plus incremental-algorithm behavior:
the update fixture intentionally preserves two document boundaries, while the
fresh auxiliary lane restores the original one-document token stream. It does
not replace the direct lane comparison.

## Pre-fix classification and minimum fix direction

- Reference cache incomplete: **no** for RR; all 1,040 formal RR initial plus
  update model requests hit.
- Cache request-key contract: **compatible for every LL/RG request reached**.
- Cached completion response contract: **incompatible** for present-empty
  `content`.
- Initial index behavior: **semantically compatible**.
- GraphLoom reading GraphRAG output: **successful through all pre-merge
  workflows**.
- Update merge behavior: **not reached; not classified**.
- Schema/storage: **strictly different, canonically equivalent at initial and
  fresh**.
- Vector store: **strict numerical/UUID differences; canonical comparison
  passes at initial and fresh**.
- Allowed nondeterminism: runtime UUIDs, corresponding reference UUIDs,
  serialized JSON property order, and sub-`1e-07` vector rounding only.

The minimum repair direction at that point was to match GraphRAG's
empty-content semantics:
distinguish a missing choice/message/content from a present `Some("")`, and
allow the latter to flow to GraphRAG-compatible indexing parsers as an empty
extraction. Do **not** silently replace it with `reasoning_content`, because
GraphRAG does not. Add a regression test using the frozen cache shape, then
rerun LL and RG from their preserved initial snapshots. The sections above are
the preserved pre-fix record; the repair and rerun results follow.

## Completion response repair

Changed product behavior:

- `CompletionResponse::content()` rejects only an absent first choice.
- Non-empty content is returned verbatim.
- `Some("")` and `None` both return `Ok("")`.
- `reasoning_content` remains available on the wire/accessor but is never
  selected as normal completion content.
- The error now says `missing choices[0]`; it no longer describes empty content
  as missing.

Regression coverage includes non-empty, empty, null, no-choice, empty plus
reasoning, a minimal faithful v4 cache JSON shape, exact JSON round-trip, and an
empty per-text-unit graph extraction. The full workspace tests also exercise
streaming, structured responses, cache middleware, indexing, and query paths.

Verification commands and results:

```text
rtk cargo +nightly fmt
exit 0
rtk cargo +nightly fmt --check
exit 0
rtk cargo test -p graphloom-llm
exit 0; 103 passed
rtk cargo test -p graphloom-llm --test cache_compat
exit 0; 64 passed
rtk cargo test -p graphloom test_should_accept_empty_graph_extraction_response
exit 0; 1 passed, 444 filtered
rtk cargo build
exit 0
rtk cargo test
exit 0; 621 passed, 4 ignored
rtk cargo clippy -- -D warnings
exit 0
rtk cargo clippy -- -D warnings -W clippy::pedantic
exit 101; 37 pre-existing query-module findings, none in changed files
```

The optional pedantic failures are unrelated baseline findings (documentation
backticks, numeric casts, and function length in query modules). The repository
required non-pedantic clippy gate passes.

## Post-fix LL rerun

The failed partial output/logs were moved, not deleted, under
`artifacts/pre-fix-failure/`. LL was restored from
`lanes/LL/initial-output`; its restored manifest was:

```text
6c6bcb8fe07c7a74bcee4adec2181496e151a8cea312d69bdd116f3355be4f05
matches saved LL initial: true
```

RG remained an exact copy of RR initial:

```text
a6d871c4b208cca3bfd4d6f2c0ee6c64f4b039cded36977955cfb49e066eea51
matches saved RR initial: true
```

The frozen cache before and after the post-fix LL run and after the diagnostic
replay remained:

```text
entries: 1,547
bytes: 59,841,404
manifest:
840704ca8ea5295903f639e59cc4ed5607ce4ab02b4d15f0f1ddd0208cd1b1fb
```

Formal rerun command:

```text
rtk env NO_PROXY=127.0.0.1,localhost \
  no_proxy=127.0.0.1,localhost \
  RUST_LOG=graphloom_llm=debug,graphloom=info \
  target/debug/graphloom update \
  --root update_reference/lanes/LL \
  --skip-validation
exit 1 after deliberate interrupt immediately following the first cache miss
```

Post-fix LL evidence:

| Item | Result |
|---|---:|
| completion hits before stop | 348 |
| completion misses | 1 |
| embedding hits/misses | 0 / 0 (not reached) |
| old empty-content key hit | yes |
| real provider response | none |
| cache manifest changed | no |
| workflows completed | 8, through `create_final_text_units` |
| merge workflows reached | no |

The new miss is:

```text
workflow: create_community_reports
namespace: community_reporting
key: 088ac5828f9a15a2110a7242c503909ffc1d8121230a720a8f1293201a86d1d2_v4
```

A temporary request-only debug field was used in a separate diagnostic replay
to identify the prompt, then removed from the source. That diagnostic output
is retained under `artifacts/post-fix-new-miss/LL-diagnostic/`; the formal
failure output is under `artifacts/post-fix-new-miss/LL-formal/`. Both replays
used the inaccessible loopback endpoint and left the frozen cache unchanged.

The independent GraphRAG prompt capture and comparison were:

```text
rtk env NO_PROXY=127.0.0.1,localhost \
  no_proxy=127.0.0.1,localhost \
  ../graphrag/.venv/bin/python \
  update_reference/log_graphrag_community_requests.py update \
  --root .../diagnostic_rr_community_requests --skip-validation --cache
exit 0; 48 community-report requests captured; zero misses

rtk ../graphrag/.venv/bin/python \
  update_reference/compare_community_requests.py \
  --graphloom-log .../LL-diagnostic/logs/indexing-engine.log \
  --graphrag-log .../diagnostic-rr-community-requests.log \
  --key 088ac582...d1d2_v4 \
  --output artifacts/community-request-nearest-diff.json
exit 0
```

The generation sites requiring follow-up are:

- GraphLoom
  `crates/graphloom/src/operations/community_reports/context.rs:301-321`
  maps both empty and absent descriptions to `No Description`.
- GraphRAG
  `packages/graphrag/graphrag/index/workflows/create_community_reports.py:143-159`
  uses pandas `fillna("No Description")`, which replaces null values but
  deliberately leaves `""` unchanged.

The row counts immediately before report generation match RR's new-data
pipeline (`202` finalized entities, `588` relationships, `48` communities), so
this is not a table-count divergence. The nearest GraphRAG key is
`96f3ab334388bfef8549cf29e65352b6c4f95c7b444ee62e2c60e5608dc87099_v4`
with a sequence similarity of `0.9978277734678045`; the complete unified diff
contains only:

```diff
-61,婚约,No Description,2
-62,送礼,No Description,2
+61,婚约,,2
+62,送礼,,2
```

This proves the minimum next incompatibility is empty-vs-null normalization in
community-report context construction. Per the stop-on-next-failure rule, this
task does not repair that second product difference.

## Phase 2 matrix conclusion (historical)

- Completion response contract: **fixed and regression-tested**.
- Reference cache integrity: **unchanged; RR-complete**.
- LL update: **blocked before merge by a new community-report cache request**.
- RG update: **not rerun after the LL miss, per the stop-on-first-miss rule**.
- RR final vs LL final: **not performed; no LL final exists**.
- RR final vs RG final: **not performed; RG was not run post-fix**.
- Update merge behavior: **not reached and not classified**.
- Final classification: **incompatible before merge due to
  `create_community_reports` request-content/cache-key contract difference**.

## Community request repair

The table-reading data flow already had the correct distinction:

- Arrow null becomes `None`, then `No Description`.
- `Some("")` remains `""`.
- non-empty and whitespace-only strings remain byte-for-byte unchanged.

The bug was the second normalization in
`crates/graphloom/src/operations/community_reports/context.rs`, where
`description_or_default()` converted every empty `String` to
`No Description`. The repair removes
`entity_with_default_description`,
`relationship_with_default_description`, and `description_or_default`.
Context construction now directly clones the table-normalized entity and
relationship rows. Claims and update merge code are unchanged.

Final semantics for both entity and relationship descriptions:

| Input | Context CSV |
|---|---|
| null | `No Description` |
| `""` | empty CSV cell |
| `" "` | one space, unchanged |
| non-empty | unchanged |

The regression fixture contains:

```text
61,婚约,,2
62,送礼, ,2
...
142,婚约,送礼,,4
```

Its minimal request key, independently computed by GraphRAG 3.1.0, is
`31ea235314d1b2f0b22f5eae33e351a888e545de305707bfb1f5d29499992bed_v4`.

Validation after both product repairs:

```text
rtk cargo +nightly fmt --check
exit 0
rtk cargo test -p graphloom
exit 0; 448 passed, 3 ignored
rtk cargo test -p graphloom-llm
exit 0; 103 passed
rtk cargo test -p graphloom-llm --test cache_compat
exit 0; 64 passed
rtk cargo test
exit 0; 627 passed, 4 ignored
rtk cargo build
exit 0
rtk cargo clippy -- -D warnings
exit 0
rtk ../graphrag/.venv/bin/python -m pytest -q \
  update_debug/test_audit_update_fixture.py
exit 0; 4 passed
```

Optional pedantic clippy still reports the same 37 pre-existing query-module
findings; none is in a modified completion or community-report file.

## Phase 3 frozen-cache lanes

Both lanes used:

```text
NO_PROXY=127.0.0.1,localhost
no_proxy=127.0.0.1,localhost
completion/embedding endpoint=127.0.0.1:9
```

LL restoration:

```text
initial manifest:
6c6bcb8fe07c7a74bcee4adec2181496e151a8cea312d69bdd116f3355be4f05
matches saved LL initial: true
```

RG restoration:

```text
initial manifest:
a6d871c4b208cca3bfd4d6f2c0ee6c64f4b039cded36977955cfb49e066eea51
matches RR initial copy baseline: true
```

Commands:

```text
rtk env NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost \
  RUST_LOG=graphloom_llm=debug,graphloom=info \
  target/debug/graphloom update \
  --root update_reference/lanes/LL --skip-validation
exit 0

rtk env NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost \
  RUST_LOG=graphloom_llm=debug,graphloom=info \
  target/debug/graphloom update \
  --root update_reference/lanes/RG --skip-validation
exit 0
```

| Lane | Completion hit/miss | Embedding hit/miss | All workflows | Merge | Cache manifest |
|---|---:|---:|---|---|---|
| LL | 507 / 0 | 84 / 0 | completed | completed | unchanged |
| RG | 507 / 0 | 84 / 0 | completed | completed | unchanged |

Both logs contain a hit for the original empty-content key
`3383d9ac…_v4` and the corrected GraphRAG community key
`96f3ab33…87099_v4`. Neither contains the old GraphLoom key
`088ac582…d1d2_v4` or any `cache miss`.

Before and after both lanes, the frozen cache remains:

```text
entries: 1,547
bytes: 59,841,404
manifest:
840704ca8ea5295903f639e59cc4ed5607ce4ab02b4d15f0f1ddd0208cd1b1fb
```

Final row counts for RR, LL, and RG are identical:

| Table | Rows |
|---|---:|
| documents | 2 |
| text_units | 58 |
| entities | 346 |
| relationships | 1,089 |
| communities | 94 |
| community_reports | 93 |
| covariates | 949 |

## Final comparisons

Both machine gates exit 0:

```text
RR final vs LL final:
update_reference/artifacts/final-rr-vs-ll-after-community-fix.json

RR final vs RG final:
update_reference/artifacts/final-rr-vs-rg-after-community-fix.json
```

For both comparisons:

- table sets and row counts match;
- all seven canonical semantic tables match with zero field mismatches;
- strict Arrow/storage comparison does not match;
- all three logical LanceDB collections have matching semantic IDs, row
  counts, dimension 1024, and vectors within `1e-07`;
- maximum vector absolute delta is
  `1.4271163917278784e-08`;
- strict vector bytes/UUIDs do not match.

Strict differences retained in the artifacts include:

- GraphLoom Arrow `large_string`/`large_list` versus GraphRAG
  `string`/`list`;
- community-report finding struct child ordering and JSON property order;
- independently generated UUIDs and dependent list references;
- implementation-generated text-unit ordinals/IDs;
- Float32 values differing below the explicit `1e-07` tolerance.

After mapping retained delta UUIDs by entity title and relationship endpoint,
RR, LL, and RG have identical GraphRAG-reference violation multisets:

| Reference | Count | Unique canonical IDs | Canonical multiset SHA-256 |
|---|---:|---:|---|
| `text_units.relationship_ids` | 249 | 70 | `e16e2d2f…51c0a` |
| `communities.entity_ids` | 119 | 41 | `60fad6f0…ef91` |
| `communities.relationship_ids` | 79 | 38 | `a4a01705…20df` |

These remain GraphRAG reference behavior, not a GraphLoom regression.

ID stability is identical across RR, LL, and RG:

- every initial Parquet ID is retained;
- every initial logical vector ID is retained;
- no retained `human_readable_id` changes;
- additions are exactly `+1` document, `+24` text units, `+160` entities,
  `+518` relationships, `+48` communities, `+47` reports, and `+367`
  covariates;
- vector additions are `+47` community, `+160` entity, and `+24` text-unit
  records.

The detailed evidence is
`update_reference/artifacts/id-stability-after-community-fix.json`.

## Final classification

- Reference cache incomplete: **no**.
- Cache request-key contract: **compatible after both fixes**.
- Completion response contract: **compatible**.
- Initial index: **canonically compatible**.
- Native LL update/merge: **canonically compatible**.
- GraphRAG → GraphLoom RG handoff/update/merge: **canonically compatible**.
- Schema/storage: **strictly different, canonically equivalent**.
- Vector store: **strict UUID/rounding differences, semantically equivalent**.
- Allowed non-determinism/representation differences: UUID identity,
  dependent UUID references, Arrow string/list width, JSON property order,
  implementation-generated row ordinals, and sub-`1e-07` Float32 rounding.

Therefore, for this configuration and fixture, GraphLoom native update and the
GraphRAG 3.1.0 → GraphLoom handoff are **compatible under the listed
non-semantic strict-storage differences**. Strict-storage compatibility,
reverse handoff, and compatibility for other configurations remain unverified.

## Files changed in the compatibility worktree

Product and Rust regression files:

- `crates/graphloom-llm/src/types/completion.rs`
- `crates/graphloom-llm/tests/cache_compat.rs`
- `crates/graphloom-llm/tests/fixtures/graphrag/completion/empty_content_with_reasoning_v4`
- `crates/graphloom/src/operations/graph/extraction.rs`
- `crates/graphloom/src/operations/community_reports/context.rs`
- `crates/graphloom/src/operations/community_reports/tables.rs`
- `crates/graphloom/tests/fixtures/community_reports/empty_descriptions_context.txt`

Experiment and audit files:

- `update_debug/audit_update_fixture.py`
- `update_debug/test_audit_update_fixture.py`
- `update_reference/REPORT.md`
- `update_reference/artifacts/matrix-status.json`
- the compact comparison, request-contract, matrix, and ID-stability evidence
  listed below.

No update merge workflow source was modified. The frozen cache was not written,
rebuilt, or supplemented.

## Reproducible artifacts

- `update_reference/effective_config_gate.py`
- `update_reference/tree_manifest.py`
- `update_reference/extract_graphrag_metrics.py`
- `update_reference/id_stability.py`
- `update_reference/artifacts/effective-config.json`
- `update_reference/artifacts/matrix-status.json`
- `update_reference/artifacts/community-request-nearest-diff.json`
- `update_reference/artifacts/community-request-fix-gate.json`
- `update_reference/artifacts/final-rr-vs-ll-after-community-fix.json`
- `update_reference/artifacts/final-rr-vs-rg-after-community-fix.json`
- `update_reference/artifacts/id-stability-after-community-fix.json`
- Improved `update_debug/audit_update_fixture.py`
- `update_debug/test_audit_update_fixture.py`
- Root `Makefile` targets `update-debug-audit`,
  `test-update-debug-audit`, and `update-reference-config-gate`

The 59 MiB frozen cache, builder cache, complete RR/LL/RG outputs, snapshots,
provider/proxy captures, and full diagnostic logs remain local experiment
artifacts and are intentionally excluded from version control. The committed
final-comparison JSON files are compact gate summaries; the full raw gate
outputs also remain local.
