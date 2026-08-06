# GraphLoom Explainability 与 Studio 架构设计

## 1. 文档状态

* 状态：设计草案
* 适用项目：GraphLoom
* 兼容基线：`graphrag-3.1.0-compat-v1`
* 目标阶段：GraphLoom Explainability 与 Studio 实现
* 最后更新：2026-08-03

本文定义 GraphLoom 在以下方面的职责边界和实现方向：

1. GraphLoom Lib 的可观测性能力；
2. GraphLoom CLI 的日志与 OpenTelemetry 支持；
3. Query Explainability 数据模型；
4. 图谱浏览所需的数据访问能力；
5. Studio 的实时展示与历史回放；
6. Explainability 数据的持久化与存储抽象。

本文不修改 GraphRAG 兼容行为，也不处理现有兼容性优化清单中的行为改进。

---

# 2. 背景与目标

GraphLoom 当前既是：

* 一个可直接运行的 CLI 应用；
* 一个可被其他 Rust 应用调用的 Library；
* 一个 GraphRAG 兼容的 Index、Update 和 Query 引擎。

后续计划实现 GraphLoom Studio。

Studio 的主要界面包括：

```text
┌──────────────────────────┬────────────────────────────────────┐
│ 左侧：Query Chat         │ 右侧：Graph Explorer              │
│                          │                                    │
│ 问题输入                 │ Entity ───── Relationship          │
│ 流式回答                 │    │                               │
│ Context                  │ Community                          │
│ Explainability Timeline  │                                    │
│                          │ Query 期间动态高亮节点和关系       │
└──────────────────────────┴────────────────────────────────────┘
```

Studio 应同时支持：

## 2.1 图谱浏览

浏览 Index 后生成的：

* Document；
* Text Unit；
* Entity；
* Relationship；
* Community；
* Community Report；
* Covariate。

## 2.2 Query Explainability

用户执行 Local、Global、DRIFT 等 Query 时，展示：

* Query 如何被处理；
  -哪些 Entity 被向量召回；
  -哪些候选被过滤；
  -哪些 Entity 最终入选；
  -沿哪些 Relationship 扩展；
  -哪些 Community Report 被使用；
  -哪些 Text Unit 进入 Context；
  -每个 Context section 分配了多少 token；
  -哪些记录因 token budget 被裁剪；
  -最终 Context 如何形成；
  -最终 Prompt 和 Answer 如何产生。

## 2.3 实时与离线

Studio 既要支持：

* 用户在界面提问时实时展示 Query 过程；
* 查看以前执行过的 Query；
  -重放历史 Query 的 Explainability 轨迹；
  -恢复当时图谱节点和关系的高亮状态。

---

# 3. 设计原则

GraphLoom Explainability 遵循以下原则：

1. **Lib 产生信息，宿主应用决定如何输出。**
2. **日志和 OpenTelemetry 共享同一套 `tracing` 插桩。**
3. **运行监控与业务解释分离。**
4. **Explainability 不能依赖解析文本日志。**
5. **OpenTelemetry 不作为完整图谱数据传输协议。**
6. **实时展示和离线回放使用同一套事件。**
7. **Studio 只依赖公开 DTO，不依赖内部 DataFrame。**
8. **不开启 Explainability 时应接近零额外成本。**
9. **Explainability 不得改变 GraphRAG 兼容行为。**
10. **先展示兼容行为，再根据证据决定如何优化。**
11. **第一阶段完整支持 Local Query，再逐步扩展。**
12. **存储通过业务接口抽象，不绑定具体数据库。**

---

# 4. 三类能力的区别

GraphLoom 需要区分三个相关但不同的概念。

## 4.1 Logging

Logging 面向开发者和运维人员，用于回答：

> 程序当前发生了什么？

例如：

```text
INFO query started method=local
INFO entity retrieval completed candidates=20 selected=10
WARN entity_description contains a stale entity id
INFO query completed elapsed_ms=850
```

日志特点：

* 面向人阅读；
  -可以是文本或 JSON；
  -格式可能调整；
  -不保证包含完整业务决策；
  -不适合作为 Studio 的稳定数据源。

GraphLoom 使用 Rust `tracing` 产生日志事件。

---

## 4.2 Operational Observability

Operational Observability 用于回答：

> GraphLoom 运行得怎么样？

包括：

* Workflow 耗时；
* LLM 请求耗时；
* Embedding 请求耗时；
* token 使用；
* cache hit/miss；
* retry；
  -错误；
  -候选数量；
  -选中数量；
* Context token 数；
  -存储和向量操作耗时。

实现方式：

```text
tracing Span / Event
        │
        ├── fmt Layer
        │       └── 控制台或文件日志
        │
        └── OpenTelemetry Layer
                └── OTLP Collector
```

日志和 OpenTelemetry 共享同一套内部插桩，不维护两套独立事件。

---

## 4.3 Explainability

Explainability 用于回答：

> 这次 Query 的答案具体是如何构造出来的？

包括：

-具体召回了哪些 Entity；
-每个候选的 score 和 rank；
-哪些候选被选中；
-哪些候选被排除；
-排除原因是什么；
-沿哪些 Relationship 扩展；
-哪些 Community Report 被使用；
-哪些 Text Unit 被使用；

* Context section 如何分配 token；
  -哪些记录最终进入 Context；
  -哪些记录被截断；
  -最终 Prompt 和 Answer 如何形成。

Explainability 数据面向程序和 Studio，因此必须：

-结构化；
-可序列化；
-有 Schema Version；
-可实时消费；
-可持久化；
-可离线回放；
-具有稳定语义。

---

# 5. 总体架构

```text
┌───────────────────────────────────────────────────────────────┐
│                        GraphLoom Lib                          │
│                                                               │
│  Index / Update / Query                                       │
│            │                                                  │
│            ├── tracing Span / Event                           │
│            │       ├── Logging                                │
│            │       └── OpenTelemetry                          │
│            │                                                  │
│            ├── ExplainabilityRecord                           │
│            │       ├── Noop Sink                              │
│            │       ├── JSONL Sink                             │
│            │       ├── Studio Sink                            │
│            │       └── Third-party Sink                       │
│            │                                                  │
│            └── GraphDataSource                                │
│                    └── Graph Explorer Data                    │
└───────────────────────────────────────────────────────────────┘
                    ▲                         ▲
                    │                         │
         ┌──────────┴──────────┐   ┌─────────┴───────────┐
         │   GraphLoom CLI     │   │ GraphLoom Studio    │
         │                     │   │                     │
         │ Logging             │   │ Graph Explorer      │
         │ OTLP Exporter       │   │ Query Chat          │
         │ Explainability JSONL│   │ Live Explainability │
         └─────────────────────┘   │ Run History         │
                                   │ Offline Replay      │
                                   └─────────────────────┘
```

依赖方向必须保持：

```text
GraphLoom Studio
        ↓
GraphLoom Lib
```

GraphLoom Lib 不得依赖：

* Studio；
* Axum；
* WebSocket；
* SSE；
  -浏览器前端；
  -SQLite；
  -Turso；
  -DuckDB；
  -具体 OpenTelemetry 后端。

---

# 6. GraphLoom Lib 的职责

GraphLoom Lib 负责：

* 执行 Index、Update、Query 和 Prompt Tune；
  -在关键操作中创建 `tracing` Span 和 Event；
  -产生结构化 `ExplainabilityEvent`；
  -提供 `ExplainabilitySink`；
  -提供 `GraphDataSource`；
  -提供 No-op 默认实现；
  -保证不开启 Explainability 时行为不变。

GraphLoom Lib 不负责：

* 初始化全局 tracing subscriber；
  -决定日志格式；
  -决定日志输出路径；
  -决定 OTLP endpoint；
  -启动 HTTP 服务；
  -启动 SSE 服务；
  -管理 Studio 历史记录；
  -选择 SQLite、Turso 或 DuckDB；
  -渲染图谱。

Library 中禁止直接执行：

```rust
tracing_subscriber::fmt().init();
```

Subscriber 必须由 CLI、Studio 或其他宿主应用初始化。

---

# 7. GraphLoom CLI 的职责

GraphLoom CLI 是 GraphLoom Lib 的一个宿主应用。

CLI 负责：

* 初始化 tracing subscriber；
  -配置日志等级；
  -配置文本日志或 JSON 日志；
  -可选输出日志文件；
  -可选初始化 OpenTelemetry exporter；
  -可选输出 Explainability JSONL；
  -在退出前完成必要的 flush。

CLI 不重新实现：

* Index；
  -Update；
  -Query；
  -Explainability 业务逻辑；
  -GraphDataSource。

推荐的 CLI 能力：

```bash
graphloom query \
  --root ./demo \
  --method local \
  "问题"
```

日志：

```bash
graphloom query \
  --log-level info \
  --log-format text \
  "问题"
```

JSON 日志：

```bash
graphloom query \
  --log-format json \
  --log-file ./logs/query.jsonl \
  "问题"
```

OpenTelemetry：

```bash
graphloom query \
  --otel-endpoint http://127.0.0.1:4317 \
  --otel-service-name graphloom \
  "问题"
```

Explainability：

```bash
graphloom query \
  --explain-output ./runs/query-001.jsonl \
  --explain-content metadata \
  "问题"
```

组合使用：

```bash
graphloom query \
  --log-format json \
  --otel-endpoint http://127.0.0.1:4317 \
  --explain-output ./runs/query-001.jsonl \
  --explain-content metadata \
  "问题"
```

---

# 8. GraphLoom Studio 的职责

GraphLoom Studio 同样是 GraphLoom Lib 的宿主应用。

Studio 负责：

* 调用 GraphLoom Query API；
  -通过 `GraphDataSource` 读取图谱；
  -通过 `ExplainabilitySink` 消费事件；
  -持久化 Explainability Run 和 Event；
  -通过 SSE 实时推送事件；
  -列出历史 Query；
  -加载历史事件；
  -离线回放；
  -展示图谱、Context 和最终回答。

Studio 可以自行初始化：

```text
tracing Subscriber
        ├── Console Logging
        ├── File Logging
        └── OpenTelemetry Layer
```

Studio 不改变 GraphLoom Lib 的业务行为。

---

# 9. Logging 与 OpenTelemetry

## 9.1 共用 tracing 插桩

GraphLoom 内部统一使用 `tracing`：

```rust
#[tracing::instrument(
    name = "graphloom.query.local.retrieve_entities",
    skip_all,
    fields(
        graphloom.query.method = "local",
        graphloom.retrieval.top_k = top_k,
    )
)]
async fn retrieve_entities(...) -> Result<...> {
    // ...
}
```

宿主应用可以同时安装多个 Layer：

```rust
tracing_subscriber::registry()
    .with(format_layer)
    .with(optional_opentelemetry_layer)
    .init();
```

同一个 Span 可以同时：

-显示在控制台；
-写入 JSON 日志；
-导出到 OpenTelemetry Collector。

---

## 9.2 Span 命名规范

Span 名称使用稳定、低基数、点分隔形式：

```text
graphloom.index.run
graphloom.index.workflow
graphloom.update.run

graphloom.query.basic
graphloom.query.local
graphloom.query.global
graphloom.query.dynamic_global
graphloom.query.drift

graphloom.query.runtime
graphloom.query.entity_mapping
graphloom.query.graph_expansion
graphloom.query.context
graphloom.query.prompt

graphloom.llm.request
graphloom.embedding.request
graphloom.vector.search

graphloom.storage.read
graphloom.storage.write
graphloom.vector.write
```

其中 `graphloom.query.basic`、`graphloom.query.global`、`graphloom.query.dynamic_global`、
`graphloom.query.drift`、`graphloom.storage.*`、`graphloom.vector.write`、Index/Update/Prompt
Tune 的详细 Span 属于后续扩展，尚未实现。当前只实现了 Local Query 的详细 Span。

不得将以下动态值写入 Span 名称：

* 用户问题；
  -Entity ID；
  -Relationship ID；
  -文件路径；
  -模型响应；
  -workflow 参数。

动态值应作为 Span 字段或 Event 字段记录。

---

## 9.3 推荐 tracing 字段

```text
graphloom.observability.version
graphloom.run.id
graphloom.operation
graphloom.query.method
graphloom.query.streaming
graphloom.explainability.enabled

graphloom.model.instance
graphloom.model.provider

graphloom.vector.index
graphloom.retrieval.top_k

graphloom.input.count
graphloom.input.tokens
graphloom.output.tokens
graphloom.context.tokens
graphloom.embedding.dimensions

graphloom.candidate.count
graphloom.selected.count
graphloom.llm.calls

graphloom.status
graphloom.error.kind
graphloom.elapsed_ms
```

`graphloom.cache.hit`、`graphloom.retry.attempt`、`graphloom.workflow.name` 属于后续扩展，
当前 Local Query 合同不包含。

OpenTelemetry Adapter 可以将通用字段映射到：

```text
gen_ai.*
openinference.*
```

GraphLoom 内部字段不应直接绑定 Langfuse、Phoenix 或其他平台的私有 Schema。

---

## 9.4 日志内容限制

默认日志只记录概要：

```text
candidate_count=20
selected_count=10
context_tokens=931
cache_hit=true
```

默认不得记录：

* 完整用户问题；
  -完整 Text Unit；
  -完整 Entity description；
  -完整 Prompt；
  -完整 Context；
  -完整模型回答；
  -API Key；
  -Authorization Header；
  -Cookie；
  -环境变量值。

---

## 9.5 Observability 合同 Version 1（已实现）

GraphLoom 公开稳定的 Observability 合同，定义在 `crates/graphloom/src/observability.rs`，
从 `graphloom::observability` 公开。该模块只定义合同，不初始化任何 subscriber。

### 9.5.1 Contract Version

```text
OBSERVABILITY_CONTRACT_VERSION = 1
```

版本升级规则：

* 新增可选 Span、Event 或 Field 不要求升级；
  -删除稳定字段要求升级；
  -重命名稳定字段要求升级；
  -改变字段值的语义或类型要求升级；
  -改变父子 Span 的核心含义要求升级。

### 9.5.2 Span 名称（已实现）

```text
graphloom.query.local
graphloom.query.runtime
graphloom.query.context
graphloom.query.entity_mapping
graphloom.embedding.request
graphloom.vector.search
graphloom.query.graph_expansion
graphloom.query.prompt
graphloom.llm.request
```

### 9.5.3 Event 名称（已实现）

```text
graphloom.cli.query.started
graphloom.cli.query.completed
graphloom.cli.query.failed
graphloom.cli.explainability.enabled
graphloom.cli.explainability.shutdown_failed
graphloom.cli.telemetry.enabled
graphloom.cli.telemetry.shutdown_failed

graphloom.query.explainability.delivery_failed
graphloom.query.explainability.contract_failed
graphloom.query.explainability.sidecar_incomplete
graphloom.query.explainability.finish_failed
```

`graphloom.query.explainability.*` 是 Local Explainability 自身的 operational warning，
不是 Explainability 业务事件。

### 9.5.4 Field 名称与类型

```text
graphloom.observability.version   u64
graphloom.run.id                  string
graphloom.operation               string
graphloom.query.method            string
graphloom.query.streaming         bool
graphloom.explainability.enabled  bool
graphloom.telemetry.enabled       bool
graphloom.model.instance          string
graphloom.model.provider          string
graphloom.vector.index            string
graphloom.retrieval.top_k         u64
graphloom.input.count             u64
graphloom.input.tokens            u64
graphloom.output.tokens           u64
graphloom.context.tokens          u64
graphloom.embedding.dimensions    u64
graphloom.candidate.count         u64
graphloom.selected.count          u64
graphloom.llm.calls               u64
graphloom.status                  string
graphloom.error.kind              string
graphloom.elapsed_ms              u64
```

`usize` 不直接进入合同；转换为 `u64` 失败时省略字段，不用 0 伪装未知值，也不让 Query 失败。

### 9.5.5 Status 值

```text
ok
error
abandoned
```

### 9.5.6 Operation 值

```text
query
runtime_load
context_build
entity_mapping
embedding
vector_search
graph_expansion
prompt_render
completion
```

Operation 是算法阶段，不使用函数名、模型名或 Query 内容。

### 9.5.7 Local Query 父子 Span（已实现）

```text
graphloom.query.local
├── graphloom.query.runtime
├── graphloom.query.context
│   ├── graphloom.query.entity_mapping
│   │   ├── graphloom.embedding.request
│   │   └── graphloom.vector.search
│   └── graphloom.query.graph_expansion
├── graphloom.query.prompt
└── graphloom.llm.request
```

父子关系通过真实 `tracing` parent/child 实现：

* `graphloom.query.local` 在请求入口创建，覆盖 project root 校验、runtime lookup、context、
  prompt 与 provider 请求；一个请求只创建一个；
  -`graphloom.query.runtime` 覆盖 root 校验、Local requirements 校验、runtime cache
  lookup/build、table/prompt/vector/model 装配；
  -`graphloom.query.context` 覆盖一次真实 `build_explainable`；
  -`graphloom.query.entity_mapping` 覆盖 mapping query 构造、embedding/rank 分支、ANN 解析、
  stale reference 过滤与 include/exclude；
  -`graphloom.embedding.request` 只在实际调用 `embedding_model.embed(...)` 时创建；
  -`graphloom.vector.search` 只在实际调用 `vector_store.search(...)` 时创建；
  -`graphloom.query.graph_expansion` 覆盖 Relationship/Covariate 收集、progressive expansion、
  token fitting 与 rollback；
  -`graphloom.query.prompt` 覆盖 Prompt bind、render、completion input token 计数与请求校验；
  -`graphloom.llm.request` 从 provider stream handshake 开始，保持到 stream 完整消费结束。

### 9.5.8 生命周期与终态

根 Span 使用同步、原子、幂等的终态门闩，不依赖 Explainability 的异步 `finish_run()`：

* 完整成功：`status = ok`，记录 input/output/context tokens、llm calls、elapsed ms；
  -业务错误：`status = error`，记录稳定 `error.kind`；
  -Stream 中途错误：`status = error`，`error.kind = query_completion`；
  -Stream 正常结束但没有 `Completed`：按业务语义记录 `error.kind = query_completion`；
  -Stream 提前 drop：根 Span 与 LLM Span 同步记录 `status = abandoned`，不 spawn 隐藏任务、
  不调用 Explainability finish、不生成 RunCompleted/RunFailed、不改变 QueryEvent。

LLM Span 覆盖 provider stream handshake 与真实 stream polling：每次 `poll_next()` 都运行在
`graphloom.llm.request` Span 中（通过 `tracing::Instrument`，不使用跨 `.await` 的 enter
guard），因此 Provider stream 内部产生的 Event/子 Span 自动成为 LLM Span 的子节点。
`Completed` 到达前不会提前关闭，`Completed` 后记录 `ok` 与 token 使用。

LLM/Root tracing Span 在 Explainability 投递与 JSONL flush 之前真正 close：

```text
LLM tracing Span 终态（取走并 drop Span 句柄）
→ Local 根 tracing Span 终态（取走并 drop Span 句柄）
→ Explainability LlmRequestCompleted / RunCompleted / RunFailed
→ Explainability finish_run / JSONL flush
```

tracing Span duration 不包含 JSONL flush。所有阶段 Span 在返回路径上闭合。

根 Span 的 `graphloom.elapsed_ms` 语义：

```text
graphloom.query.local elapsed_ms
    = 收到请求级 QueryOptions 后，到 tracing 业务终态的完整生命周期

QueryResult.elapsed
    = 现有 Local Query 执行计时，兼容语义不变
```

成功、失败与 abandoned 统一使用 Session request start 到业务终态的计时，
不再使用 `QueryResult.elapsed` 作为根 Span elapsed。

Entity mapping 忽略 stale vector reference 时产生 named Event
`graphloom.query.entity_mapping.stale_reference`（`error.kind = stale_reference`），
tracing 不记录具体 Entity/Vector/文档 ID；stale ID 只通过 Explainability 内容通道表达。

CLI `graphloom.cli.query.completed` 与 `graphloom.cli.query.failed` 互斥：streaming 路径在
terminal newline 与 stdout flush 全部成功后才记录 completed；stdout 失败时只记录 failed。
任何 Recorder shutdown failure（无论 Query 成败）都产生一次
`graphloom.cli.explainability.shutdown_failed`。

### 9.5.9 tracing 与 Explainability 分离

`tracing` Span/Event 与 `ExplainabilityRecord`/Envelope 是两个独立通道：

* tracing 不依赖 Explainability；Explainability 也不依赖 tracing；
  -run ID 只在调用方提供 `QueryOptions.explainability` 时写入 `graphloom.run.id`，用于关联；
  -不通过 Explainability Event 反推 tracing 字段，也不从 JSONL Recorder 生成 tracing；
  -tracing 失败或缺少 subscriber 不影响 Query；
  -`graphloom.query.explainability.*` warning 不是 Explainability 业务事件。

### 9.5.10 CLI named events

CLI Query 命令日志改用稳定 named event，保留人类可读 message：

```text
graphloom.cli.query.started
graphloom.cli.query.completed
graphloom.cli.query.failed
graphloom.cli.explainability.enabled
graphloom.cli.explainability.shutdown_failed
```

CLI 的 `query.log` 使用稳定字段（`graphloom.query.method`、`graphloom.query.streaming`、
`graphloom.status` 等），不写完整错误文本，不写 Explainability output path，不写 API Key。
最终用户可见错误仍由 CLI 主程序输出到 stderr。

### 9.5.11 当前实现范围与后续扩展

当前只有 Local Query 详细 Span 已实现。以下内容尚未实现，不得描述为可用：

* Basic、Global、DRIFT 的详细 Core Span；
  -Index、Update、Prompt Tune 的 tracing 重构；
  -graphloom-llm 通用 Provider wrapper 重构。

JSON 日志 CLI 参数与 OpenLIT 接入尚未实现。OpenTelemetry Layer 以本合同的
Span/Event/Field 作为稳定输入；CLI 的 OTLP/HTTP Trace Adapter 见 9.6。

---

# 9.6 OTLP/HTTP Trace Adapter（已实现）

`graphloom query` 现在可以把 Local Query 的 `tracing` Span 通过
OTLP/HTTP binary protobuf 导出到 OpenTelemetry Collector。

## 9.6.1 传输与协议

* 只实现 Trace，不实现 Metrics、Logs；
  -固定使用 OTLP/HTTP binary protobuf（`application/x-protobuf`）；
  -不实现 OTLP/gRPC、OTLP/JSON；
  -不启用压缩、自动重试、Baggage、Trace Context HTTP 传播；
  -HTTP 客户端使用官方 `reqwest-blocking-client`（Batch 工作线程内阻塞导出，
  不在 Query async 热路径执行 HTTP 请求），TLS 使用 `reqwest-rustls-webpki-roots`；
  -当前依赖版本：

```text
opentelemetry 0.32.0
opentelemetry_sdk 0.32.1
opentelemetry-otlp 0.32.0
tracing-opentelemetry 0.33.0
```

`opentelemetry-otlp` 的 `http-proto` feature 会编译该 crate 的 proto/metrics 类型，
但 GraphLoom 不创建 Meter Provider，也不导出任何 Metrics。

## 9.6.2 CLI 参数

```text
--otel-endpoint <OTEL_ENDPOINT>
--otel-service-name <OTEL_SERVICE_NAME>
```

* `--otel-endpoint` 是 Collector base endpoint；只有显式指定时才启用 OTLP。
  Adapter 按 OTLP HTTP 规则追加 `/v1/traces`；
  -未指定时完全不创建 exporter、不启动 batch 工作线程、不发起网络请求，
  Query 行为与性能保持原样；
  -只有 `--method local` 合法；Basic、Global、DRIFT 携带 endpoint 时在参数校验
  阶段以 exit code 2 拒绝；
  -`--otel-service-name` 依赖 `--otel-endpoint`，有效默认值为 `graphloom`；
  拒绝空字符串、纯空白与超过 128 字节的值；
  -endpoint、service name、timeout 均不会进入日志、Span 或错误 Display。

示例：

```bash
graphloom query \
  --root ./demo \
  --method local \
  --otel-endpoint http://localhost:4318 \
  --otel-service-name graphloom-demo \
  "问题"
```

与 Explainability 同时启用：

```bash
graphloom query \
  --root ./demo \
  --method local \
  --explain-output ./runs/query.jsonl \
  --explain-content metadata \
  --otel-endpoint http://localhost:4318 \
  --otel-service-name graphloom-demo \
  "问题"
```

两通道职责不同：

```text
OTLP Trace
    → 运行性能、父子 Span、token/count/status

Explainability JSONL
    → Candidate、选择原因、Context 决策与可回放内容
```

## 9.6.3 Resource 与 Instrumentation Scope

Resource 使用 SDK `Resource::builder()`（保留 `telemetry.sdk.*` 等默认属性），并固定包含：

```text
service.name = <--otel-service-name 或 graphloom>
service.version = CARGO_PKG_VERSION
graphloom.observability.version = 1
```

不记录 project root、cwd、Query、output path、endpoint、API base URL、用户账号或主机名。

Instrumentation Scope：

```text
name = graphloom
version = CARGO_PKG_VERSION
```

## 9.6.4 Batch Span Processor 与 Layer Filter

生产路径使用官方 `SdkTracerProvider::builder().with_batch_exporter(exporter)`：

* Span close 不等待网络导出，已完成 Span 进入 SDK 有界 batch queue；
  -不新增 queue size、batch size、schedule delay、sampler、retry 参数；
  -不使用 SimpleSpanProcessor 作为生产实现；
  -不使用实验性 async-runtime BatchSpanProcessor。

`tracing-opentelemetry` Layer 持有显式 tracer，不设置全局 OTel Provider：

```rust
tracing_opentelemetry::layer()
    .with_tracer(tracer)
    .with_filter(EnvFilter::new("off,graphloom::query=info"))
```

只导出 `graphloom::query` 及其子 target（含
`graphloom.query.local`、`graphloom.query.runtime`、`graphloom.query.context`、
`graphloom.query.entity_mapping`、`graphloom.embedding.request`、
`graphloom.vector.search`、`graphloom.query.graph_expansion`、
`graphloom.query.prompt`、`graphloom.llm.request`）。`opentelemetry`、`reqwest`、
`hyper`、`rustls` 与依赖库日志不会被再次导出，避免递归。

## 9.6.5 生命周期与错误语义

```text
load project config
→ 初始化 file/console/可选 OTLP subscriber
→ 创建可选 Explainability Recorder
→ 执行 Query
→ shutdown Explainability Recorder
→ force-flush + shutdown OTLP provider（spawn_blocking）
→ 合并 Query/Recorder/Telemetry outcome
→ drop query.log WorkerGuard
```

* Query Core Span 在 Query 返回前关闭；OTLP flush 发生在全部 Query Span close 后；
  -Recorder creation 失败时也会显式关闭 OTLP provider；
  -没有早期 `?` 跳过 shutdown；
  -force flush、shutdown 与 shutdown task join 的失败都返回聚焦的
  `GraphLoomError::Telemetry`，Display 不包含 endpoint、Header、Token 或
  response body；
  -错误优先级固定为：Query 业务错误 > Explainability Recorder 错误 >
  Telemetry flush/shutdown 错误；Telemetry 失败不会覆盖业务 Query 错误。

初始化成功时产生一次 `graphloom.cli.telemetry.enabled`
（`graphloom.observability.version=1`、`graphloom.telemetry.enabled=true`）；
force flush 或 shutdown 失败产生一次 `graphloom.cli.telemetry.shutdown_failed`
（`graphloom.error.kind=telemetry_output`）。`OBSERVABILITY_CONTRACT_VERSION`
保持 1，新增 Event/Field/error kind 均是可选项。

## 9.6.6 内容安全

OTLP Trace 与 `query.log` 均不包含：

```text
Query、Prompt、Context、Response
endpoint、Header、Token
output path、Entity/Vector ID
```

标准 OTLP Header 环境变量（如 `OTEL_EXPORTER_OTLP_HEADERS`、
`OTEL_EXPORTER_OTLP_TRACES_HEADERS`）由官方 exporter 读取并发送；GraphLoom
自己不读取、不记录、不复制这些值。Explainability Content 通道仍按原合同包含
允许的内容。

## 9.6.7 未实现

Metrics、Logs、OTLP/gRPC、OTLP/JSON、压缩、自动重试、Trace Context HTTP
传播、Baggage、SSE、Studio、OpenLIT、Collector 部署、Basic/Global/DRIFT 详细
Span、Index/Update/Prompt Tune Span 重构均未实现。

---

# 10. Explainability 核心接口

## 10.1 ExplainabilitySink

第一阶段在 `graphloom` crate 内定义：

```rust
#[async_trait::async_trait]
pub trait ExplainabilitySink: Send + Sync + std::fmt::Debug {
    async fn emit(
        &self,
        record: Arc<ExplainabilityRecord>,
    ) -> Result<(), ExplainabilitySinkError>;

    async fn finish_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<(), ExplainabilitySinkError>;
}
```

Core 在事件发生时创建 `ExplainabilityRecord`，因此 Sink 收到的记录已经具有完整的
业务身份和 Span 父子关系。`emit` 接收共享所有权的不可变 Record，可以异步等待有界
Adapter 队列的入队容量；成功返回表示 Adapter 已可靠接受该 Record，队列满时不得
静默丢弃或伪装成功。等待只发生在入队容量上，文件、数据库和网络 I/O 由独立单写者
执行，不得在 Tokio worker 上使用 `blocking_send` 或直接执行阻塞 I/O。

`finish_run` 表示该 Run 不再产生新 Record，并等待该 Run 已接受事件完成必要的写入、
flush 或最终确认。后台 writer、flush 或持久化失败必须通过
`ExplainabilitySinkError` 返回；该方法不改变 `ExplainabilityRunStatus`，也不自动产生
`RunCompleted`，Run 的业务完成事件仍由 Core 产生。

该 Trait 需要作为 `Arc<dyn ExplainabilitySink>` 动态分发，因此使用 `async-trait` 保持
对象安全。Sink 不得 panic，所有可恢复的关闭、不可用、未接受、writer 和完成确认错误
都使用安全、结构化的错误类别报告。

默认实现和有序 fan-out 实现：

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopExplainabilitySink;

#[async_trait::async_trait]
impl ExplainabilitySink for NoopExplainabilitySink {
    async fn emit(
        &self,
        _record: Arc<ExplainabilityRecord>,
    ) -> Result<(), ExplainabilitySinkError> {
        Ok(())
    }

    async fn finish_run(
        &self,
        _run_id: &ExplainabilityRunId,
    ) -> Result<(), ExplainabilitySinkError> {
        Ok(())
    }
}

pub struct ExplainabilitySinkChain {
    sinks: Vec<Arc<dyn ExplainabilitySink>>,
}
```

调用方不传入 Sink 时使用 No-op 实现。Chain 按注册顺序串行 await 所有 Sink，共享同一
`Arc<ExplainabilityRecord>` 而不深度复制候选数据；一个 Sink 失败后仍调用其余 Sink，
最终错误按稳定 Sink 索引聚合 emit 或 finish 的全部失败。空 Chain 成功。

Core Explainability 只定义可靠输入合同，不提供会静默丢事件的 Best Effort delivery
mode。运行时采样属于 `tracing` / OpenTelemetry；持久化后的实时广播属于 Live Hub / SSE。

## 10.2 Query 请求级配置

Local Query 运行时通过 `QueryOptions.explainability` 接收可选的请求级配置：

```rust
pub struct QueryExplainabilityOptions {
    run_id: ExplainabilityRunId,
    content_mode: ExplainabilityContentMode,
    sink: Arc<dyn ExplainabilitySink>,
}
```

调用方可以使用 `QueryExplainabilityOptions::new` 提前提供 `run_id`，也可以使用
`generated` 生成便利 ID。Studio 因而可以先创建历史 Run、建立该 ID 的浏览器订阅，再调用
GraphLoom；Core 不强制隐藏或独占 Run ID 的生成。

Explainability 属于请求状态，不进入 `QueryEngine` 的长期资源缓存。缓存用的 resource
options 会清除 callbacks、conversation history 和 Explainability 配置；query text、run ID、
content mode、sink、投递失败计数和流状态都不会被缓存。共享同一 warm Local runtime 的并发
Query 各自创建 Session、Span ID 和失败状态，因此不会串 Sink 或 Run。

Local Run 的请求边界从 GraphLoom 已收到方法为 Local 且带 Explainability 的
`QueryOptions` 开始：先创建并启动请求级 Session，再校验项目根目录、构建或获取 runtime。
因此 root mismatch、必需表或 vector index 缺失、schema、prompt、model 或 runtime 构建失败
同样产生安全低基数的 `RunFailed`，并尝试一次 `finish_run`。`QueryEngine::load(config, root)`
本身尚未收到 `QueryOptions` 和调用方 run ID，所以单独调用 load 不创建 Run；one-shot API 在
调用 load 前已经持有 options，因而由其请求编排层覆盖 load 失败。

---

## 10.3 谁负责生成 Envelope

GraphLoom Core 负责产生业务记录：

```rust
pub struct ExplainabilityRecord {
    pub run_id: ExplainabilityRunId,
    pub timestamp: DateTime<Utc>,
    pub span_id: ExplainabilitySpanId,
    pub parent_span_id: Option<ExplainabilitySpanId>,
    pub event: ExplainabilityEvent,
}
```

其中：

* `run_id` 来自本次请求的 `QueryExplainabilityOptions`，便利调用方也可在请求前生成；
  -`span_id` 和 `parent_span_id` 由 Core 根据真实业务阶段创建；
  -`timestamp` 是业务事件发生时间；
  -`event` 是结构化业务事件。

宿主侧 Adapter 的独立单写者按照实际持久化顺序补充：

* `schema_version`；
  -`sequence`。

`sequence` 在一个 `run_id` 内从 1 开始并严格递增。它不能由并发 Query 任务预先
分配，也不能假设任务产生顺序等于 channel 到达或持久化顺序。

推荐的数据模型是：

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExplainabilityEnvelope {
    schema_version: u32,
    sequence: NonZeroU64,
    pub record: ExplainabilityRecord,
}

impl ExplainabilityEnvelope {
    pub fn new(sequence: u64, record: ExplainabilityRecord) -> Result<Self, ExplainabilityContractError>;
    pub const fn schema_version(&self) -> u32;
    pub const fn sequence(&self) -> u64;
}
```

这里使用嵌套 `record`，避免重复存储 run、span、timestamp 或 event。最终 Envelope
同时用于 JSONL、Store、SSE 实时消费和离线回放；实时消费者不得另造一份缺少
`sequence` 的传输事件。

`schema_version` 和 `sequence` 是只读的 writer 合同字段：构造函数拒绝零序号，
离线反序列化也必须经过相同的 Schema Version 和非零序号校验，不能通过公开字段
绕过不变量。

字段含义：

* `schema_version`：事件 Schema 版本；
  -`sequence`：同一次 Run 内严格递增且不为 0 的持久化序号；
  -`record`：Core 产生且未经修改的完整业务记录。

事件顺序必须以 `sequence` 为准。

时间戳不能作为唯一排序依据。

---

# 11. Explainability Run

除了事件，还需要保存一次运行的元数据。

```rust
pub struct ExplainabilityRun {
    pub run_id: ExplainabilityRunId,

    pub kind: ExplainabilityRunKind,
    pub status: ExplainabilityRunStatus,

    pub query: Option<String>,
    pub query_method: Option<ExplainabilityQueryMethod>,

    pub started_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,

    pub compatibility_profile: Option<String>,

    pub event_count: u64,
}
```

```rust
pub enum ExplainabilityRunKind {
    Index,
    Update,
    Query,
    PromptTune,
}
```

```rust
pub enum ExplainabilityRunStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}
```

Studio 可以根据 Run 数据展示历史：

```text
今天
├── 西门庆与哪些人形成主要利益关系？     Local
├── 潘金莲与武松是什么关系？             DRIFT
└── 西门家的主要权力结构是什么？         Global
```

---

# 12. Explainability Event

## 12.1 第一阶段事件

第一阶段完整覆盖 Local Query。

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExplainabilityEvent {
    RunStarted(RunStarted),
    RunCompleted(RunCompleted),
    RunFailed(RunFailed),

    QueryStarted(QueryStarted),
    MappingQueryBuilt(MappingQueryBuilt),

    EmbeddingStarted(EmbeddingStarted),
    EmbeddingCompleted(EmbeddingCompleted),

    CandidatesRetrieved(CandidatesRetrieved),
    CandidatesFiltered(CandidatesFiltered),
    EntitiesSelected(EntitiesSelected),

    GraphExpansionStarted(GraphExpansionStarted),
    RelationshipsSelected(RelationshipsSelected),
    CommunityReportsSelected(CommunityReportsSelected),
    CovariatesSelected(CovariatesSelected),
    TextUnitsSelected(TextUnitsSelected),

    ContextBudgetAllocated(ContextBudgetAllocated),
    ContextSectionBuilt(ContextSectionBuilt),
    ContextCompleted(ContextCompleted),

    LlmRequestStarted(LlmRequestStarted),
    LlmRequestCompleted(LlmRequestCompleted),

    Warning(ExplainabilityWarning),
}
```

后续扩展：

* Basic Query；
  -Global Query；
  -Dynamic Global Query；
  -DRIFT；
  -Index；
  -Update；
  -Prompt Tune。

---

## 12.2 Span 与 Event

有持续时间、可能失败、需要统计耗时的操作应使用 Span，例如：

```text
query.local
embed_mapping_query
retrieve_entities
expand_graph
build_context
generate_answer
```

操作内部瞬时发生的业务事实使用 Event，例如：

```text
candidate_selected
candidate_rejected
stale_reference_skipped
token_budget_exhausted
cache_hit
```

一个 Span 可以对应多个 Explainability Event。

---

# 13. Explainability DTO

Explainability 不能直接暴露：

* Polars DataFrame；
  -LanceDB 类型；
  -内部 Entity struct；
  -内部借用对象；
  -内部 iterator；
  -Provider 原始响应。

必须使用稳定 DTO。

持久化合同在 Serde 边界同时限制读写规模：短元数据字符串最多 256 bytes，安全错误或
警告消息最多 4 KiB，显式开启的 query/context/prompt/response 内容最多 1 MiB；单个
Event 最多包含 10,000 个 Candidate 或记录 ID，以及 32 个 Context section budget。
这些限制不依赖未来 Web body limit，JSONL 和 Store 离线读取同样执行验证。

## 13.1 Record Type

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplainabilityRecordType {
    Document,
    TextUnit,
    Entity,
    Relationship,
    Community,
    CommunityReport,
    Covariate,
}
```

---

## 13.2 Candidate

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExplainabilityCandidate {
    pub id: String,
    pub short_id: Option<String>,
    pub title: Option<String>,

    pub record_type: ExplainabilityRecordType,

    pub score: Option<ExplainabilityScore>,
    pub rank: Option<u32>,

    pub selected: bool,
    pub reason: Option<SelectionReason>,

    pub source_id: Option<String>,
    pub relationship_id: Option<String>,
    pub expansion_depth: Option<u32>,
}
```

`CandidatesRetrieved` 和 `CandidatesFiltered` 保留外层 `record_type`，并将 Candidate
列表定义为同质集合：每个 `candidate.record_type` 必须与外层集合类型相同。外层字段
使空结果仍能表达本次检索或过滤的目标类型。两种 Payload 使用 fallible constructor
在构造时验证，并在 Serde 反序列化时通过相同不变量验证；不一致数据属于 Schema 1 中
原本无效的状态，不需要提升 Schema Version。构造后的 `record_type` 和 `candidates`
为私有字段，只提供只读 getter，不能通过公共可变访问制造矛盾状态。

---

## 13.3 Selection Reason

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionReason {
    AnnResult,
    ExplicitlyIncluded,
    ExplicitlyExcluded,

    GraphExpansion,
    CommunityMembership,
    SourceReference,

    RankThreshold,
    TokenBudget,

    StaleReference,
    MissingRecord,
}
```

第一阶段不要求所有算法都产生同样的 Reason。

Reason 应反映当前真实行为，而不是为了 UI 美观推断不存在的原因。

---

## 13.4 Context Section

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExplainabilityContextSection {
    pub section: ContextSectionKind,
    pub name: Option<String>,

    pub token_budget: u64,
    pub tokens_used: u64,

    pub candidate_count: u64,
    pub selected_count: u64,

    pub truncated: bool,

    pub selected_record_ids: Vec<String>,
}
```

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSectionKind {
    ConversationHistory,
    CommunityReports,
    LocalGraph,
    Entities,
    Relationships,
    Covariates,
    Sources,
    MapContext,
    ReduceContext,
}
```

`name` 只用于区分低基数的逻辑子组，例如多个 Covariate group；它是可选、受长度限制的
Schema 1 增量字段。`LocalGraph` 表达 Entities、Relationships 和 Covariates 实际共享的
Local budget，避免在 `ContextBudgetAllocated` 中把同一份预算虚构成三份独立预算。该语义
补全发生在 Explainability 尚未持久化发布的基础合同阶段，因此 Schema Version 仍为 1。

---

# 14. Explainability 内容级别

```rust
#[derive(Debug, Clone, Copy)]
pub enum ExplainabilityContentMode {
    Metadata,
    Content,
    Debug,
}
```

## 14.1 Metadata

默认模式，记录：

* ID；
  -类型；
  -数量；
  -score；
  -rank；
  -token；
  -耗时；
  -selected；
  -selection reason；
  -model；
  -operation。

不记录完整业务内容。

## 14.2 Content

额外记录：

* 用户问题；
  -Entity description；
  -Text Unit 内容；
  -最终 Context；
  -Prompt；
  -模型回答。

## 14.3 Debug

额外记录：

* 完整候选；
  -中间排序结果；
  -provider 请求的非敏感字段；
  -provider 响应的非敏感字段；
  -内部转换结果；
  -兼容性调试信息。

任何模式都不得记录：

* API Key；
  -Authorization Header；
  -Cookie；
  -完整环境变量；
  -Secret；
  -密码；
  -访问令牌。

---

# 15. GraphDataSource

## 15.1 目的

Explainability Event 只描述一次 Run 使用了哪些数据。

Studio 还需要在没有运行 Query 时浏览完整图谱，因此需要独立的数据访问接口。

```rust
#[async_trait::async_trait]
pub trait GraphDataSource: Send + Sync + std::fmt::Debug {
    async fn entities(
        &self,
        query: EntityQuery,
    ) -> Result<Vec<EntityView>>;

    async fn relationships(
        &self,
        query: RelationshipQuery,
    ) -> Result<Vec<RelationshipView>>;

    async fn communities(
        &self,
        query: CommunityQuery,
    ) -> Result<Vec<CommunityView>>;

    async fn community_reports(
        &self,
        query: CommunityReportQuery,
    ) -> Result<Vec<CommunityReportView>>;

    async fn text_units(
        &self,
        ids: &[String],
    ) -> Result<Vec<TextUnitView>>;
}
```

第一阶段可基于现有 Table Provider 和 Parquet 读取实现。

---

## 15.2 Entity View

```rust
pub struct EntityView {
    pub id: String,
    pub short_id: String,

    pub title: String,
    pub entity_type: Option<String>,
    pub description: Option<String>,

    pub rank: Option<u64>,
    pub degree: Option<u64>,

    pub community_ids: Vec<String>,
}
```

---

## 15.3 Relationship View

```rust
pub struct RelationshipView {
    pub id: String,
    pub short_id: String,

    pub source_id: Option<String>,
    pub target_id: Option<String>,

    pub source_title: String,
    pub target_title: String,

    pub description: Option<String>,
    pub weight: Option<f64>,
    pub rank: Option<u64>,
}
```

Studio 不得直接读取内部 DataFrame 后自行猜测列语义。

---

# 16. Local Query Explainability 流程

推荐事件流程：

```text
RunStarted
    ↓
QueryStarted
    ↓
MappingQueryBuilt
    ↓
EmbeddingStarted
    ↓
EmbeddingCompleted
    ↓
CandidatesRetrieved
    ↓
CandidatesFiltered
    ↓
EntitiesSelected
    ↓
GraphExpansionStarted
    ↓
ContextBudgetAllocated
    ├── CommunityReportsSelected
    ├── RelationshipsSelected
    ├── CovariatesSelected
    └── TextUnitsSelected
    ↓
ContextSectionBuilt × N
    ↓
ContextCompleted
    ↓
LlmRequestStarted
    ↓
LlmRequestCompleted
    ↓
RunCompleted
```

实际 Span 身份为：

```text
local_query
├── entity_mapping
│   ├── embedding
│   └── entity_retrieval
├── graph_expansion
├── context_construction
└── llm_completion
```

Embedding 事件只在非空 mapping query 确实调用模型时出现；空 mapping query 的 rank
fallback 仍产生真实的 retrieved/filtered/selected 事件。Selection 和 section sidecar 在
排序、引用解析、include/exclude、rank filter、token fitting、当前 entity 回退与最终 prefix
接受的位置同步捕获，不解析最终文本或 DataFrame 反推。`ContextCompleted.context` 复用实际
`QueryContextText::Text`，LLM prompt 记录实际渲染的 Local system prompt，而不是 Provider
完整请求对象。

Progressive Local Graph expansion 回滚时，以最后一次 accepted section 的完整 Candidate
快照和 Context 为最终真相；failed attempt 相对 accepted 新出现的 occurrence 追加为未选择的
`token_budget`，已有的 `rank_threshold` / `missing_record` 原因保持不变。重复 ID 按出现顺序
逐个消费而不按 Set 合并，Covariate 按 `kind + name` 独立匹配。发出前从最终 Candidate 顺序
重新推导 selected flags 对应的 `selected_count` 与 `selected_record_ids`，使其与最终 Context
行顺序一致。Community section 实际输出空列表 `"[]"` 时，仅在 Explainability 开启时复用当前
tokenizer 统计真实 tokens；统计失败只标记 sidecar 不完整并省略不可靠的 section capture，
不会改变 `"[]"`、Query 或 Usage。

每个业务事件只调用一次 Sink，不自动重试。Chain 的部分成功不会触发整条 Chain 重试，
后续不同事件仍继续投递。Sink emit 失败只标记 Explainability Run 不完整，不改变 Context、
Provider 请求、QueryEvent、Answer 或 Usage；业务成功但曾有投递失败时，终态为安全的
`RunFailed(error_kind = "explainability_delivery")`。业务 Query 错误产生低基数、安全消息的
`RunFailed`，原始 `QueryError` 原样返回。终态只尝试一次，随后 `finish_run` 只调用一次；
终态或 finish 投递失败均不改变业务结果，也不重试。

Local streaming 只包装共享的 completion event stream：Context、Token、Completed 和 callback
顺序保持不变。完整消费到 Completed 或 Err 时产生终态并 finish；调用方提前 drop stream 时
不在 Drop 中执行异步工作、不 spawn 隐藏任务，Run 暂时保持未完成，等待后续 Store/Studio
阶段通过超时或 abandoned 状态处理。

当前只有 Local Query 接入运行时 Explainability。Basic、Global 和 DRIFT 即使收到请求配置
也不会产生 Local 事件。JSONL Recorder、bounded channel Adapter 和每 Run sequence allocator
已经实现；SQLite、Turso、DuckDB、Store、SSE、Studio 和 OpenTelemetry 仍属于后续阶段，
尚未实现。

前端可据此处理：

```text
CandidatesRetrieved
→ 候选节点浅色高亮

EntitiesSelected
→ 最终实体强高亮

RelationshipsSelected
→ 高亮图中的边

CommunityReportsSelected
→ 高亮社区或社区边界

TextUnitsSelected
→ 展示来源文本

ContextSectionBuilt
→ 展示 token budget 和裁剪结果

ContextCompleted
→ 展示最终 Context

LlmRequestCompleted
→ 展示最终回答
```

---

# 17. 实时通信协议

## 17.1 第一版使用 SSE

GraphLoom Studio 第一版采用：

```text
HTTP + Server-Sent Events
```

而不是 WebSocket。

交互模式是：

```text
浏览器 ──POST Query──→ Studio
浏览器 ←──SSE Events── Studio
```

这属于典型的后端单向持续推送。

SSE 的优势：

* 基于普通 HTTP；
  -浏览器原生支持；
  -适合单向事件流；
  -支持自动重连；
  -支持 `Last-Event-ID`；
  -更容易经过反向代理；
  -服务端实现简单；
  -JSON 文本足以表达 Explainability Event。

WebSocket 暂不作为第一阶段要求。

未来出现以下需求时再评估 WebSocket：

* 同一连接中频繁双向控制；
  -多人协作；
  -远程交互式调试；
  -高频二进制流；
  -复杂连接复用。

---

## 17.2 推荐 HTTP API

提交 Query：

```http
POST /api/queries
```

返回：

```json
{
  "run_id": "01J..."
}
```

订阅事件：

```http
GET /api/runs/{run_id}/events
Accept: text/event-stream
```

列出历史 Run：

```http
GET /api/runs
```

获取 Run：

```http
GET /api/runs/{run_id}
```

读取历史事件：

```http
GET /api/runs/{run_id}/events?after_sequence=0
```

取消 Query：

```http
POST /api/runs/{run_id}/cancel
```

删除历史：

```http
DELETE /api/runs/{run_id}
```

取消、删除等客户端控制使用普通 HTTP，不需要因此引入 WebSocket。

---

## 17.3 SSE 事件格式

`sequence` 作为 SSE 的 Event ID：

```text
id: 12
event: entities_selected
data: {"schema_version":1,"sequence":12,"record":{"run_id":"01J...",...}}
```

浏览器断线重连时可以发送：

```http
Last-Event-ID: 12
```

服务端应补发：

```text
sequence > 12
```

的事件。

---

# 18. 实时与离线共用同一事件流

实时展示和离线回放不能设计成两套机制。

推荐数据流：

```text
GraphLoom Core
      ↓ await ExplainabilitySink::emit
bounded adapter queue
      ↓
single writer
      ↓ 分配 sequence
生成并持久化 ExplainabilityEnvelope
      ↓
Store
      ├── 历史回放
      └── Live Hub / SSE 实时展示
```

核心原则：

> 实时模式是边持久化边消费，离线模式是从存储重新消费同一事件。

事件 Schema、顺序和语义必须完全相同。

`GraphLoom Core → ExplainabilitySink → 持久化单写者 → Store` 是可靠、可背压、错误
可见的边界。`Store → Live Hub / SSE 客户端` 位于持久化之后，可以发生暂时的实时
丢失；慢客户端或断线不影响 Store，客户端通过 Store 和 `Last-Event-ID` 补发恢复。

---

## 18.1 先持久化，再广播

推荐顺序：

```text
1. 接收 ExplainabilityRecord
2. 分配 sequence
3. 生成 ExplainabilityEnvelope
4. 写入 ExplainabilityStore
5. 持久化成功
6. 将同一个 Envelope 广播给实时订阅者
```

这样可以保证：

```text
用户实时看到的事件
=
以后离线能回放的事件
```

如果先广播、后持久化，进程崩溃时可能导致：

-用户已看到事件；
-数据库中没有事件；
-历史回放不完整。

---

## 18.2 浏览器连接竞态

Query 可能在浏览器建立 SSE 之前就已经产生事件。

因此 SSE 连接不能只订阅实时广播。

正确流程是：

```text
1. 订阅实时广播
2. 确定当前事件边界
3. 从 Store 读取历史事件
4. 按 sequence 发送历史事件
5. 继续发送实时广播事件
6. 使用 sequence 去重
```

客户端只接受：

```text
sequence > last_seen_sequence
```

避免重复。

---

## 18.3 前端只维护一个 Reducer

前端实时展示和离线回放使用同一个状态转换函数：

```typescript
function applyExplainabilityEvent(
  state: ExplainabilityState,
  event: ExplainabilityEnvelope,
): ExplainabilityState {
  // 更新图谱、Context、Timeline 和 Answer
}
```

实时模式：

```typescript
eventSource.onmessage = event => {
  state = applyExplainabilityEvent(
    state,
    JSON.parse(event.data),
  );
};
```

离线模式：

```typescript
for (const event of storedEvents) {
  state = applyExplainabilityEvent(state, event);
}
```

这样能保证历史回放和实时展示行为一致。

---

# 19. ExplainabilityStore

## 19.1 存储抽象

Studio 不应直接依赖 SQLite API。

应通过业务接口访问：

```rust
#[async_trait::async_trait]
pub trait ExplainabilityStore: Send + Sync + std::fmt::Debug {
    async fn create_run(
        &self,
        run: ExplainabilityRun,
    ) -> Result<()>;

    async fn append_events(
        &self,
        events: &[ExplainabilityEnvelope],
    ) -> Result<()>;

    async fn complete_run(
        &self,
        completion: RunCompletion,
    ) -> Result<()>;

    async fn get_run(
        &self,
        run_id: &str,
    ) -> Result<Option<ExplainabilityRun>>;

    async fn list_runs(
        &self,
        query: RunQuery,
    ) -> Result<Vec<ExplainabilityRun>>;

    async fn load_events(
        &self,
        run_id: &str,
        after_sequence: Option<u64>,
    ) -> Result<Vec<ExplainabilityEnvelope>>;

    async fn delete_run(
        &self,
        run_id: &str,
    ) -> Result<()>;
}
```

接口只暴露业务语义，不暴露数据库连接。

不推荐：

```rust
fn sqlite_connection(&self) -> &Connection;
```

---

# 20. 存储实现选择

## 20.1 SQLite

第一版 Studio 默认使用 SQLite。

适用场景：

* 单机 Studio；
  -一个 writer task；
  -实时小批量事件写入；
  -按 `run_id + sequence` 查询；
  -历史记录列表；
  -删除 Run；
  -本地零配置。

推荐实现：

```text
SqliteExplainabilityStore
```

第一版不需要复杂 ORM，可以使用轻量 SQL 访问层。

---

## 20.2 Turso

Turso 可以作为后续可选实现：

```text
TursoExplainabilityStore
```

适合：

* 多设备访问；
  -本地优先同步；
  -远程 Studio；
  -团队共享 Query History；
  -云端持久化；
  -离线运行后同步。

第一版暂不将 Turso 作为默认数据库，原因包括：

* 当前单机需求不需要同步；
  -增加认证和远程配置；
  -需要定义 push/pull；
  -需要处理网络错误；
  -需要处理同步状态；
  -需要明确冲突策略。

存储抽象必须保证未来引入 Turso 时，不影响：

* GraphLoom Lib；
  -ExplainabilityEvent；
  -SSE；
  -前端 Reducer。

---

## 20.3 DuckDB

DuckDB 不作为第一版实时 Event Store。

它更适合 Explainability Analytics，例如：

* 分析不同 Query Method 的平均延迟；
  -统计 token 使用；
  -分析 Context section；
  -比较 compatible 和 optimized mode；
  -分析大量 JSONL；
  -直接查询 Parquet；
  -生成 Benchmark 报表。

推荐定位：

```text
SQLite / Turso
    └── Run 与 Event 主存储

DuckDB
    └── 跨 Run 分析和 Benchmark
```

未来可以提供：

```text
DuckDbExplainabilityAnalytics
```

而不是将 DuckDB 作为实时历史回放的默认后端。

---

## 20.4 JSONL

CLI 使用 JSONL 作为 Explainability 输出格式。

```bash
graphloom query \
  --root ./demo \
  --method local \
  --explain-output ./runs/local-query.jsonl \
  --explain-content metadata \
  "问题"
```

当前实现结构为：

```text
JsonlExplainabilityRecorder
├── JsonlExplainabilitySink
│   └── bounded mpsc queue（默认容量 256，可由库调用方配置）
└── single writer task
    ├── 每 Run 从 1 开始分配严格递增 sequence
    ├── ExplainabilityEnvelope::new
    └── compact JSON + LF → write_all → flush
```

`--explain-output` 当前只允许 Local Query；`--explain-content` 支持 `metadata`、`content`
和 `debug`，省略时采用 `metadata`。CLI 在项目配置与日志初始化成功后创建 Recorder、生成
run ID 并提交 `QueryOptions`；因此 settings、`.env` 或配置解析失败不会创建 Run。Query、
stream 消费或 stdout 失败后，CLI 仍显式调用 `shutdown()`；Query 与 Recorder 同时失败时，
原 Query 错误保持主错误，Recorder 错误进入不含用户内容的安全日志。

输出路径相对 CLI 进程当前工作目录解析，必要父目录会创建，文件使用 `create_new`：既不
覆盖，也不 truncate 或跨进程 append 已存在文件。每个 Envelope 是一个紧凑 JSON 对象并
固定追加单个 LF 字节，不写 BOM、数组或 tracing 文本。writer 每写完一行即 flush，
`finish_run()` 在确认该 Run 之前再次 flush，Recorder `shutdown()` 处理此前已接受 command、
再次 flush 并等待 writer task。这里不执行 `fsync` / `sync_data`；正常 finish/shutdown 后已
接受事件均已进入底层异步文件，进程突然终止时只保证保留已经完成写入的 JSONL 前缀，
极端情况下不完整的末行应被 Reader 视为损坏而不是猜测修复。

stream 被调用方提前 drop 时不伪造终态；shutdown 仍写完并 flush 已接受事件，JSONL 会如实
保留未完成 Run 前缀。Recorder 不提供文件轮转、压缩、远程上传、离线播放器或 append。

优点：

* 可逐行流式写入；
  -进程崩溃后保留已写事件；
  -便于 diff；
  -便于作为测试 fixture；
  -脚本容易读取；
  -不引入数据库依赖。

JSONL 不要求承担 Studio 的历史查询、分页和同步功能。

---

# 21. SQLite Schema

第一版 Studio 可以使用两张表。

## 21.1 Runs

```sql
CREATE TABLE explainability_runs (
    run_id TEXT PRIMARY KEY,

    kind TEXT NOT NULL,
    status TEXT NOT NULL,

    query TEXT,
    query_method TEXT,

    started_at TEXT NOT NULL,
    completed_at TEXT,

    compatibility_profile TEXT,

    event_count INTEGER NOT NULL DEFAULT 0
);
```

推荐索引：

```sql
CREATE INDEX explainability_runs_by_started_at
ON explainability_runs(started_at DESC);
```

---

## 21.2 Events

```sql
CREATE TABLE explainability_events (
    run_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,

    span_id TEXT NOT NULL,
    parent_span_id TEXT,

    timestamp TEXT NOT NULL,
    event_type TEXT NOT NULL,

    payload_json TEXT NOT NULL,

    PRIMARY KEY (run_id, sequence),

    FOREIGN KEY (run_id)
        REFERENCES explainability_runs(run_id)
        ON DELETE CASCADE
);
```

推荐索引：

```sql
CREATE INDEX explainability_events_by_run
ON explainability_events(run_id, sequence);
```

第一版不需要将所有 Event 字段拆成列。

完整 Event 保存在：

```text
payload_json
```

中即可。

---

# 22. Studio Explainability Service

推荐内部结构：

```text
graphloom-studio
├── ExplainabilityService
│   ├── ExplainabilityStore
│   ├── LiveExplainabilityHub
│   └── StudioExplainabilitySink
│
├── HTTP API
│   ├── POST /api/queries
│   ├── GET /api/runs
│   ├── GET /api/runs/{id}
│   ├── GET /api/runs/{id}/events
│   ├── POST /api/runs/{id}/cancel
│   └── DELETE /api/runs/{id}
│
└── Frontend
    ├── Graph Explorer
    ├── Query Chat
    ├── Live Explainability
    ├── Run History
    └── Offline Replay
```

Sink 示例：

```rust
pub struct StudioExplainabilitySink {
    sender: tokio::sync::mpsc::Sender<ExplainabilityCommand>,
}
```

Writer task：

```text
mpsc receiver
    ↓
分配 sequence
    ↓
生成 ExplainabilityEnvelope
    ↓
写入 ExplainabilityStore
    ↓
broadcast::Sender
    ↓
SSE subscribers
```

---

# 23. 同步、背压与错误处理

Explainability Sink 不得在 Query 热路径中执行阻塞 I/O。

推荐使用：

```text
GraphLoom Core
      ↓ await ExplainabilitySink::emit
bounded adapter queue
      ↓
single writer
      ↓
分配 sequence 并生成 ExplainabilityEnvelope
      ↓
Store / JSONL
      ↓ 持久化成功后
Live Hub / SSE
```

## 23.1 Core 到 Store 的可靠投递

`ExplainabilitySink` 始终是可靠输入合同：

* `emit` 可以异步等待 bounded queue 容量，以明确背压保持资源上界；
  -成功只表示 Adapter 已可靠接受 Record，不表示直接等待了磁盘或数据库 I/O；
  -Queue Full、Closed、Unavailable 或 writer failure 不得使用 panic 表达；
  -无法接受时返回显式 `ExplainabilitySinkError`，不得静默丢失；
  -`finish_run` 确认该 Run 已接受事件完成必要持久化或 flush；
  -writer、flush 和完成确认错误必须返回调用方。

已实现的 CLI JSONL 和未来的 Studio 本地 SQLite 均建立在这一可靠边界上。Core Sink 不提供
`BestEffort` 模式，也不允许同一个 Sink 在不通知调用方时降级为丢事件。

## 23.2 持久化后的可恢复实时广播

普通 `tracing` / OpenTelemetry 属于可采样的运行时观测，不承担完整业务历史。SSE
实时广播在 Store 持久化成功之后发生：慢客户端、断线或广播队列溢出可以导致客户端
暂时漏过实时事件，但不能影响 Store，也不能反向定义 Core Sink 为 Best Effort。
客户端使用 Store 中按 sequence 保存的事件和 `Last-Event-ID` 补发恢复。

---

# 24. OpenTelemetry 映射

GraphLoom 内部 Explainability Schema 不等于 OpenTelemetry Schema。

推荐映射：

| GraphLoom 操作       | OpenInference / GenAI 语义 |
| ------------------ | ------------------------ |
| 整体 Query           | `CHAIN`                  |
| Query Embedding    | `EMBEDDING`              |
| ANN Search         | `RETRIEVER`              |
| 候选重排               | `RERANKER`               |
| Prompt 构造          | `PROMPT`                 |
| LLM 请求             | `LLM`                    |
| DRIFT 总流程          | `AGENT` 或 `CHAIN`        |
| DRIFT Local Action | 子 `CHAIN`                |

OpenTelemetry 中记录：

* 操作名称；
  -耗时；
  -错误；
  -token；
  -model；
  -provider；
  -cache；
  -candidate count；
  -selected count；
  -context tokens。

完整的：

* Entity 列表；
  -Relationship 列表；
  -Context 内容；
  -Text Unit；
  -selection reason；
  -图扩展路径；

保留在 Explainability Event 中。

---

# 25. 与现有 Callback 的关系

现有接口：

```text
IndexWorkflowCallbacks
QueryCallbacks
```

第一阶段保留。

推荐边界：

```text
现有 Callback
    └── 既有调用方、进度、流式回答和兼容 API

ExplainabilitySink
    └── 结构化业务解释、Studio 和历史回放
```

第一阶段禁止为了统一接口而：

* 删除现有 Callback；
  -大规模重构 Query；
  -改变 Query streaming；
  -改变兼容行为。

当 Explainability Schema 稳定后，再评估：

-哪些 Callback 继续保留；
-哪些可以由 Adapter 实现；
-哪些旧接口可以逐步 deprecated。

---

# 26. Crate 与模块策略

第一阶段不立即新增独立 workspace crate。

建议先在现有 crate 中建立：

```text
crates/graphloom/src/explainability/
├── mod.rs
├── event.rs
├── sink.rs
├── dto.rs
├── content_mode.rs
└── graph_data.rs
```

JSONL Recorder 可以先放在：

```text
crates/graphloom/src/explainability/jsonl.rs
```

Studio 存储实现放在 Studio 自己的模块中：

```text
crates/graphloom-studio/src/explainability/
├── service.rs
├── store.rs
├── sqlite.rs
├── live_hub.rs
└── sse.rs
```

满足以下条件后，再考虑拆 crate：

* Studio 已正式依赖 Explainability DTO；
  -第三方 crate 需要独立使用 DTO；
  -Schema 需要独立版本；
  -OpenTelemetry Adapter 依赖明显增加；
  -Core 编译依赖受影响。

未来可能拆成：

```text
graphloom-explainability
graphloom-explainability-jsonl
graphloom-opentelemetry
graphloom-studio
```

第一阶段不为了形式上的模块化提前增加复杂度。

---

# 27. 兼容性要求

引入 Explainability 后，以下运行方式必须产生相同业务结果：

```text
不开启 Explainability
NoopExplainabilitySink
RecordingExplainabilitySink
JsonlExplainabilitySink
StudioExplainabilitySink
```

必须保持相同的：

* LLM 请求；
  -Embedding 请求；
  -cache key；
  -cache hit/miss；
  -候选顺序；
  -Context；
  -最终回答；
  -usage；
  -存储产物；
  -向量产物；
  -错误语义。

Explainability 插桩不得：

* 重新排序数据；
  -增加 LLM 调用；
  -增加 Embedding 调用；
  -重复执行向量检索；
  -改变并发调度；
  -改变 token budget；
  -改变 fallback；
  -改变 GraphRAG 兼容行为；
  -修改 compatibility fixture；
  -更新现有 golden；
  -放宽现有断言。

---

# 28. 测试策略

## 28.1 单元测试

验证：

* Event 序列化；
  -Envelope 序列化；
  -Schema Version；
  -Content Mode；
  -敏感字段过滤；
  -No-op Sink；
  -Selection Reason；
  -Record Type；
  -Context Section。

---

## 28.2 Local Query Fixture

使用：

* 固定 Completion Model；
  -固定 Embedding Model；
  -固定向量结果；
  -固定 Query 数据。

验证事件顺序：

```text
QueryStarted
MappingQueryBuilt
EmbeddingStarted
EmbeddingCompleted
CandidatesRetrieved
CandidatesFiltered
EntitiesSelected
GraphExpansionStarted
RelationshipsSelected
CommunityReportsSelected
TextUnitsSelected
ContextBudgetAllocated
ContextSectionBuilt
ContextCompleted
LlmRequestStarted
LlmRequestCompleted
RunCompleted
```

动态字段测试时进行规范化：

* `run_id`；
  -`span_id`；
  -`timestamp`；
  -绝对路径。

业务字段和事件顺序必须严格比较。

---

## 28.3 兼容回归

比较：

```text
No Explainability
Noop Sink
Recording Sink
```

确保三者：

* Provider Requests 相同；
  -Context 相同；
  -Response 相同；
  -Usage 相同；
  -Cache 相同；
  -产物相同。

---

## 28.4 JSONL 测试

验证：

* 每行都是合法 JSON；
  -事件顺序正确；
  -sequence 唯一；
  -flush 正常；
  -写入失败可见；
  -中断前事件可读取；
  -不存在重复事件。

---

## 28.5 Store 测试

对所有 `ExplainabilityStore` 实现运行同一套合同测试：

* create run；
  -append events；
  -load after sequence；
  -complete run；
  -list runs；
  -delete run；
  -event order；
  -duplicate sequence rejection；
  -cascade delete。

这样未来添加 Turso 实现时，可以复用相同测试。

---

## 28.6 SSE 测试

验证：

* 新连接可以读取已有事件；
  -运行期间可以收到新增事件；
  -`Last-Event-ID` 可以续传；
  -历史与实时切换不漏事件；
  -历史与实时切换不重复事件；
  -慢客户端不会阻塞 Query；
  -客户端断开不会影响 Run。

---

# 29. Studio 第一阶段范围

第一版 Studio 实现以下功能。

## 29.1 Graph Explorer

* Entity 节点；
  -Relationship 边；
  -Community 基础展示；
  -节点详情；
  -Relationship 详情；
  -Text Unit 来源查看；
  -按 Entity Type 过滤；
  -按 Community 过滤。

## 29.2 Query Chat

* 输入问题；
  -选择 Query Method；
  -流式回答；
  -展示最终 Context；
  -展示 usage。

## 29.3 Local Query Explainability

* 候选 Entity；
  -最终 Entity；
  -Relationship 扩展；
  -Community Report；
  -Text Unit；
  -token budget；
  -最终 Context；
  -LLM 请求；
  -最终回答。

## 29.4 Run History

* 历史 Query 列表；
  -按时间排序；
  -按 Query Method 过滤；
  -查看运行状态；
  -删除 Run；
  -加载历史事件。

## 29.5 Offline Replay

* 从头回放；
  -暂停；
  -继续；
  -单步；
  -调整速度；
  -直接跳转最终状态。

第一版不要求：

* Index 实时动画；
  -Update 实时动画；
  -DRIFT 完整可视化；
  -多用户权限；
  -云端同步；
  -复杂 Trace 搜索；
  -compatible/optimized 对比；
  -DuckDB Analytics；
  -Turso 后端；
  -WebSocket。

---

# 30. 实施顺序

## Phase 1：设计与基础类型

1. 建立 `explainability` 模块；
   2.定义 `ExplainabilityEvent`；
   3.定义 DTO；
   4.定义 `ExplainabilityContentMode`；
   5.定义经验证的 Run ID 和 Span ID；
   6.定义 `ExplainabilityRecord` 和 `ExplainabilityEnvelope`；
   7.定义 `ExplainabilitySink`；
   8.实现 `NoopExplainabilitySink` 和 `ExplainabilitySinkChain`；
   9.增加序列化和公共 API 测试。

## Phase 2：Local Query 插桩

1. Query 开始和完成；
   2.Mapping Query；
   3.Embedding；
   4.ANN 候选；
   5.候选过滤；
   6.Entity 选择；
   7.Relationship 扩展；
   8.Community Report；
   9.Text Unit；
   10.Context Budget；
   11.最终 Context；
   12.LLM 请求。

## Phase 3：CLI JSONL

已实现：

1. JSONL Sink；
   2.bounded channel；
   3.writer task；
   4.flush；
   5.CLI 参数；
   6.错误处理；
   7.fixture 测试。

## Phase 4：tracing 规范化

1. Query Span；
   2.Embedding Span；
   3.Retriever Span；
   4.Graph Expansion Span；
   5.Context Span；
   6.LLM Span；
   7.字段统一；
   8.日志回归。

## Phase 5：OpenTelemetry

1. CLI OTLP 配置；
   2.标准 Resource；
   3.OpenInference/GenAI 映射；
   4.shutdown；
   5.flush；
   6.Collector 集成测试。

## Phase 6：GraphDataSource

1. Entity View；
   2.Relationship View；
   3.Community View；
   4.Community Report View；
   5.Text Unit View；
   6.分页；
   7.过滤；
   8.基于现有 Provider 实现。

## Phase 7：Studio Store 与 SSE

1. `ExplainabilityStore`；
   2.SQLite 实现；
   3.Run Service；
   4.Event writer task；
   5.Live Hub；
   6.SSE；
   7.历史补发；
   8.断线续传。

## Phase 8：Studio 前端

1. Graph Explorer；
   2.Query Chat；
   3.Live Explainability；
   4.Run History；
   5.Offline Replay。

## Phase 9：后续扩展

1. Basic Query；
   2.Global Query；
   3.Dynamic Global；
   4.DRIFT；
   5.Index Timeline；
   6.Update Timeline；
   7.Turso Store；
   8.DuckDB Analytics；
   9.compatible/optimized 对比。

---

# 31. 当前阶段明确不处理的内容

当前阶段不修改：

* O-01～O-21 中为 GraphRAG 兼容保留的行为；
  -实体身份策略；
  -Update 原子发布；
  -删除传播；
  -Claim 进入 Community Context；
  -Claim gleaning 行为；
  -Prompt Tune 行为优化；
  -Community bottom-up 生成；
  -PostgreSQL；
  -pgvector；
  -S3；
  -optimized/native mode。

这些问题应在 Explainability 和 Studio 能清晰展示其实际影响后，再基于运行证据决定优先级。

---

# 32. 验收标准

第一阶段完成后，应满足：

* GraphLoom Lib 提供稳定的 `ExplainabilitySink`；
  -不开启 Explainability 时无业务行为变化；
  -Local Query 能产生完整事件序列；
  -CLI 能输出 Explainability JSONL；
  -CLI 默认日志正常；
  -日志和 OpenTelemetry 共用 `tracing` 插桩；
  -CLI 可以可选导出 OpenTelemetry；
  -完整图谱记录不会默认上传到 OTLP；
  -GraphDataSource 可以提供 Studio 所需图谱数据；
  -Studio 默认使用 SQLite；
  -Studio 实时推送使用 SSE；
  -实时事件先持久化再广播；
  -历史回放与实时展示共用相同事件；
  -SSE 支持 `Last-Event-ID`；
  -前端实时和离线使用同一个 Reducer；
  -存储实现通过 `ExplainabilityStore` 抽象；
  -架构允许未来增加 Turso；
  -DuckDB 被定位为分析后端；
  -现有 GraphRAG compatibility gate 全部通过。

---

# 33. 最终架构结论

```text
                         GraphLoom Lib
                              │
        ┌─────────────────────┼─────────────────────┐
        │                     │                     │
     tracing          ExplainabilityRecord    GraphDataSource
        │                     │                     │
   ┌────┴────┐        ┌───────┴────────┐            │
   │         │        │                │            │
Logging     OTLP     CLI             Studio     Graph Explorer
                      │                │
                    JSONL      ExplainabilityService
                                       │
                         ┌─────────────┴─────────────┐
                         │                           │
               ExplainabilityStore             Live Hub
                         │                           │
                  SQLite 默认实现                    SSE
                         │                           │
                  Offline Replay              Live Explainability
                         │                           │
                         └─────────────┬─────────────┘
                                       │
                                Same UI Reducer
```

最终决策如下：

1. 统一使用名称 **Explainability**。
2. 日志和 OpenTelemetry 共用 `tracing` 插桩。
3. Explainability 与普通日志分离。
4. GraphLoom Lib 提供 Explainability Event 和图谱数据能力。
5. CLI 提供日志、OTLP 和 JSONL Adapter。
6. Studio 第一版使用 HTTP + SSE。
7. 实时展示和离线回放共用同一持久化事件流。
8. Studio 使用 `ExplainabilityStore` 抽象。
9. 第一版默认存储使用 SQLite。
10. 后续可增加 Turso 作为远程同步实现。
11. DuckDB 用于跨 Run 分析和 Benchmark，不作为默认实时 Event Store。
12. 第一阶段只完整实现 Local Query Explainability。
