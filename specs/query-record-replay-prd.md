# PRD — Query LLM record/replay compatibility

状态：已实施 v2 · 维护者：graphloom · 日期：2026-07-22

## 1. 问题

真实 LLM 对相同输入仍可能返回不同文本，因此分别运行 GraphRAG 3.1.0 与 GraphLoom 后直接比较最终答案，
无法区分模型随机性和 Query 实现差异。Global 与 DRIFT 又会把中间响应用于构造后续请求，随机差异会级联。

## 2. 目标

提供一个本地 OpenAI-compatible caching proxy。请求的 model 与 messages/input 语义相同时，两端获得同一份
真实 DeepSeek/Ollama 响应；不影响内容的可选参数差异保留记录但不拆分 cache。测试端独立记录并先比较请求，
再比较最终答案。

## 3. 成功标准

- cache miss 只调用一次 LiteLLM，成功响应持久化为 ignored JSONL；cache hit 原样返回响应；
- 不同 key 并发执行，同 key 并发 miss single-flight；
- Authorization/API key 不进入日志、cassette 或报告；
- 使用 `debug/` 与 `../graphrag/debug/` 完成 Basic、Local、Global、DRIFT 非流式真实测试；
- 每个方法报告 normalized 请求多重集合是否相同、exact sequence 是否相同、所有 raw 差异、cache hit/miss
  和最终 stdout 是否相同。

## 4. 非目标

- proxy 不判断兼容性，也不伪造 Global/DRIFT 响应；只生成用于 cache 的窄 semantic view，raw 请求不改写；
- 不把 debug index、cassette、完整 prompt 或答案加入 Git/CI；
- 不代理任意用户 URL；upstream provider 来自固定 debug settings；
- 不以最终答案相同替代请求比较。

## 5. 用户入口

一个 Make target 运行固定双端验证；proxy 同时提供独立 CLI，便于人工接入。DeepSeek API key 只从
`debug/.env` 读取。
