# GraphRAG 3.1.0 Compatibility Optimization Opportunities

Last reviewed: 2026-07-30

Every item below describes behavior that GraphLoom intentionally preserves for
the GraphRAG 3.1.0 compatibility baseline. These are compatibility debts, not
accidental TODOs: changing them in the default path without an explicit mode
and new evidence would break the current contract.

The authoritative status of the surrounding feature is maintained in the
[GraphRAG compatibility matrix](compatibility-matrix.md).

## Priority and maintenance policy

| Priority | Meaning |
|---|---|
| P0 | Publication, recovery, or stale-data risk that can leave an index misleading or internally inconsistent. |
| P1 | Correctness, result-quality, or material model/vector cost issue. |
| P2 | Efficiency, usability, or maintainability issue with a narrower operational impact. |

An optimization may replace compatible behavior only in an explicitly named
non-compatible mode until it has a migration story and its own tests. A change
must preserve the default compatibility fixture, document artifact differences,
and update both this backlog and the compatibility matrix in the same change.

## Backlog summary

| ID | Area | Preserved debt | Priority | Optimized mode |
|---|---|---|---|---|
| O-01 | Extract graph | Entity summary joins by title only. | P1 | Not implemented |
| O-02 | Update | Extraction and update use different entity identities. | P1 | Not implemented |
| O-03 | Update | Entity degree is retained instead of recomputed. | P1 | Not implemented |
| O-04 | Update | Document changes are detected by title only. | P1 | Not implemented |
| O-05 | Update | Deleted inputs remain in tables and vectors. | P0 | Not implemented |
| O-06 | Update | No-op updates still copy `previous`. | P2 | Not implemented |
| O-07 | Update | Every provider table is copied. | P2 | Not implemented |
| O-08 | Update | Delta and final records are embedded separately. | P1 | Not implemented |
| O-09 | Vectors | Every embedding pass overwrites managed collections. | P1 | Not implemented |
| O-10 | Update | Delta vectors become visible before final tables. | P0 | Not implemented |
| O-11 | Update | Community `children` IDs are not remapped. | P1 | Not implemented |
| O-12 | Update | Community report titles retain old numbering. | P2 | Not implemented |
| O-13 | Update | Sequential merge failure leaves partial final output. | P0 | Not implemented |
| O-14 | Prompt tune | Auto ranks a sample but selects original-row positions. | P1 | Not implemented |
| O-15 | Prompt tune | Oversized Random limit falls back to 15 and can still fail. | P2 | Not implemented |
| O-16 | Prompt tune | Concurrent examples reuse one accumulated message list. | P1 | Not implemented |
| O-17 | Claims | Gleaning responses are requested but not parsed. | P1 | Not implemented |
| O-18 | Community reports | Claims are omitted from graph context. | P1 | Not implemented |
| O-19 | Query | Static community roll-up conflates entities by title. | P1 | Not implemented |
| O-20 | Index publication | Standard indexing writes directly to active output. | P0 | Not implemented |
| O-21 | Community reports | Standard execution never reaches child-report context replacement. | P1 | Not implemented |

## 1. Entity summary title-only join

**GraphRAG behavior:** Entity extraction groups by `(title,type)`, but summary
rows are joined back by `title` only, producing a Cartesian product for one
title with multiple types.

**Problem:** Descriptions can be associated with the wrong type and row counts
grow multiplicatively.

**GraphLoom compatibility behavior:** The title-only many-to-many join and row
order are reproduced exactly.

**Future optimization:** Preserve typed summary identity through the join.

**Affected components:** `extract_graph`, graph summarization, `finalize_graph`.

Compatibility baseline: implemented

Future optimization: not implemented

## 2. Update entity identity differs from extraction identity

**GraphRAG behavior:** Update merges entities by `title`, while extraction first
groups by `(title,type)`.

**Problem:** Type distinctions can collapse during update.

**GraphLoom compatibility behavior:** Update uses sorted title grouping and
keeps the first row's identity fields.

**Future optimization:** Introduce an explicit, separately tested identity
strategy.

**Affected components:** entity merge, entity ID mapping, text-unit remapping.

Compatibility baseline: implemented

Future optimization: not implemented

## 3. Entity degree is not recomputed

**GraphRAG behavior:** The merged entity keeps the first `degree`.

**Problem:** Degree can disagree with the merged relationship graph.

**GraphLoom compatibility behavior:** The first degree is retained.

**Future optimization:** Recompute degree from final relationships.

**Affected components:** entity merge, Local Search ranking.

Compatibility baseline: implemented

Future optimization: not implemented

## 4. Document delta uses title only

**GraphRAG behavior:** Existing titles are ignored even when text or metadata
changes.

**Problem:** Edited documents cannot be refreshed incrementally.

**GraphLoom compatibility behavior:** Only title membership determines new
input.

**Future optimization:** Add an explicit content-aware update algorithm.

**Affected components:** `load_update_documents`, documents, cache/model work.

Compatibility baseline: implemented

Future optimization: not implemented

## 5. Deleted inputs are not applied

**GraphRAG behavior:** Deleted titles can be calculated, but the update pipeline
does not remove their records.

**Problem:** Removed source material remains queryable.

**GraphLoom compatibility behavior:** No document, graph, or vector deletion is
performed.

**Future optimization:** Add reference-aware deletion and vector cleanup.

**Affected components:** all final tables and managed vectors.

Compatibility baseline: implemented

Future optimization: not implemented

## 6. No-op update still copies previous

**GraphRAG behavior:** The timestamp namespace and complete `previous` copy are
created before new titles are detected.

**Problem:** A no-op consumes storage and I/O.

**GraphLoom compatibility behavior:** The copy occurs, then the pipeline stops
after `load_update_documents`.

**Future optimization:** Detect no-op before backup under an explicit
non-compatible mode.

**Affected components:** update runtime preparation and table providers.

Compatibility baseline: implemented

Future optimization: not implemented

## 7. Every provider table is copied

**GraphRAG behavior:** Update startup lists and copies every formal output table
provider entry.

**Problem:** Backup cost scales with the entire index, not the delta.

**GraphLoom compatibility behavior:** Every listed table is read and written to
`previous`; cache, logs, and LanceDB are not copied.

**Future optimization:** Use snapshots, reflinks, or table-version references.

**Affected components:** update storage and runtime preparation.

Compatibility baseline: implemented

Future optimization: not implemented

## 8. Embeddings are generated twice

**GraphRAG behavior:** Delta standard indexing embeds delta tables, then
`update_text_embeddings` embeds final tables again.

**Problem:** New records incur duplicate model and vector work.

**GraphLoom compatibility behavior:** Both passes run with the configured cache,
batching, snapshots, and callbacks.

**Future optimization:** Reuse delta vectors or embed only changed final rows.

**Affected components:** embedding model, cache, snapshots, vector store.

Compatibility baseline: implemented

Future optimization: not implemented

## 9. Embedding passes overwrite managed collections

**GraphRAG behavior:** `embed_text` calls the LanceDB provider's
`create_index()`, which creates the table with `mode="overwrite"` before loading
documents. Both delta and final passes therefore replace each managed
collection. Every later flush uses unconditional append, so duplicate IDs remain
duplicate rows whether they occur within one batch or across multiple flushes.

**Problem:** The delta pass mutates the final store but is later discarded, and
duplicate content-addressed report IDs make the vector collection non-unique.

**GraphLoom compatibility behavior:** Each configured managed collection whose
source table exists is reset once per embedding pass, then every flush appends.
The complete update manifest retains duplicate rows, matching GraphRAG 3.1.0.
Unconfigured collections, configured fields with missing source tables, and
unknown third-party collections are untouched.

**Future optimization:** Use keyed incremental upserts with explicit stale-ID
cleanup and uniqueness guarantees.

**Affected components:** all managed vectors, vector manifests, storage writes.

Compatibility baseline: implemented

Future optimization: not implemented

## 10. Update mutates final vectors early

**GraphRAG behavior:** Delta embeddings overwrite configured final managed
collections before table merging completes.

**Problem:** A later failure can leave vector state ahead of final Parquet.

**GraphLoom compatibility behavior:** Delta collection replacement is immediate
and remains visible until a later final embedding pass replaces it.

**Future optimization:** Stage vector publication atomically with tables.

**Affected components:** update runtime, embedding workflow, failure recovery.

Compatibility baseline: implemented

Future optimization: not implemented

## 11. Community children are not remapped

**GraphRAG behavior:** Delta `community` and `parent` are rebased; `children`
remains unchanged.

**Problem:** Child IDs can refer to the delta-local numbering.

**GraphLoom compatibility behavior:** Only `community` and `parent` are mapped.

**Future optimization:** Remap every hierarchy reference consistently.

**Affected components:** communities and hierarchical query context.

Compatibility baseline: implemented

Future optimization: not implemented

## 12. Community report title is not rewritten

**GraphRAG behavior:** Report `community`, `parent`, and
`human_readable_id` change, but report `title` does not.

**Problem:** A title containing the old community number can become stale.

**GraphLoom compatibility behavior:** The delta report title is preserved.

**Future optimization:** Regenerate or rewrite titles with explicit semantics.

**Affected components:** community reports and Global Search context.

Compatibility baseline: implemented

Future optimization: not implemented

## 13. Merge failure leaves partial final output

**GraphRAG behavior:** Merge workflows write final tables sequentially without
rollback.

**Problem:** Failure can leave a mixed-version index.

**GraphLoom compatibility behavior:** Completed final and vector writes remain;
`previous` and `delta` are preserved for diagnosis.

**Future optimization:** Add atomic publication and recovery metadata.

**Affected components:** all update workflows, final storage, vector storage.

Compatibility baseline: implemented

Future optimization: not implemented

## 14. Auto selection ranks one set and returns another

**GraphRAG behavior:** Auto randomly samples up to `n_subset_max` chunks,
embeds and ranks the sample positions by centroid distance, then applies those
positions to the original unsampled chunk list.

**Problem:** The selected chunks are not necessarily the chunks whose
embeddings were ranked, so the model cost does not reliably improve sample
representativeness.

**GraphLoom compatibility behavior:** The ranked positional indices are applied
to the original chunk list exactly as in GraphRAG 3.1.0.

**Future optimization:** In an explicit optimized selection mode, return the
ranked sampled rows themselves and expose a deterministic sampling hook.

**Affected components:** prompt-tune Auto selection, embedding requests, prompt
examples.

Compatibility baseline: implemented

Future optimization: not implemented

## 15. Oversized Random limit fallback can create an avoidable error

**GraphRAG behavior:** The public API rejects non-positive `limit`,
`min_examples_required`, `n_subset_max`, and `k` values. A positive limit larger
than the chunk count reaches the loader, falls back to 15, and Random selection
then fails when fewer than 15 chunks exist.

**Problem:** A request to use all available small-corpus chunks becomes an
error, and the fallback obscures the invalid effective value.

**GraphLoom compatibility behavior:** GraphLoom validates the four positive
fields at its public API boundary and reproduces the oversized-limit fallback
and resulting Random error. Top remains observable as a separate clamped path.

**Future optimization:** Validate the requested value directly or use
`min(limit, chunk_count)` in a clearly named safe selection mode.

**Affected components:** prompt-tune Top/Random selection and CLI diagnostics.

Compatibility baseline: implemented

Future optimization: not implemented

## 16. Relationship examples reuse an accumulated request

**GraphRAG behavior:** Concurrent relationship-example coroutines share one
mutable message builder. By the time they execute, every request observes the
final accumulated list of selected documents.

**Problem:** Requests are repeated with unrelated document messages, total
input grows approximately quadratically with the example count, and a response
is no longer isolated to one intended document.

**GraphLoom compatibility behavior:** One accumulated request is cloned for
each selected document and responses are collected in producer order.

**Future optimization:** Build one independent request per document, or issue
one explicitly batched request and define how its response maps to examples.

**Affected components:** prompt-tune relationship examples, completion cost,
generated extraction prompt.

Compatibility baseline: implemented

Future optimization: not implemented

## 17. Claim gleaning responses are discarded

**GraphRAG behavior:** Claim extraction can send continuation and loop-check
requests, but tuple parsing uses only the initial completion.

**Problem:** Extra model calls consume latency and tokens without contributing
additional claims; valid claims found by continuation responses are lost.

**GraphLoom compatibility behavior:** The continuation conversation and stop
checks run, but only the initial response is parsed.

**Future optimization:** Parse and merge every accepted continuation with
stable deduplication, or skip continuation calls when compatibility is not
required.

**Affected components:** covariate extraction, LLM cache and cost, downstream
claim coverage.

Compatibility baseline: implemented

Future optimization: not implemented

## 18. Claims are omitted from community-report graph context

**GraphRAG behavior:** Community-report preparation supplies claims as scalar
merge values while the context sorter accepts claim lists, so claims disappear
from the rendered graph context.

**Problem:** Claim extraction may incur model and storage cost without enriching
community reports, reducing report grounding and wasting token budget allocated
to covariates.

**GraphLoom compatibility behavior:** Claims remain part of the workflow input
contract but are intentionally absent from the compatible rendered context.

**Future optimization:** Normalize claims into an explicit per-community list
and budget them alongside entities and relationships.

**Affected components:** community-report context, covariates, report quality
and token budgeting.

Compatibility baseline: implemented

Future optimization: not implemented

## 19. Static Query community roll-up uses entity title

**GraphRAG behavior:** Static community-report adaptation explodes entity
memberships, groups them by entity title, takes the maximum community number,
and uses those numbers to select reports.

**Problem:** Distinct entities with the same title can collapse, while opaque
numeric maxima are not a stable semantic parent/child rule.

**GraphLoom compatibility behavior:** The title-based maximum-community roll-up
is reproduced for static Global Query.

**Future optimization:** Select reports through entity IDs and explicit
hierarchy relationships, with a migration test for duplicate titles.

**Affected components:** Query index adaptation, static Global context and
community selection.

Compatibility baseline: implemented

Future optimization: not implemented

## 20. Standard indexing publishes directly to active output

**GraphRAG behavior:** Workflows write active Parquet and vector outputs
directly. There is no generation pointer, ready marker, cross-command lock, or
atomic activation step.

**Problem:** Concurrent Query can observe mixed generations, and a failed index
can leave a partial but parseable active index.

**GraphLoom compatibility behavior:** Standard indexing preserves direct active
publication. This is separate from the safer transactional `init` and
prompt-tune file publication.

**Future optimization:** Stage a complete generation, validate tables and
vectors, then atomically switch a generation pointer with recovery metadata.

**Affected components:** standard indexing, Query startup, storage layout and
failure recovery.

Compatibility baseline: implemented

Future optimization: not implemented

## 21. Child community reports never replace parent detail

**GraphRAG behavior:** The standard workflow builds and freezes context for
every community before generating any report. Although the context builder can
replace child-community detail with an existing child report, the report
collection is empty during context construction and that branch is not reached.

**Problem:** Parent prompts repeat low-level entity and relationship detail
instead of reusing denser child summaries. At very low limits, the compatible
first-relationship fallback can also leave the final context above the
configured token budget.

**GraphLoom compatibility behavior:** Context is built before report
generation, including GraphRAG's truncation and first-relationship fallback.

**Future optimization:** Add an explicit bottom-up mode that generates deepest
reports first, budgets complete prompts, and safely substitutes available child
reports. Baseline and optimized caches must remain isolated.

**Affected components:** Community-report scheduling, hierarchical context,
token budgeting, cache keys, and report quality. See the
[detailed optimization design](community-report-hierarchical-context-optimization.md).

Compatibility baseline: implemented

Future optimization: not implemented
