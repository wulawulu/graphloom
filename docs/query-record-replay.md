# Query LLM record/replay compatibility

This local workflow compares GraphRAG 3.1.0 and GraphLoom without allowing LLM randomness to hide
or invent compatibility. An OpenAI-compatible proxy uses a normalized semantic request as its cache
key. A miss calls LiteLLM and durably stores the successful response; a hit returns the same
response. Request matching and final-answer comparison remain test-runner responsibilities.

The fixed inputs are:

```text
GraphRAG  -> ../graphrag/debug
GraphLoom -> ./debug
DeepSeek API key -> ./debug/.env:GRAPHRAG_API_KEY
```

Run all four non-streaming methods with one question and a new case name:

```bash
make query-record-replay \
  CASE=jinpingmei-01 \
  QUERY='西门庆和武松之间有什么联系？'
```

Run only one method with `METHOD=basic`, `local`, `global`, or `drift`. Output is ignored by Git and
written to `debug/query-record-replay/<CASE>/<METHOD>/`:

```text
cache.jsonl
graphrag-requests.jsonl
graphloom-requests.jsonl
graphrag.stdout / graphrag.stderr
graphloom.stdout / graphloom.stderr
report.json
```

`report.json` reports `requests.matchEqual`, `requests.exactEqual`, every raw difference, and stdout
equality. Semantic matching compares a multiset, so harmless completion order differences do not
fail a concurrent run; an order-only difference is still recorded as `$.requestOrder`.
`affectsMatch` distinguishes semantic differences from ignored transport/options fields. If
requests differ, the answer comparison remains observable but cannot isolate local post-LLM logic.
Consumer settings live only in an automatically deleted temporary directory and retain their
configured concurrency. Authorization headers and API keys are never written to cache, request
transcripts, reports, stdout, or proxy errors.

`indexArtifactPresence` lists the Parquet filenames on both sides and highlights files present on
only one side. It is a diagnostic precondition, not a claim that same-named Parquet files contain
identical logical rows.

The Make target exits non-zero when either request or answer comparison fails; this is a compatibility
finding, not necessarily a proxy or provider failure. Use the two recorded process exit codes in the
report to distinguish a comparison failure from an execution failure.

The proxy itself can also be started independently:

```bash
make llm-cache-proxy \
  CASSETTE=debug/query-record-replay/manual/cache.jsonl \
  COMPLETION_PROVIDER=deepseek \
  EMBEDDING_PROVIDER=ollama \
  EMBEDDING_API_BASE=http://localhost:11434
```

The match view deliberately stays small. Chat keys contain endpoint, model, `messages`, and whether
streaming is actually enabled; embedding keys contain endpoint, model, `input`, and the same stream
flag. Missing `stream` and `stream: false` are equivalent. Fields such as `encoding_format`,
`response_format`, `temperature`, `top_p`, and `n` remain in the raw transcript but do not split the
cache. Message text, whitespace, Unicode, roles, message order, embedding input, model, endpoint,
and `stream: true` remain significant.

DRIFT's HYDE prompt embeds a randomly selected community report. Only that bounded random template
slot is replaced by a marker in the match view; the query and fixed prompt text remain significant.
All original content is retained in JSONL and its difference is reported with `affectsMatch: false`.
Different keys may call LiteLLM concurrently. Concurrent misses for the same key are coalesced into
one upstream call.

Provider adaptation happens only after the request has been keyed and observed. In particular,
unsupported embedding fields may be dropped by LiteLLM, and DeepSeek `json_schema` is sent upstream
as `json_object`; the cassette and comparison transcript still retain the original request.

The two debug directories must represent equivalent indexes. The runner intentionally does not hide
context differences caused by different artifacts or settings. For example, a `covariates.parquet`
present on only one side can change Local/DRIFT token budgeting and therefore the resulting
`messages`; that remains a semantic incompatibility finding for the chosen test inputs.
