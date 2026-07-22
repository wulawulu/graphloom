# Query LLM record/replay proxy 设计

状态：已实施 v2 · 依赖：[PRD](query-record-replay-prd.md) · 基线：GraphRAG 3.1.0

## 1. 职责边界

```text
GraphRAG / GraphLoom                Local caching proxy                 LiteLLM
        │                                   │                              │
        │ POST OpenAI JSON ────────────────▶│ canonical request key        │
        │                                   │                              │
        │                         hit ───────┤                              │
        │◀──────── saved status/body ───────│                              │
        │                                   │                              │
        │                         miss       │ completion / embedding ─────▶│
        │                                   │◀──── provider response ──────│
        │                                   │ append+fsync cassette         │
        │◀──────────────────────────────────│                              │

Test runner owns: per-run request snapshots → semantic multiset match + raw diff → stdout comparison.
```

Proxy 只拥有 cache、single-flight 和请求观测。matching 结论属于 runner。

## 2. Cache key 与记录

Key 为 semantic match view 的 SHA-256。Chat view 保留 endpoint、model、messages 和有效 stream 布尔值；
embedding view 保留 endpoint、model、input 和有效 stream 布尔值。缺失 stream 与 false 等价，true 仍区分 SSE。
其他选项不参与 key，但完整原始 body 仍进入 cassette/observation。DRIFT HYDE prompt 只统一随机 community
template 槽，query 和固定文本不变。Header 不参与 key。Cassette 每行保存版本、key、endpoint、完整 request
body、response status/content-type/body；observation 另存 exact key；二进制 body 使用 Base64。只接受 chat
completions 与 embeddings。

每个进入 proxy 的请求另存运行期 observation，其中含 ordinal、key、endpoint、body 和 hit/miss；不含 headers。

## 3. 并发

cache map 和 in-flight map 分离。首个同 key miss 成为 leader；followers 只等待该 key 的结果。不同 key 不共享
锁并可并发访问 LiteLLM。只有完整成功响应进入 cache；失败唤醒 followers 并返回相同错误，不缓存失败。

## 4. Provider 与 streaming

Proxy 使用 GraphRAG 3.1.0 锁定的 LiteLLM 1.86.2，而不升级到当前 1.93.0，以免改变固定兼容基线。completion
按 `provider/model` 调用 DeepSeek；embedding 按 debug settings 调用 Ollama。原始 request 在 adaptation 前完成 key 和
observation；LiteLLM 丢弃目标 provider 不支持的参数，并把 DeepSeek 不支持的 `json_schema` 降级为其支持的
`json_object`。流式 miss 收集 LiteLLM chunks，编码为 OpenAI SSE 后持久化；hit 重放同一 SSE body。

## 5. 安全和限制

遵循 AGENTS.md 的输入边界：loopback-only、16 MiB request、64 MiB response、4096 cache entries、JSON depth/
collection/string caps、未知 endpoint 拒绝、provider timeout。Bearer token 只传入 LiteLLM 调用，不进入 `Debug`、
错误或持久化。cassette 固定写入 ignored `debug/query-record-replay/`，拒绝覆盖和 symlink 逃逸。

## 6. 测试协议

每个 method 使用独立空 cache：先 GraphRAG，再 GraphLoom。两边 consumer view 只重写 model `api_base`、将
embedding transport 统一为 OpenAI-compatible，并保留原并发配置；真实 Parquet/LanceDB 仍来自各自 debug。
Runner 以 normalized key 的多重集合判断语义相同，避免并发完成顺序造成误报；exact sequence、顺序变化和所有
raw field diff 仍记录。随后比较 stdout。若请求不同，答案结果仍报告，但不把答案差异归因于本地算法。

## 7. 工程约束

这是测试工具而非 Rust 公共 API；Rust 类型/API、unsafe、actor 等条款 N/A。Python 错误必须带上下文且不得泄密；
测试覆盖 hit/miss、同 key single-flight、不同 key 并发、stream replay、损坏 cassette、semantic match、
顺序独立比较和 raw request diff。

## 8. 参考

- [GraphRAG v4 LLM cache interoperability](../docs/research/study-graphrag-llm-cache.md)
- [Phase 2 Query compatibility](phase2.md)
