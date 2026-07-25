# GraphRAG 3.1.0 Compatibility Optimization Opportunities

Every item below describes behavior that GraphLoom intentionally preserves for
the GraphRAG 3.1.0 compatibility baseline.

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
