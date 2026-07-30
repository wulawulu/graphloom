# 研究：GraphRAG v4 LLM 缓存互操作

状态：已完成 · 维护者：graphloom · 日期：2026-07-12 · 源码固定：
`../graphrag` @ `79ab7c9ad586856e82635264c200d8a1eb3c63d9`

## 研究原因

GraphLoom 需要 provider-neutral completion/embedding 类型，其 JSON 表示、
cache key、namespace 和 invalidation 行为可直接与当前 GraphRAG cache
互操作。

本研究只讨论缓存协议，不要求复刻 LLM 调用后的全部 Pandas 转换。实体摘要
连接差异及 GraphLoom 更强输出语义的原因见
[GraphRAG `extract_graph` 输出语义研究](study-graphrag-extract-graph-output-zh.md)。

## 架构图

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

## 热路径

1. Workflow 用每项配置的 `model_instance_name` 为 extraction、
   summarization、reporting、claims、embeddings 创建独立 child cache。
2. Cache middleware 绕过 streaming 与 mock call，对原始 kwargs hash；
   cache `response` 解码为 `LLMCompletionResponse` 或
   `LLMEmbeddingResponse`，provider `model_dump()` 与 metrics 一起存储。
3. Hash 前移除 `metrics`、stream/mock control、timeout、endpoint/auth、
   Azure token provider 和 `drop_params`。
4. Common hasher 使用 PyYAML `dump(..., sort_keys=True)` 和 SHA-256；
   GraphRAG 追加 `_v4`。
5. JsonCache 删除非法 UTF-8/JSON，返回外层 `result`，写入
   `{result: value}`。

## 关键数据结构

`LLMCompletionResponse` 扩展 OpenAI `ChatCompletion`，增加
`formatted_response` 并产生顶层 computed `content`。
`LLMEmbeddingResponse` 扩展 OpenAI embedding response，产生
`embeddings` 与 `first_embedding`。这些 computed field 及 provider-specific
nested field 出现在真实 fixture 中，必须在语义 round trip 后保留。

Completion fixture
`ragdebug/cache/extract_graph/04ad9d...e3e_v4` 含 choice/message extension、
reasoning content、详细 usage、顶层 computed field 和浮点 metrics。
Embedding fixture `ragdebug/cache/text_embedding/8428d...9_v4` 含五个
1024 维向量、详细 usage、computed field 和 metrics。

## 关键算法

Key 为 `sha256(PyYAML.dump(filtered_kwargs, sort_keys=True)) + "_v4"`。
Namespace/model instance 只用于目录路由，不进入 hash。缺失 kwargs 保持
缺失，不规范化为 null。

## 采用

- 明确核心字段并 flatten unknown 的 provider-neutral Rust request/response；
- completion/embedding 共用 `{response, metrics}` envelope；
- 在 raw provider 外包 cache middleware，model resolution 时选 child
  namespace；
- 非法 JSON/schema entry 删除并视为 miss，storage error 仍 fatal；
- 对 canonical request kwargs 生成 GraphRAG-compatible v4 key。

## 避免

- Workflow API 暴露 provider response type；
- 永久保留旧 GraphLoom 简化 cache payload adapter；
- 把 request-level namespace/model ID 注入 hash；
- workflow/operation-specific cache 读写。

## 兼容加固后续（2026-07-12）

固定 GraphRAG 调用 `yaml.dump(data, sort_keys=True)`，不覆盖 PyYAML 默认值。
本地解析为 PyYAML 6.0.3：block style、width 80、LF、`allow_unicode=None`
（转义非 ASCII）。兼容需要逐字节 PyYAML emission，而不只是 YAML 语义
等价。Fixture generator 记录 GraphRAG commit 与 PyYAML 版本，并以 31 个
边界 golden 驱动 Rust emitter。

GraphRAG cache middleware 的 sync/async 路径都使用
`kwargs.get("mock_response") or False`，这是 Python truthiness，不是只接受
bool。LiteLLM completion 把它声明为 `str | None`，所以非空字符串必须
bypass cache。

本地 `async-openai` 0.41.1 启用 `byot`。`create_byot` 接收任意
`Serialize` body，同时保留 client、auth、Tower retry、timeout、concurrency
stack。因此 GraphLoom 可传输经校验的 canonical `extra` 字段，无需替换
transport 或静默丢弃 unknown。

最终语料增加长 plain、single-quoted、double-quoted/multiline、sequence 和
nested mapping scalar。PyYAML width 80 是优选断行阈值：先输出当前 word，
再把之后合适的空格换成 newline，所以物理行可超过 80。Continuation indent
由 mapping/sequence nesting 决定。Python `json.loads` 还决定 float 在
PyYAML 前的表示：`1e7` 为 `10000000.0`，`1e-7` 为 `1.0e-07`，
`1e20` 为 `1.0e+20`。

GraphRAG 排除的 cache-key kwargs 混合 provider transport 配置和 middleware
control。它们在 raw kwargs 中对 key 兼容仍合法，但 GraphLoom OpenAI BYOT
adapter 必须在 HTTP serialize 前拒绝：`mock_response`、endpoint/auth、
timeout、Azure token provider、`drop_params`、`stream_options`、metrics。
不冲突的 provider body extension 保持开放并原样发送。

PyYAML plain-scalar indicator 规则区分无条件 prefix 与上下文 prefix。开头
quote、closing bracket/brace、comma、percent、anchor、alias、tag 总要
single quote；`-`、`?`、`:` 主要在后接空白时需要 quote；精确 `---`/`...`
特殊。重复空格有语义：wrap 只替换一个合适 separator，保留其余 alignment
space。70-case corpus 同时检查重复空格的字节与 YAML decode 等价。

Provider validation 必须先于 cache routing。`api_key` 等 client-only extra
被 GraphRAG hash 排除，与合法请求共用 key；若只在 provider builder 内
校验，非法请求可能 cache hit。Model trait 现提供 object-safe 同步 preflight；
cache wrapper 在 bypass/hash/lookup 前委托 inner model。OpenAI 覆盖 provider
规则，mock model 使用 canonical-only validation 并继续接受 `mock_response`。

PyYAML 6.0.3 通过 `yaml_implicit_resolvers` 选择隐式 scalar type。对于
JSON-origin string，GraphLoom 只镜像适用的 `bool`、`null`、`int`、`float`
和 `timestamp` regex，包括 YAML 1.1 的 `~`、`.inf`、`.nan`、binary/
octal/hex、sexagesimal number 和 timestamp。JSON 无法表达的 tag、binary
node、omap、pair、set、anchor、alias 不在 emitter scope。

含源码 newline 的 clean ASCII 使用 PyYAML single-quoted multiline style。
自动 width break 是一个物理 newline；一个原始 newline 编码为两个，原始
blank paragraph `\n\n` 编码为三个。每个逻辑行按 scalar nesting indent
独立 wrap；quote doubling 在 width accounting 前发生。Generator 与 Rust
测试除字节外还验证完整语料的 semantic YAML round trip。

`Emitter.write_single_quoted` 在 source break group 后无条件调用
`write_indent()`，包括 group 位于 scalar 尾部时。Trailing newline 后的
closing quote 因此位于 continuation indent。一个、两个、三个尾部源码
newline 分别产生两个、三个、四个物理 line break，再输出 indent 和 quote。

String value 相关的剩余 implicit resolver 是 merge key `<<` 与 value marker
`=`。PyYAML 完整 named escape table 也影响 cache 字节：`\0`、`\a`、`\b`、
`\t`、`\n`、`\v`、`\f`、`\r`、`\e`、`\"`、`\\`、`\N`、`\_`、`\L`、
`\P`；其他不可打印 code point 使用大写十六进制 `\x`、`\u`、`\U`。
语料覆盖 C0/C1、DEL、NBSP、源码尾部 break 及 nested 位置。

PyYAML 6.0.3 用 `Emitter.check_simple_key` 判断 block mapping key 是否
simple。准备后的 anchor、tag、scalar event length 总和必须严格小于 128，
且 scalar 非空、非 multiline。JSON object key 是 string scalar，隐式
`!!str` tag 占五个字符，所以 122 个 Unicode scalar 仍 simple，123 个则
需要显式 `? key`/`: value`。计算用 Python string length，而非 UTF-8
字节或 escaped YAML 长度。

GraphLoom emitter 分开判断 mapping-key layout 与 scalar style。Simple 与
explicit key 都使用同一 formatter；simple-key context 像 PyYAML 一样禁止
width wrap。Explicit key 先写 `? `，再从真实输出 column 推导 scalar 初始
column，让两列 indicator 参与 width-80 wrap；continuation indent 仍位于
mapping 内两列。共享 mapping-entry writer 处理普通 mapping 与 sequence
中的 mapping，包括 compact `: nested: value` 和 `: - item`。173-case
corpus 覆盖 root/nested/JSON Schema/tool parameter/sequence 的 empty、
multiline、trailing-newline、oversized、Unicode、control 和 mixed key。

Double-quoted wrap 必须保留 PyYAML `self.column + (end - start)` 的有符号
projection。Escaped char 后 `start = end + 1`，delta 为 `-1`。Rust 原先
`end.saturating_sub(start)` 把它变成 0，在真实 column 下提前一个 escape
wrap。当前实现保留真实 column 并按 checked ordering 计算有符号等价值；
173-case corpus 含 38/39/40-BEL 边界与 nested/sequence 变体。

## 开放问题

实现范围内没有。Streaming、Anthropic 和 metrics aggregation 明确不在
范围内。
