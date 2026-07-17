# Study: GraphRAG v4 LLM cache interoperability

Status: Done · Owner: graphloom · Date: 2026-07-12 · Source pin: `../graphrag` @ `79ab7c9ad586856e82635264c200d8a1eb3c63d9`

## Why this study

GraphLoom needs provider-neutral completion and embedding types whose JSON representation, cache
keys, namespaces, and invalidation behavior interoperate directly with current GraphRAG caches.

本文研究缓存协议的互操作性，并不要求 GraphLoom 复刻 GraphRAG 在 LLM 调用之后执行的每一项 Pandas
转换。实体摘要连接差异，以及 GraphLoom 保留更强输出语义的原因，详见
[GraphRAG `extract_graph` 输出语义研究](study-graphrag-extract-graph-output.md)。

## Architecture map

```text
GraphRAG workflow
  │ cache.child(model_instance_name)
  ▼
completion / embedding factory
  ▼
with_cache middleware ── hit ──▶ Pydantic OpenAI-compatible response
  │ miss
  ▼
LiteLLM provider ──▶ {response: model_dump(), metrics: {...}}
  ▼
JsonCache ──▶ {result: {response: {...}, metrics: {...}}}
```

## Hot path walkthrough

1. Workflows create separate child caches for extraction, summarization, reporting, claims, and
   embeddings using each configuration's `model_instance_name`
   (`packages/graphrag/graphrag/index/workflows/extract_graph.py:41-58`,
   `generate_text_embeddings.py:79-85`).
2. The cache middleware bypasses streaming and mocked calls, hashes the original kwargs, decodes
   cached `response` as either `LLMCompletionResponse` or `LLMEmbeddingResponse`, and stores the
   provider `model_dump()` together with metrics
   (`packages/graphrag-llm/graphrag_llm/middleware/with_cache.py:106-150`).
3. GraphRAG removes `metrics`, streaming controls, mock controls, timeout, endpoint/auth fields,
   Azure token providers, and `drop_params` before hashing
   (`packages/graphrag-llm/graphrag_llm/cache/create_cache_key.py:42-69`).
4. The common hasher applies PyYAML `dump(..., sort_keys=True)` and SHA-256
   (`packages/graphrag-common/graphrag_common/hasher/hasher.py:16-59`); GraphRAG appends `_v4`
   (`packages/graphrag/graphrag/cache/cache_key_creator.py:9-45`).
5. JsonCache deletes malformed UTF-8/JSON, returns the outer `result`, and writes
   `{result: value}` (`packages/graphrag-cache/graphrag_cache/json_cache.py:33-55`).

## Key data structures

`LLMCompletionResponse` extends the OpenAI `ChatCompletion`, adds `formatted_response`, and emits a
computed top-level `content`. `LLMEmbeddingResponse` extends OpenAI's embedding response and emits
computed `embeddings` and `first_embedding`
(`packages/graphrag-llm/graphrag_llm/types/types.py:86-101,181-198`). These computed fields and
provider-specific nested fields occur in the real fixtures and must survive semantic round trips.

The completion fixture at
`ragdebug/cache/extract_graph/04ad9d...e3e_v4` contains nested choice/message extension fields,
reasoning content, detailed usage, top-level computed fields, and floating-point metrics. The
embedding fixture at `ragdebug/cache/text_embedding/8428d...9_v4` contains five 1024-dimensional
vectors, detailed usage, computed fields, and metrics.

## Key algorithms

The key is `sha256(PyYAML.dump(filtered_kwargs, sort_keys=True)) + "_v4"`. Namespace and model
instance names are directory routing only; they are not added to the hash. Missing kwargs remain
missing rather than being normalized to null.

## What we will adopt

- Provider-neutral Rust request/response types with explicit core fields and flattened unknowns.
- One generic `{response, metrics}` envelope for completion and embedding.
- Cache middleware around raw provider models, with child namespaces selected during model
  resolution.
- Invalid JSON/schema entries are deleted and treated as misses; storage errors remain fatal.
- GraphRAG-compatible v4 key generation over canonical request kwargs.

## What we will avoid

- Provider response types in workflow APIs.
- Permanent adapters for GraphLoom's old simplified cache payloads.
- Request-level cache namespaces or model ids injected into hashes.
- Workflow/operation-specific cache reads and writes.

## Compatibility hardening follow-up (2026-07-12)

The pinned GraphRAG implementation calls `yaml.dump(data, sort_keys=True)` without overriding
PyYAML defaults (`packages/graphrag-common/graphrag_common/hasher/hasher.py:57`). In the local
environment this resolves to PyYAML 6.0.3: block style, width 80, LF line endings, and
`allow_unicode=None`, which escapes non-ASCII scalars. Compatibility therefore requires
byte-for-byte PyYAML emission, not merely YAML semantic equivalence. The fixture generator records
both the GraphRAG commit and PyYAML version and drives the Rust compatibility emitter with 31
edge-case goldens.

GraphRAG's cache middleware uses `kwargs.get("mock_response") or False` for both sync and async
paths (`packages/graphrag-llm/graphrag_llm/middleware/with_cache.py:60,110`). This is Python
truthiness, not a boolean-only flag. LiteLLM completion plumbing explicitly types the value as
`str | None` (`packages/graphrag-llm/graphrag_llm/completion/lite_llm_completion.py:255`), so
non-empty strings must bypass cache.

The local `async-openai` 0.41.1 dependency enables its `byot` feature. Its `create_byot` methods
accept arbitrary `Serialize` request bodies while preserving the existing client,
authentication, Tower retry, timeout, and concurrency stack. GraphLoom can therefore transmit
validated canonical `extra` fields without replacing the provider transport or silently losing
unknown fields.

The final compatibility corpus adds long plain, single-quoted, double-quoted/multiline, sequence,
and nested mapping scalars. PyYAML's default width of 80 is a preferred break threshold: it emits
the current word first and replaces the following eligible space with a newline, so a physical
line can legitimately exceed 80 columns. Continuation indentation is determined by the scalar's
mapping/sequence nesting level. Python `json.loads` also determines float rendering before
PyYAML sees the value: moderate positive exponents expand to decimal (`1e7` becomes
`10000000.0`), negative one-digit exponents are zero-padded (`1e-7` becomes `1.0e-07`), and large
exponents remain scientific (`1e20` becomes `1.0e+20`).

GraphRAG's excluded cache-key kwargs mix provider transport configuration and middleware controls.
They remain valid raw kwargs for key compatibility, but GraphLoom's OpenAI BYOT adapter must reject
them before HTTP serialization: `mock_response`, endpoint/auth fields, timeout, Azure token
providers, `drop_params`, `stream_options`, and metrics. Arbitrary non-conflicting provider body
extensions remain open and are transmitted unchanged.

PyYAML's plain-scalar indicator rules distinguish unconditional indicator prefixes from contextual
ones. Leading quotes, closing brackets/braces, comma, percent, anchor, alias, and tag markers are
always single-quoted; `-`, `?`, and `:` require quoting primarily when followed by whitespace;
exact `---` and `...` are special. Repeated spaces are meaningful: wrapping replaces only an
eligible separator space and preserves every additional alignment space. The 70-case corpus checks
both byte equality and YAML decode equality for repeated-space cases.

Provider validation must precede cache routing. A client-only extra such as `api_key` is excluded
from GraphRAG's hash and therefore shares a key with the valid request; validating only inside the
provider request builder lets an illegal request return a cache hit. The model traits now expose an
object-safe, synchronous preflight. Cache wrappers delegate it to their inner model before bypass
decisions, hashing, or lookup; OpenAI overrides it with provider rules, while mock models inherit
canonical-only validation and continue to accept `mock_response`.

PyYAML 6.0.3 selects implicit scalar types through
`yaml.resolver.Resolver.yaml_implicit_resolvers`. For JSON-origin strings, GraphLoom mirrors only
the applicable `bool`, `null`, `int`, `float`, and `timestamp` resolver expressions. This includes
YAML 1.1 spellings such as `~`, `.inf`, `.nan`, binary/octal/hex and sexagesimal integers/floats,
and timestamps. YAML node kinds that JSON cannot express—tags, binary nodes, omap, pairs, sets,
anchors, and aliases—remain outside the emitter scope.

Clean ASCII values containing source newlines use PyYAML's single-quoted multiline style. Within
that style an automatic width break is one physical newline, one original source newline is encoded
as two physical newlines, and an original blank paragraph (`\n\n`) is encoded as three. Each
logical source line is independently width-wrapped with the scalar's mapping/sequence continuation
indent; quote doubling happens before width accounting. The fixture generator and Rust tests now
verify semantic YAML round trips across the complete corpus in addition to byte equality.

PyYAML's `Emitter.write_single_quoted` unconditionally calls `write_indent()` after emitting a
source break group, including when that group ends the scalar. Closing quotes after trailing
newlines therefore sit at the scalar continuation indent. One, two, and three trailing source
newlines produce two, three, and four physical line breaks respectively, followed by indentation
and the closing quote.

The remaining implicit resolvers relevant to string values are the merge key `<<` and value marker
`=` (`Resolver.yaml_implicit_resolvers` under `<` and `=`). PyYAML's complete named escape table is
also load-bearing for cache bytes: `\0`, `\a`, `\b`, `\t`, `\n`, `\v`, `\f`, `\r`, `\e`, `\"`,
`\\`, `\N`, `\_`, `\L`, and `\P`; other non-printable code points use upper-case hexadecimal
`\x`, `\u`, or `\U` forms. The corpus includes C0/C1 controls, DEL, NBSP, source trailing breaks,
and nested trailing-break positions.

PyYAML 6.0.3 decides whether a block mapping key is simple in
`yaml.emitter.Emitter.check_simple_key`. The prepared anchor, tag, and scalar event lengths must
sum to strictly less than 128; the scalar must also be non-empty and non-multiline. A JSON object
key is a string scalar whose prepared implicit `!!str` tag contributes five characters, so 122
Unicode scalar values remain simple while 123 require explicit `? key` / `: value` syntax. This
calculation uses Python string length (Unicode scalar count), not UTF-8 bytes or the length of the
escaped YAML representation.

The GraphLoom emitter classifies mapping-key layout separately from scalar style. Both simple and
explicit keys use the existing scalar formatter; simple keys suppress width wrapping as PyYAML's
simple-key context does. For explicit keys, the writer emits `? ` first and then derives the
scalar's initial column from the real output position, so the indicator's two columns participate
in width-80 wrapping; the continuation indent remains two columns inside the mapping. A shared
mapping-entry writer handles ordinary mappings and mappings inside sequences, including PyYAML's
compact `: nested: value` and `: - item` forms. The 173-case
corpus covers root, nested, JSON Schema, tool parameter, and sequence mappings with empty,
multiline, trailing-newline, oversized, Unicode, escaped-control, and mixed sorted keys.

Double-quoted wrapping must preserve the signed projection used by PyYAML's
`self.column + (end - start)`. After an escaped character, PyYAML sets `start = end + 1`, making
that delta `-1`. Rust's former `end.saturating_sub(start)` changed it to zero and wrapped one
escape too early when using the scalar's real output column. GraphLoom now keeps the real column
and applies the signed-equivalent projection with checked ordering; the 173-case corpus includes
38/39/40-BEL boundary cases plus nested and sequence variants.

## Open questions

None for the implementation scope. Streaming, Anthropic, and metrics aggregation remain explicitly
out of scope.
