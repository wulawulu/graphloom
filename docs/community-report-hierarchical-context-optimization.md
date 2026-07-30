# Community-report hierarchical-context optimization

Status: Planned optimization · Compatibility baseline: GraphRAG
`79ab7c9ad586856e82635264c200d8a1eb3c63d9` · Recorded: 2026-07-15

## Background

GraphLoom currently prioritizes compatibility with the observed behavior of the
pinned GraphRAG baseline for `create_community_reports`. That contract includes:

- community entity and relationship selection, ordering, and deduplication;
- field order, escaping, and newlines equivalent to Pandas CSV;
- truncation at the token limit and the first-relationship fallback;
- rendered prompts, completion parameters, and GraphRAG v4 cache keys;
- skipping invalid community-report responses.

This baseline is a prerequisite for later optimization. It must not change
without an explicit mode, cache isolation, and cross-implementation validation.
The cache constraints are documented in the
[GraphRAG v4 LLM cache interoperability study](research/study-graphrag-llm-cache.md).

## Current baseline behavior

GraphRAG's `build_level_context` contains a hierarchical replacement strategy:
when a parent community's local context exceeds the token limit, it can replace
some detailed entity and relationship context with already generated child
community reports.

However, the pinned baseline's standard workflow builds context for every level
before generating any community report. The `reports` collection passed during
context construction is therefore always empty, so the standard workflow never
enters the child-report replacement branch:

1. Build and freeze local context for every level.
2. Truncate the relationship list when it exceeds the limit.
3. Keep the first relationship and its related entities even when that
   relationship alone exceeds the limit.
4. Generate reports by level only after all contexts have been built.

GraphLoom reproduces this execution order and fallback semantics. Compatibility
with that behavior does not imply that it is the best report-generation
strategy.

## Validated compatibility baseline

Cross-validation used identical `entities.parquet`, `relationships.parquet`,
`communities.parquet`, `covariates.parquet`, and graph prompts:

| `max_input_length` | Communities | GraphRAG unique request keys | GraphLoom unique request keys | Exact matches | Notes |
| ---: | ---: | ---: | ---: | ---: | --- |
| 1,000 | 89 | 88 | 88 | 88 | Covers ordinary truncation; two communities produce the same request |
| 50 | 89 | 71 | 71 | 71 | All 89 contexts exceed the limit; covers the first-relationship fallback |

At a limit of 50, GraphRAG's final contexts still contain 75–911 tokens. This
shows that the standard workflow does not use child-report replacement to
enforce the limit. GraphLoom produces the same request-key set.

The corresponding Rust regression is
`operations::community_reports::context::tests::test_should_keep_first_edge_when_it_alone_exceeds_limit_like_graphrag`.

## Optimization goal

After the compatibility baseline remains stable, introduce genuinely bottom-up
community-report generation:

1. Generate reports beginning with the deepest communities.
2. Allow parent context to reference successfully generated direct-child
   reports.
3. Prefer replacing the child-community detail with the highest token cost.
4. Recompute the complete prompt token count after each replacement, not only
   the data-fragment count.
5. Retain as many high-value entities, relationships, and child reports as the
   budget permits.
6. Deterministically fall back to compatible truncation when child reports are
   missing, invalid, or still too long.
7. Ensure concurrency cannot change community, entity, relationship, or report
   selection order.

This may reduce repeated detail in large parent communities and use denser
child summaries. Success must be measured through report quality, token use,
and end-to-end latency, not inferred merely from shorter context.

## Compatibility and cache boundary

Optimized prompts will differ from the GraphRAG baseline and therefore produce
different cache keys. An implementation must:

- keep baseline-compatible behavior available and passing GraphRAG cross-tests;
- require explicit opt-in instead of silently changing existing results;
- use an independent cache namespace or an explicit cache-version discriminator;
- never introduce a cache converter or rewrite copied GraphRAG caches;
- restore the existing compatible request keys when optimization is disabled;
- prevent configuration changes, retries, and missing child reports from
  polluting the other mode's cache.

## Acceptance criteria

The optimization is complete only when all of the following hold:

1. Compatibility mode continues to match the pinned GraphRAG baseline
   request-by-request at the default, 1,000, and extremely low limits.
2. Optimized mode demonstrably consumes generated child reports in parent
   context rather than leaving a dead branch.
3. The final prompt, not an intermediate fragment, fits the configured token
   budget or returns an explicit observable unsatisfiable state.
4. Replacement order, fallback paths, and output remain deterministic across
   concurrency levels.
5. Invalid or missing child reports do not fail the entire workflow.
6. Baseline and optimized caches can coexist without cross-hits.
7. Representative large-community data compares quality, tokens, model calls,
   cache hit rate, and total latency.
8. The complete Rust gates and cross-project GraphRAG compatibility tests pass.

## Non-goals

- Modifying old GraphRAG caches to fit optimized mode.
- Replacing higher-quality context with shorter context without quality
  evaluation.
- Treating a branch unused by the pinned standard workflow as already validated
  standard behavior.
- Breaking GraphRAG configuration, Parquet, or cache interoperability to enable
  the optimization.
