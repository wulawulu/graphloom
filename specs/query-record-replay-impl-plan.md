# Query LLM record/replay 实施计划

状态：已完成 v2 · 依赖：[设计](query-record-replay-design.md)

## Phase 1 — Cache proxy

实现 semantic key、独立 exact key、versioned JSONL cassette、LiteLLM backend、per-key single-flight、OpenAI
JSON/SSE server 和专项测试。退出条件：并发测试证明同 key 一次 upstream、不同 key 可并行，且 secret 不落盘。

## Phase 2 — Query compatibility runner

实现固定 debug consumer views、GraphRAG/GraphLoom 顺序执行、normalized request multiset comparison、raw
sequence diff、stdout comparison、JSON summary 和 Make target。退出条件：无真实 provider 时可用 fake backend
完成端到端测试，并发请求仅因完成顺序不同不会误判。

## Phase 3 — Real debug verification

从 `debug/.env` 读取 DeepSeek key，分别运行 Basic、Local、Global、DRIFT。退出条件：四种方法都有完整报告，
明确区分 request mismatch、answer mismatch 和 provider/test failure。

## Verification

- `make test-query-record-replay`
- Python Ruff format/check；
- `make test-compat`，因为 compatibility Python surface 发生变化；
- Rust Query prompt compatibility 修复需运行完整 Rust gate。
