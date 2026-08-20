# GraphLoom Explainability 与 Studio 架构设计

## 1. 文档状态

* 状态：已实现的架构合同（持续演进）
* 适用项目：GraphLoom
* 兼容基线：`graphrag-3.1.0-compat-v1`
* 目标阶段：GraphLoom Explainability 与 Studio 实现
* 最后更新：2026-08-09

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

GraphLoom Studio Frontend MVP 已实现，当前界面建立在已冻结的 Query、Result、Run、SSE 与
Graph Explorer HTTP 合同之上。

Studio V3 Phase 1 使用 Graph-first Explainable QA Workspace：

```text
┌──────────────────┬────────────────────────────┬──────────────────┐
│ Query Composer   │                            │ Graph Inspector  │
│ Answer           │ Main Knowledge Graph Canvas│ Entity           │
│ Execution Trace │                            │ Relationship     │
│ Run History      │                            │ Community        │
└──────────────────┴────────────────────────────┴──────────────────┘
```

左右面板可调整宽度或折叠；折叠只改变 presentation，不中断 Query/SSE、不清空 Run 或
Inspector selection，也不会重新读取 Graph projection。中央画布保持单一 Graph Explorer
state owner，并占用桌面布局的主要空间。

Phase 1 主画布仍只呈现现有 API 提供的 query-visible Entity/Relationship projection。
Document、Text Unit、Community 与 Report 不会在本阶段伪装成 Cytoscape graph node。

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
  -只允许 `http://` 与 `https://`（大小写不敏感）；拒绝空字符串、纯空白、
  query string、fragment，以及已经以 `/v1/traces` 结尾的值；
  -endpoint 校验发生在参数校验阶段（project config 加载、OTLP Runtime 创建、
  网络请求与 Query 执行之前），错误消息为固定低基数文本，不回显用户输入；
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
→ prepare query log directory（在 OTLP Runtime 创建前）
→ 构建可选 OTLP Runtime（exporter/provider/tracer）
→ 初始化 file/console/可选 OTLP subscriber
→ 创建可选 Explainability Recorder
→ 执行 Query
→ shutdown Explainability Recorder
→ force-flush + shutdown OTLP provider（spawn_blocking）
→ 合并 Query/Recorder/Telemetry outcome
→ drop query.log WorkerGuard
```

* Query Core Span 在 Query 返回前关闭；OTLP flush 发生在全部 Query Span close 后；
  -Query log directory 在 OTLP Runtime 创建前准备；目录创建失败时
  Runtime builder / SpanExporter / Batch worker 均未创建，直接返回
  `create Query log directory` I/O error，不产生 telemetry shutdown Event；
  -OTLP Runtime 创建之后到 `set_global_default` 之间没有其他可失败步骤；
  subscriber install 失败时在 `spawn_blocking` 上显式
  `force_flush()` + `shutdown_with_timeout()`，不遗留 batch worker；
  -初始化清理失败不覆盖初始化主错误；该路径尽最大努力关闭 Provider，
  且不会产生 `graphloom.cli.telemetry.shutdown_failed`（subscriber 尚未安装）；
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
    async fn load_snapshot(
        &self,
    ) -> Result<GraphDataSnapshot, GraphDataSourceError>;
}
```

`GraphDataSnapshot` 承载 Studio-owned typed entity/relationship/community/report records；它们
有公开、安全的构造入口，使 crate 外的 datasource backend 能真正实现该 trait。第一版
`ParquetGraphDataSource` 基于现有 `TableProvider` 实现，并只把 Query-side adapter 输出单向映射
成这些 records。HTTP 只依赖 `Arc<dyn GraphDataSource>`，后续 `PostgresGraphDataSource` 可以
保持相同 wire API。

V1 明确定义为 **Query-visible final graph**，不是 raw Parquet debugger，也不展示中间 index
表。数据流为：

```text
┌───────────────────────────────┐
│ GraphLoom Index Output        │
│ entities / relationships /   │
│ communities / reports        │
└───────────────┬───────────────┘
                ▼
┌───────────────────────────────┐
│ ParquetTableProvider          │
└───────────────┬───────────────┘
                ▼
┌───────────────────────────────┐
│ GraphRAG-compatible Query     │
│ read_indexer_* adapters       │
└───────────────┬───────────────┘
                ▼
┌───────────────────────────────┐
│ GraphDataSnapshot             │
└───────────────┬───────────────┘
                ▼
┌───────────────────────────────┐
│ Studio Graph DTO → HTTP       │
└───────────────────────────────┘
```

Studio 不自行解析 Polars `AnyValue`、nullable/list 兼容列、human-readable id 或 community
roll-up。这样 Query 与 Explorer 对同一份 output 使用相同兼容语义，避免 “Query sees A,
Studio shows B”。

---

## 15.2 Query-visible community/report 语义

`read_indexer_communities` 只返回有 report 的 Query-visible community；因此 community HTTP
API 表示 report-backed/query-visible communities，不保证覆盖 `communities.parquet` 的每一条
raw row。

Entity membership 使用：

```text
read_indexer_entities(community_level=i64::MAX, method=Local)
```

避免把普通 Query 的 level=2 默认值误当 Explorer 的展示边界。Report snapshot 使用：

```text
read_indexer_reports(community_level=i64::MAX, dynamic=true, method=Local)
```

这里的 dynamic=true 只为复用 schema-compatible adapter 并绕开 non-dynamic 的 title roll-up；
两个不同 community 即使 report title 相同也都保留。HTTP 不输出
`full_content_embedding`；relationship `source`/`target` 继续表示 entity title，不伪造 entity
stable ID。

---

## 15.3 Graph Explorer HTTP 合同

已实现 routes：

```http
GET /api/graph/summary
GET /api/graph/overview
POST /api/graph/subgraph
GET /api/graph/entities
GET /api/graph/entities/{entity_id}
GET /api/graph/relationships
GET /api/graph/relationships/{relationship_id}
GET /api/graph/communities
GET /api/graph/communities/{community_id}
GET /api/graph/communities/{community_id}/report
```

三个 list endpoint 统一使用 `limit`（默认 50、最大 200）与 lexical `after` cursor；服务端按
stable id ASC 排序、过滤 `id > after` 后取一页。Entity 支持 exact `type`/`community`，
Relationship 支持 exact `source`/`target`，Community 支持 exact `level`/`parent`。list DTO
省略长 description/text-unit 内容；完整 report 只由 report detail endpoint 返回，任何 Graph
HTTP JSON 都不包含 embedding/vector。

每次 request 都重新读取 current output，不缓存 snapshot，因此 Index/Update 发布后新 request
自然看到新数据。Snapshot 验证 Entity/Relationship/Community/Report stable id、community
short id 和 report community id 的唯一性；不一致时返回固定低信息的 503，不静默 dedup。

HTTP pagination 只限制 response payload。当前 `TableProvider::read_dataframe` 合同仍会读取四张
完整 Parquet 表，本版本没有 backend-side row pushdown，不能把磁盘扫描复杂度声称为
`O(page_size)`。未来可以用 Postgres、DuckDB/DataFusion、Arrow pushdown 或 indexed graph
backend 实现同一 `GraphDataSource`，无需修改 frontend API。

非法 query/path 返回固定 400，item 不存在返回固定 404，output 尚未生成、缺表、schema 损坏
或 snapshot invariant 失败返回固定 503；response 不包含 filesystem path、table/Polars error 或
row content。

### 15.3.1 Graph Overview V2 与 Query-linked Subgraph

旧 Studio MVP 分别读取 bounded Entity page 与 Relationship page，再由浏览器按 title 解析边：

```text
first N entities + first M relationships
                ↓
       frontend title resolution
```

这两个独立 page 不保证形成同一个 topology。完整图中真实存在的 endpoint 只要不在 Entity page，
对应 Relationship 就会被误报为 unresolved。V2 把解析顺序改为：

```text
┌──────────────────────── Full GraphDataSnapshot ────────────────────────┐
│ entities_by_id / entities_by_title / relationships_by_id / adjacency  │
└────────────────────────────────┬───────────────────────────────────────┘
                                 ▼
                 endpoint title 必须唯一匹配 Entity
                                 ▼
                     deterministic resolved topology
                                 ▼
           edge-first Overview 或 seed-preserving 1-hop Subgraph
                                 ▼
                 bounded HTTP response → bounded Cytoscape graph
```

`GET /api/graph/overview` 默认最多 80 个 Entity、160 个 Relationship，硬上限分别为
200/400。resolved Relationship 按 `rank DESC`、`weight DESC`、relationship stable ID ASC
排序；只有加入所需 endpoint 后仍不超过 Entity limit 的边才进入 projection。只要存在
resolved topology，就不会用无关孤立节点填满 Entity limit；仅当 resolved Relationship 为零时，
才按 Entity `rank DESC`、ID ASC 返回 fallback 节点。

`POST /api/graph/subgraph` 是无状态的 depth 0/1 projection。Entity seed 若存在必须保留；
resolved Relationship seed 的两个 endpoint 自动成为 Entity seed。不存在的 seed 分别进入
`missing_entity_ids`/`missing_relationship_ids`，存在但 endpoint title 缺失或歧义的 Relationship
seed 进入 `unresolved_relationship_ids`。有效 seed 自身超过请求 limit 时固定返回 400，不能静默
丢 seed；depth 1 只从初始 seed Entity 扩展真实相邻边，并以同一确定性 priority 在 limits 内选择。

`unresolved_relationship_count` 统计完整 `GraphDataSnapshot` 中 endpoint title 缺失或不能唯一匹配
而真正无法稳定解析的 Relationship；它不再表示 frontend page sampling 缺少 endpoint。
Projection 是 Studio visualization concern，不参与 GraphRAG Query、Context、retrieval 或 ranking，
Overview 的显示顺序也不会改变任何 GraphRAG observable semantics。HTTP response 和浏览器图是
bounded 的，但 Parquet datasource 仍执行 whole-table snapshot read，本阶段没有解决磁盘 scan pushdown。

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

Core runtime 当前完整接入 Local Query 与 static Global Query Explainability。Dynamic Global、
Basic 和 DRIFT 即使收到请求配置也保持安全 no-op，不会创建缺失关键 evidence 的半截 Run。
Local tracing topology 保持不变；static Global 仅接入 Explainability，不扩展 OpenTelemetry。
JSONL Recorder、Store、SQLite、bounded persistence writer、每 Run sequence allocator、Live Hub、
host-side Explainability SSE、Studio Local Query API、Query Result、Run metadata、Run history API、
Query-visible Graph Explorer API 与浏览器 Frontend MVP 已实现；Turso、DuckDB 和 Global Studio
Semantic Timeline 已实现；Turso、DuckDB 和 Dynamic Global UI 仍属于后续阶段。Studio Query composer 当前仍只开放 Local，因此对 Basic、
Global 与 DRIFT 返回 422；这不限制 Core/CLI 对 static Global Explainability 的支持。

Static Global 的 request-scoped topology 与真实 fan-out/fan-in 如下；每个 batch span 都有独立
ID，`batch_index` 是语义身份，持久化 sequence 只表示实际 emission 顺序：

```text
Query root
├── Global context
│   └── GlobalContextBuilt
├── Map
│   ├── batch 0: BatchBuilt → LLM Started → LLM Completed → PointsProduced
│   ├── batch 1: BatchBuilt → LLM Started → LLM Completed → PointsProduced
│   └── ...（并行，完成顺序不保证等于 batch_index）
└── Reduce
    ├── ReduceContextBuilt（含每个 point 的真实 selection decision）
    ├── ReduceSkipped(NoPositivePoints)，或
    └── LLM Started → streamed completion → LLM Completed
```

CommunityReport stable ID 在真实 batch-local sort 后、CSV render 前后同一构造路径中作为
sidecar 捕获，不从 CSV 或 short ID 反推。Reduce decision 也在真实 `score > 0`、stable score
sort 与 first-over-budget `break` 循环中捕获；Explainability 不重新执行 selection。

Studio 使用 `QueryStarted.method` 分派 method-specific semantic presentation。Local builder 保持原
四步模型；static Global builder 单次按 sequence 扫描，并使用 `batch_index`、`span_id` 和
`parent_span_id` 聚合并行 batch。React 只消费纯函数产生的 view model，不扫描 raw envelopes：

```text
Explainability envelopes
          │
          ▼
 QueryStarted.method dispatcher
          │
    ┌─────┴─────┐
    ▼           ▼
 Local builder  Static Global builder
    │           │
    │           ├─ Community Context
    │           ├─ Map Analysis (batch_index stable order)
    │           ├─ Evidence Reduction
    │           └─ Answer Generation
    ▼
 Entity Mapping → Graph Expansion → Context Assembly → Answer Generation
```

Global exact Map/Reduce context、prompt 和 raw response 直接使用 G1 捕获字段；Preview 只是安全
presentation，前端不重建 exact input、不重新执行 Reduce selection，也不把 report/point 猜成
Graph focus。

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

底层实时 fan-out 已由 `graphloom::explainability::ExplainabilityLiveHub` 实现，host-side
Explainability SSE 由独立的 `graphloom-studio` crate 实现。依赖方向固定为
`graphloom-studio → graphloom`；GraphLoom Lib 不依赖 Axum、SSE 或浏览器协议。Live Hub
只接收成功持久化的 `ExplainabilityEnvelope`，不持有 Store、不分配 sequence，也不提供
HTTP 能力。Studio Local Query、Query Result、Run metadata/history、SSE、Graph Explorer read API
与 React Frontend MVP 已实现。

## 17.1 第一版使用 SSE

GraphLoom Studio 第一版采用：

```text
HTTP + Server-Sent Events
```

而不是 WebSocket。

当前已实现的交互模式是：

```text
既有宿主流程 ──创建并执行 Run──→ GraphLoom
浏览器      ←──SSE Events────── graphloom-studio
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

## 17.2 HTTP API 状态

当前已实现的 routes 是：

```http
POST /api/query
GET  /api/query/{run_id}/result
GET  /api/explainability/runs
GET  /api/explainability/runs/{run_id}
GET  /api/explainability/runs/{run_id}/events
GET  /api/graph/summary
GET  /api/graph/entities
GET  /api/graph/entities/{entity_id}
GET  /api/graph/relationships
GET  /api/graph/relationships/{relationship_id}
GET  /api/graph/communities
GET  /api/graph/communities/{community_id}
GET  /api/graph/communities/{community_id}/report
```

`POST /api/query` 只接受 Local，先进行有界 admission，再由服务端生成 run ID；每个 Query
拥有独立 Recorder，并在 `create_run` 的 Store ACK 与 LiveHub 注册均完成后才返回 202。
GET metadata/history 只读 Store；SSE 继续组合 Store replay 与 LiveHub，不绑定 TCP 端口。

Query 生命周期固定为：

```text
POST /api/query
→ try_acquire Query permit
→ server run_id
→ per-query StoreExplainabilityRecorder
→ create_run（Store + LiveHub ready）
→ HTTP 202
→ GraphLoom Local Query
→ Core terminal Event + finish_run
→ QueryResult conversion
→ successful Result registry insertion
→ Studio complete_run(Completed/Failed)
→ Recorder shutdown
```

Core 拥有 `RunStarted`/`RunCompleted`/`RunFailed` 与 `finish_run`；Studio 只拥有 Run metadata
创建、基于 Query 返回结果的 Completed/Failed transition 与 HTTP 编排。Studio 不重复 finish，
也不直接调用 Store 绕过 Recorder。202 只表示 Run 已创建并接受执行，不保证 Query 成功。
HTTP/SSE client disconnect 不取消 Query；本阶段没有 cancellation API 或 task registry。

### 17.2.1 Query Result 与 Explainability 的边界

`GET /api/query/{run_id}/result` 是最终业务答案的正式 API。它返回 `run_id`、`response`、
`elapsed_ms` 和 provider usage（总量及按 category 的稳定映射），不会暴露 `QueryContext`、
DataFrame、候选 records、prompt 或 raw provider body。`POST /api/query` 的 202 JSON 同时返回
`result_url`；`Location` 仍指向 Run metadata。

```text
Query Result
    = 用户请求的最终业务答案

Explainability
    = 为什么以及如何得到答案的持久化执行轨迹
```

`ExplainabilityContentMode` 只决定 Explainability 轨迹可以持久化多少内容，不控制业务结果
的可见性。因此 `metadata` 模式仍可从 Result API 得到完整 answer，同时 Run.query 与
`QueryStarted.query` 保持省略。成功路径先发布内存结果，再写入 Completed metadata：

```text
GraphLoom Query Ok(QueryResult)
→ checked result conversion（usize/Duration → u64）
→ bounded Result registry insert
→ recorder.complete_run(Completed)
→ recorder.shutdown
```

这保证一旦 Store 对外可见 `Completed`，同一进程中的结果已先准备好。若 completion 失败，
registry entry 会被移除，Run 保留 truthful `Running` prefix；若结果转换失败，同样不伪造
terminal metadata。Query 本身失败时不存成功结果，并由 Studio 写入 `Failed` metadata。

V1 Result registry 属于 Studio 当前进程，默认最多保留 128 个成功结果，按成功插入顺序 FIFO
淘汰；它没有无界队列，也不写入 `ExplainabilityStore`。进程重启或淘汰后，Explainability Run
与 Event 仍可由 Store 持久化存在，但 `Completed` Run 的 Result GET 返回 `410 Gone`。未来如需
跨重启保留 answer，应新增独立的 Studio Query Result persistence，而不是扩展
`ExplainabilityStore`。

Result GET 先读取 Store lifecycle，再读取 registry：Running/Pending 返回 202，Failed/Cancelled
返回 409，Completed 且 retained 返回 200，Completed 但缺失返回 410；非法 ID 返回 400，未知
或非 Query Run 返回 404，Store failure 返回固定安全的 500。

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
event: explainability
data: {"schema_version":1,"sequence":12,"record":{"run_id":"01J...",...}}
```

浏览器断线重连时可以发送：

```http
Last-Event-ID: 12
```

`id` 严格等于 decimal `ExplainabilityEnvelope.sequence`，`data` 是完整 Envelope 的 compact
JSON；业务事件类型仍从 `data.record.event.type` 读取，不建立第二套 SSE event-name schema。
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
      ↓ reliable: await ExplainabilitySink::emit
StoreExplainabilityRecorder
      ↓ reliable bounded queue / producer backpressure
single writer（已实现）
      ↓ 分配 sequence，构造 ExplainabilityEnvelope
ExplainabilityStore.append_events
      ↓ durable success
writer committed sequence update
      ↓ best effort, bounded, non-blocking
ExplainabilityLiveHub（已实现，one channel / active Run）
      ↓
graphloom-studio Explainability SSE stream（已实现）
```

核心原则：

> 实时模式是边持久化边消费，离线模式是从存储重新消费同一事件。

事件 Schema、顺序和语义必须完全相同。

`GraphLoom Core → ExplainabilitySink → 持久化单写者 → Store` 是可靠、可背压、错误
可见的边界。`Store success → Live Hub → SSE 客户端` 位于持久化之后，可以发生暂时的实时
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

第 1～6 步已由 `StoreExplainabilityRecorder` 与 `ExplainabilityLiveHub` 实现：
`Store.append_events` 成功后，writer 先提交自己的 sequence 状态，再把同一次构造的
Envelope 包装为 `Arc` 交给 Hub。默认的 `StoreExplainabilityRecorder::new` 不创建 Hub、
channel 或每事件 `Arc`；只有 `new_with_live_hub` 启用该路径。Hub 的 `publish` 同步、
非阻塞且不返回 persistence error；无订阅者、订阅者断开或 lag 均不能使 writer 失败。

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
1. subscription = hub.subscribe(run_id)
2. 如果为 None，仅从 Store replay
3. 如果为 Some，先注册 live receiver，再读取 snapshot_sequence = S
4. 从 last_seen 开始分页读取 Store，至少追到 S
5. 再消费 live receiver，sequence <= last_seen 的重复项跳过
6. 收到 Lagged 时重新 subscribe，读取新的 snapshot，再从 Store 恢复
7. 收到 Closed 时做最后一次 Store catch-up，然后结束 stream
```

客户端只接受：

```text
sequence > last_seen_sequence
```

避免重复。

receiver 注册必须先于 `snapshot_sequence` 读取。若并发 publish 在快照之后发生，receiver
会收到它；若发生在 receiver 注册与快照读取之间，Store catch-up 与 live 可能都包含它，
由 sequence 去重；若 publish 时没有 subscriber，快照仍前进，Store catch-up 会补齐。
`snapshot_sequence` 表示 Recorder 已成功 Store commit、并更新 writer committed sequence
之后提交给 Hub 的最新 sequence；它是 Store catch-up boundary，不是任一 subscriber 的
delivery ACK。Store 的 `after_sequence + limit` 已足够：分页若读到 `> S` 的事件，仍按
sequence 去重，无需为 Store V1 增加 upper bound。

已实现状态机的完整恢复分支为：

```text
HTTP Last-Event-ID = L（否则 query after_sequence，否则 0）
↓
Store.get_run preflight
↓
LiveHub.subscribe
├── None
│   └── Store replay after L → exhaustion → EOF
└── Some(subscription)
    ↓ receiver 已注册；读取 snapshot S
    ↓ Store replay after L，至少追到 S
    ↓ Live recv
       ├── sequence <= last_seen → overlap dedup
       ├── sequence == last_seen + 1 → SSE send
       ├── sequence gap → resubscribe + NEW snapshot + Store recovery
       ├── Lagged → resubscribe + NEW snapshot + Store recovery
       └── Closed → final Store catch-up → EOF
```

Store replay 每连接每次只读取 64 个 Envelope；一页 drain 完才读取下一页，不缓存完整历史，
也不建立 per-client relay task 或无界 channel。Store page 必须与请求 Run 一致并从
`last_seen + 1` 严格连续；wrong-run、duplicate、gap 或 out-of-order 都终止连接，不能排序、
跳过或重新编号。若 snapshot target 在 Store 中不可达，同样作为内部合同损坏终止连接。

`Last-Event-ID` 只接受未 trim 的 ASCII decimal `u64`。Header 优先于 query cursor；cursor
大于 preflight `event_count` 返回 409。不存在的 Run 返回 404，非法请求返回 400，preflight
Store failure 返回 500，响应均为固定低信息文本。SSE headers 发出后的 Store/序列化错误只能
安全终止 stream，客户端随后按最后收到的 SSE ID 重连；不会构造新的 `event: error` schema。
SSE 使用 15 秒空 comment keepalive，keepalive 不进入 Store 或 sequence。

`Closed → final catch-up → EOF` 只表示当前 server 已无更多实时/持久化事件可发送，绝不表示
`Run.status == Completed`。`RunCompleted` / `RunFailed` Envelope 与其他 Envelope 一样只是数据，
不会驱动 SSE 或 Hub 生命周期。客户端断开只 drop stream/subscription，不 cancel、finish 或
complete Run；服务端不保存 per-client cursor registry 或 durable subscriber ACK。

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

已实现公开业务接口 `ExplainabilityStore`（`crates/graphloom/src/explainability/store.rs`）：

```rust
#[async_trait::async_trait]
pub trait ExplainabilityStore: Send + Sync + std::fmt::Debug {
    async fn create_run(
        &self,
        run: ExplainabilityRun,
    ) -> Result<(), ExplainabilityStoreError>;

    async fn append_events(
        &self,
        events: &[ExplainabilityEnvelope],
    ) -> Result<(), ExplainabilityStoreError>;

    async fn complete_run(
        &self,
        completion: RunCompletion,
    ) -> Result<(), ExplainabilityStoreError>;

    async fn get_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<Option<ExplainabilityRun>>;

    async fn list_runs(
        &self,
        query: &RunQuery,
    ) -> Result<Vec<ExplainabilityRun>>;

    async fn load_events(
        &self,
        run_id: &ExplainabilityRunId,
        query: &EventQuery,
    ) -> Result<Vec<ExplainabilityEnvelope>>;

    async fn delete_run(
        &self,
        run_id: &ExplainabilityRunId,
    ) -> Result<()>;
}
```

接口只暴露业务语义，不暴露数据库连接。

不推荐：

```rust
fn sqlite_connection(&self) -> &Connection;
```

## 19.2 业务不变量

Store 合同固定以下 Version 1 不变量，未来 SQLite 实现必须逐条满足：

* `create_run` 只接受 `event_count == 0`、`completed_at == None`、
  `Pending`/`Running` 初始状态；相同 run ID 返回 `RunAlreadyExists`；
  非 Query Run 不得携带 `query_method`；
  -`append_events` 整批 all-or-nothing：空 batch 为 no-op；同一 batch 只能属于
  一个 Run；Run 必须已存在；terminal Run 拒绝追加；sequence 必须从
  `event_count + 1` 起严格连续，`u64` 溢出返回明确错误；
  -`ExplainabilityRun.event_count` 只由 Store 维护，始终等于已成功持久化的
  Envelope 数量；失败的 batch 不改变 event_count，也不留下部分事件；
  -`complete_run` 只接受终态（Completed/Failed/Cancelled），要求
  `completed_at >= started_at`；完全相同的终态重试幂等返回 Ok，任何不同的
  终态返回 `CompletionConflict`，不得覆盖原终态；
  -`delete_run` 原子删除 Run 及其全部 Event，幂等（不存在也返回 Ok）；
  -Run 历史固定 `started_at DESC, run_id DESC`，使用 `(started_at, run_id)`
  cursor 分页，不用 offset；Event 回放固定 `sequence ASC` 且
  `sequence > after_sequence`，两个方向都拒绝无界读取；
  -Store 不根据 `RunStarted`/`RunCompleted`/`RunFailed` Event 推导 Run status，
  生命周期由宿主服务通过 `create_run`/`complete_run` 显式控制；
  -Store 不分配、不修改、不重排 Envelope 的 sequence；sequence 由写入方
  Adapter 分配，Store 只验证并持久化。

公开 DTO：

```text
RunCompletion      终态 completion（constructor 拒绝非终态）
RunQuery           过滤 + cursor + limit（默认 50，最大 200）
RunListCursor      (started_at, run_id) 稳定分页位置
EventQuery         after_sequence + limit（默认 500，最大 1000）
ExplainabilityStoreError
```

## 19.3 实现状态

```text
ExplainabilityStore V1
    → frozen

InMemoryExplainabilityStore
    → reference / test backend（已实现）

StoreExplainabilityRecorder
    → bounded queue + single writer persistence adapter（已实现）

ExplainabilityLiveHub
    → per-run bounded post-persistence realtime fan-out（已实现）

SqliteExplainabilityStore
    → persistent backend（已实现）

graphloom-studio host-side service library
    → Explainability SSE + Local Query + Query Result + Run metadata/history + Graph Explorer implemented

Studio React Frontend MVP
    → Query + Result + Explainability Timeline + Run History + Graph Explorer implemented

Studio Basic/Global/DRIFT Explainability Query
    → not implemented
```

Store 写入架构：

```text
GraphLoom Core
    ↓ ExplainabilityRecord

StoreExplainabilityRecorder（已实现）
    ↓ allocates sequence / envelope

ExplainabilityStore
    ├── InMemory
    └── SQLite

Store append success
    ↓ writer sequence commit
ExplainabilityLiveHub（已实现）
    └── one bounded broadcast channel / active Run
```

当前 `SqliteExplainabilityStore` 是底层持久化 Store：

* 不分配 sequence；
* 不解释 Event 内容；
* 不根据 `RunStarted`/`RunCompleted`/`RunFailed` Event 推导 Run status；
* 不自动修改 Run status；
* 完整遵守已冻结的 Store V1 合同。

`StoreExplainabilityRecorder` 是中间持久化层：

* `new(...)` 是可失败的构造函数：必须从活动 Tokio runtime 内调用；
  没有 runtime 时返回 `RuntimeUnavailable`，不 spawn writer task、
  不创建后台资源、不调用 Store、不 panic；
* `create_run`：显式创建 Run metadata（不从 Event 推导）；
* `emit`：有界队列可靠接受 Record（await capacity，不丢 Record）；
* `finish_run`：只作为 persistence barrier，不改变 Run status、
  不生成 RunCompleted；
* `complete_run`：显式把 Store Run 置为 terminal metadata；
* `shutdown`：drain 已接受工作并结束 writer，不隐式 finish/complete。

控制命令（create/complete/finish）一旦成功进入 writer queue，即使调用方
Future 随后被取消，writer 仍可能继续执行该 operation；本阶段不实现撤销
协议。`create_run` 调用方只有收到 `Ok` 后才把 Run 视为可用。

V1 只管理全新 Run：不支持 attach existing Run / resume sequence / 进程重启
续写。crash/shutdown 后已持久化 prefix 保留，Run 可以保持 Running/Pending，
但 writer 不会自动续写同一 Run ID。

## 19.4 InMemory 参考实现

`InMemoryExplainabilityStore` 是 Version 1 reference/development backend：

* 内部为 `RwLock<MemoryState>`（runs + events 两个 HashMap）；
* 所有写事务（create/append/complete/delete）在同一个 write lock 内验证并
  提交，一批 append 不会中途释放锁；
* 读取 clone owned DTO 后释放锁，不把锁 guard 跨 `.await`；
* 不 spawn、不 blocking、不依赖全局状态，可被多个 async task 共享；
* 并发同一 Run append 由锁串行化，最终一个成功一个 `SequenceConflict`，
  不存在 lost update；append 与 complete 竞争只允许两种线性化结果。

## 19.5 SQLite 持久化实现

`SqliteExplainabilityStore`（`crates/graphloom/src/explainability/sqlite.rs`，
可选 feature `sqlite-store`）：

* 真正的文件持久化，使用 `rusqlite` bundled（0.40.1，内置 SQLite 3.53.2）；
* 单一 `Connection`，由 `std::sync::Mutex` 保护同一实例内的访问；所有
  SQLite I/O 运行在 `tokio::task::spawn_blocking`，不阻塞 Tokio worker；
* 同一实例内的操作由 `operation_gate` 串行化，但业务语义不依赖它；
  跨 Store instance / 跨进程的写入原子性由 SQLite transaction 保证；
* 所有写路径使用 `BEGIN IMMEDIATE`；delete 依赖
  `FOREIGN KEY ... ON DELETE CASCADE`；
* 错误与 `Debug` 不泄露 DB path、SQL 参数、Query、Event payload 或 Secret；
* 未来 Studio 通过 `graphloom = { features = ["sqlite-store"] }` 使用该 Store。

---

# 20. 存储实现选择

```text
InMemoryExplainabilityStore
    → reference / tests / embedded development（已实现）

SqliteExplainabilityStore
    → persistent backend（已实现，optional feature `sqlite-store`）

ExplainabilityLiveHub
    → 已实现

Studio Explainability SSE / Local Query / Query Result / Run metadata / Run history / Graph Explorer HTTP API
    → 已实现

Studio Frontend MVP
    → 已实现

Auth / Query cancellation
    → 未实现
```

本阶段交付业务合同、内存参考实现与 SQLite 持久化实现。

## 20.1 SQLite

Studio 默认使用 SQLite。

适用场景：

* 单机 Studio；
* 一个 writer task；
* 实时小批量事件写入；
* 按 `run_id + sequence` 查询；
* 历史记录列表；
* 删除 Run；
* 本地零配置。

已实现：

```text
SqliteExplainabilityStore
```

第一版使用 `rusqlite + tokio::task::spawn_blocking`，不使用 ORM、连接池或
多数据库抽象。SQLite 仅通过可选 feature `sqlite-store` 进入依赖图；普通
GraphLoom binary 和 Library 用户不编译 rusqlite/libsqlite3-sys，也不产生
数据库文件。

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

## 21.1 两种版本号

```text
EXPLAINABILITY_SCHEMA_VERSION = 1
    → Envelope / Event transport schema

SQLITE_STORE_SCHEMA_VERSION = 1
    → SQLite physical database schema
```

二者含义不同，未来可以独立升级：

* `EXPLAINABILITY_SCHEMA_VERSION` 由
  `crates/graphloom/src/explainability/record.rs` 定义，版本化
  `ExplainabilityEnvelope` 的 JSON transport 形态；
* `SQLITE_STORE_SCHEMA_VERSION` 由
  `crates/graphloom/src/explainability/sqlite.rs` 定义，版本化 SQLite
  物理表结构，保存在独立的 `explainability_store_meta` 表中，不使用
  `PRAGMA user_version`（未来同一 DB 文件可能还有其他模块的表）。

## 21.2 表结构

第一版创建三张表和一张索引：

```sql
CREATE TABLE explainability_store_meta (
    singleton INTEGER PRIMARY KEY
        CHECK (singleton = 1),

    schema_version INTEGER NOT NULL
);

CREATE TABLE explainability_runs (
    run_id TEXT PRIMARY KEY COLLATE BINARY,

    kind TEXT NOT NULL,
    status TEXT NOT NULL,

    query TEXT,
    query_method TEXT,

    started_at TEXT NOT NULL COLLATE BINARY,
    completed_at TEXT,

    compatibility_profile TEXT,

    event_count INTEGER NOT NULL
        CHECK (event_count >= 0)
);

CREATE TABLE explainability_events (
    run_id TEXT NOT NULL COLLATE BINARY,
    sequence INTEGER NOT NULL
        CHECK (sequence > 0),

    schema_version INTEGER NOT NULL,

    span_id TEXT NOT NULL,
    parent_span_id TEXT,

    timestamp TEXT NOT NULL COLLATE BINARY,
    event_type TEXT NOT NULL,

    payload_json TEXT NOT NULL,

    PRIMARY KEY (run_id, sequence),

    FOREIGN KEY (run_id)
        REFERENCES explainability_runs(run_id)
        ON DELETE CASCADE
);

CREATE INDEX explainability_runs_by_started_at
ON explainability_runs(
    started_at DESC,
    run_id DESC
);
```

`explainability_events` 的 `PRIMARY KEY (run_id, sequence)` 本身支持
sequence replay，不额外创建重复索引。

## 21.3 PRAGMA

每次创建连接后、schema 初始化前配置并验证：

```text
foreign_keys  = ON
journal_mode  = WAL
synchronous   = FULL
busy_timeout  = 5s
```

`WAL + FULL` 为第一版固定选择，优先完整 durability；不提供
`--sqlite-synchronous` 或其他配置参数。SQLite 默认 autocheckpoint 负责
WAL checkpoint，不在每次 append/complete 后手工 checkpoint。

## 21.4 初始化规则

`open` 流程：

```text
open connection
→ configure PRAGMA（并验证）
→ BEGIN IMMEDIATE
→ inspect schema metadata
→ create / validate V1 schema
→ commit
```

* 全新 DB：在同一个 `BEGIN IMMEDIATE` transaction 内创建 meta、runs、events
  和 index，写入 `schema_version = 1`；
* 已存在 V1：验证必要表存在，不重建、不清空；
* Future version（例如 2）：拒绝为 `Internal`，不 downgrade、不删除、
  不重建；
* Partial schema（例如有 runs 无 meta）：拒绝为 `Internal`，不静默接管
  不明旧表；
* 并发首次 open：`BEGIN IMMEDIATE` 保证只有一个连接初始化，另一个等待后
  验证已提交的完整 schema。由于 `PRAGMA journal_mode = WAL` 在另一个连接
  持有写事务时不会调用 SQLite busy handler，open 初始化期间额外使用同目录
  `<database>.lock` advisory lock 串行化首次配置，锁获取受
  `SQLITE_OPEN_LOCK_TIMEOUT`（5s）限制，以 10ms 间隔轮询
  `File::try_lock()`；进程崩溃后由 OS 释放锁。

## 21.5 Run / Event 映射

时间统一使用与现有 JSON 合同相同的 RFC3339 UTC 固定 9 位纳秒 `Z`
（`SecondsFormat::Nanos`），因此 `TEXT COLLATE BINARY` 的字典序即 UTC
时间序。数据库不生成业务时间。

枚举字段通过现有 Serde 字符串合同持久化（不带 JSON 引号）：

```text
kind            → "index" | "update" | "query" | "prompt_tune" | ...
status          → "pending" | "running" | "completed" | "failed" | "cancelled"
query_method    → "basic" | "local" | "global" | "drift"
```

Event 不作为整条 Envelope 的 opaque JSON blob 保存：

```text
run_id            → explainability_events.run_id
sequence          → explainability_events.sequence
schema_version    → explainability_events.schema_version
span_id           → explainability_events.span_id
parent_span_id    → explainability_events.parent_span_id
timestamp         → explainability_events.timestamp
event_type        → payload_json["type"]（同一份序列化结果提取，不维护第二套 match）
event             → explainability_events.payload_json（紧凑 JSON）
```

读取时从列重建原始 `ExplainabilityRecord`，再通过
`ExplainabilityEnvelope::new(sequence, record)` 构造 Envelope；sequence、
timestamp 与 Event 内容都不允许被后端修改或重新编号。若
`event_type` 列与 `payload_json["type"]` 不一致，视为数据库损坏并返回
`Internal`，不猜测修复。

## 21.6 SQLite 持久化不变量

时间戳物理存储合同固定为 canonical 格式：

```text
UTC
Z
exactly 9 fractional nanosecond digits
RFC3339
```

读取时先解析 RFC3339 并转换为 UTC，再用唯一 canonical writer
（`SecondsFormat::Nanos` + `Z`）重新格式化，要求与数据库原始字符串逐字节
相等。非 canonical 表示（例如 `+08:00` 偏移、无纳秒、不足 9 位纳秒、
`+00:00`）即使表示同一 instant 也视为数据库损坏 → `Internal`。原因是
`ORDER BY started_at DESC` / cursor 比较发生在解析之前，SQL 的 lexical
ordering 依赖 canonical representation。

Run lifecycle 一致性（`Pending`/`Running` → `completed_at = NULL`；终态 →
`completed_at` 非空且 `>= started_at`）在 read 路径（`run_from_row`）与
write 路径（`append_events`、`complete_run`）统一验证。损坏的 persisted
state 一律返回 `Internal`，不会被翻译成 `RunAlreadyTerminal`、
`SequenceConflict` 或 `CompletionConflict`。

Sidecar `<database>.lock` advisory lock：

```text
只在 open / configure / schema bootstrap 期间使用
获取有界：SQLITE_OPEN_LOCK_TIMEOUT = 5s，10ms 固定轮询
lock file 可以保留在磁盘（Drop 不删除，避免 inode race）
advisory lock 属于 File handle，进程崩溃由 OS 释放
```

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
    ↓ writer sequence commit
ExplainabilityLiveHub / per-run broadcast::Sender（已实现）
    ↓
graphloom-studio SSE subscribers（已实现）
```

持久化 writer 层本身已作为 `StoreExplainabilityRecorder` 在 GraphLoom Lib
实现（`crates/graphloom/src/explainability/store_recorder.rs`），Studio
服务后续直接复用，不需要再次实现 sequence allocation 或 Envelope 构造。

---

# 23. 同步、背压与错误处理

Explainability Sink 不得在 Query 热路径中执行阻塞 I/O。

推荐使用：

```text
GraphLoom Core
      ↓ await ExplainabilitySink::emit
StoreExplainabilityRecorder
      ├── bounded adapter queue
      ↓
single writer
      ↓
分配 sequence 并生成 ExplainabilityEnvelope
      ↓
Store / JSONL
      ↓ 持久化成功后
ExplainabilityLiveHub（已实现）
      ↓
graphloom-studio Explainability SSE（已实现）
```

`StoreExplainabilityRecorder` 已实现该数据流（Store 分支）：bounded queue
默认容量 256，backpressure 通过 await capacity 提供；`finish_run` 是
persistence barrier；accepted Record 一旦写入失败，writer 进入 FAILED 并
向 `finish_run`/`shutdown` 暴露根错误。post-persistence 顺序固定为：

```text
Store.append_events succeeds
↓
writer sequence committed（state.sequences.insert）
↓
ExplainabilityLiveHub.publish（使用同一个已持久化 Envelope）
```

Live Hub 广播固定插入 committed sequence 更新之后；0 订阅者、慢客户端、断线或
广播队列满都不能影响已成功的 persistence。`Store.append_events` 成功点保留的同一
Envelope 是广播的唯一来源；Hub 不检查、重写、重排或重新分配 sequence。

这里有两个语义完全不同的有界 buffer：

* persistence writer queue：全 Recorder 有界；满时 producer await/backpressure，已经成功
  接受的 Record 不得丢失，Store failure 会使 writer FAILED；
* Live Hub broadcast ring：每 active Run 独立有界（默认 256 Envelope）；慢 subscriber
  收到 `Lagged { skipped }`，writer 不等待且 Store 不受影响。Run A 的高频流量不会使
  Run B 的 subscriber lag。

Hub map 只保存 active Run 的 channel、固定容量 ring 和 `last_sequence`，不保存完整历史、
Run metadata 或永久 tombstone。`finish_run` remove channel；recorder shutdown 或 fatal
writer failure 只关闭该 Recorder 注册的 Run。existing receiver 先 drain buffer，再得到
`Closed`。Live close 只表示该 Recorder 不再产生实时 Envelope，不等价于 Store Run
Completed；`RunCompleted` / `RunFailed` Event 也不驱动 channel close。

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
`graphloom-studio` 在 Lagged 或 live sequence gap 时重新订阅获取新 snapshot，再使用 Store
中按 sequence 保存的事件恢复；Closed 时 final catch-up 后 EOF。HTTP body polling 直接驱动
该状态机，没有 per-client background task，因此 HTTP backpressure 不会传回 persistence writer。

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

Explainability SSE 已用 in-memory Store、真实 Recorder/Live Hub 链路及 SQLite feature smoke
覆盖以下合同：

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

* Full-snapshot resolution 后的 bounded Overview；
  -Query-linked bounded depth-1 Subgraph；
  -Entity 节点；
  -Relationship 边；
  -Community 基础展示；
  -节点详情；
  -Relationship 详情；
  -Text Unit 来源查看；
  -按 Entity Type 过滤；
  -按 Community 过滤。

## 29.2 Query Chat

* 输入问题；
  -Local Query（Basic/Global/DRIFT 尚未开放完整 Explainability 生命周期）；
  -Metadata/Content/Debug Explainability 模式；
  -异步接受 Run；
  -通过独立 Query Result API 展示最终回答；
  -展示 elapsed 与 usage。

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
  -加载历史事件。

## 29.5 Explainability 与图谱联动

* SSE 与 Store replay 共用 `ExplainabilityEnvelope`；
  -按 sequence 排序与去重；
  -未知 Event 使用 forward-compatible fallback；
  -点击 Entity/Relationship/Graph Expansion Event 后，以 stable ID 请求新的 bounded Subgraph；
  -found Entity seed 与 resolved Relationship endpoint 必须实际进入 projection；
  -layout 完成后 viewport 自动 fit Query seed，seed 与普通 neighbor 使用不同视觉状态；
  -missing/unresolved seed 明确展示，不伪造节点或边；
  -Details 对已知 Event 使用 typed presenter，未知 Event 使用 forward-compatible fallback；
  -Raw JSON 只保留在默认折叠的 Developer data 中。

## 29.6 Frontend 与 Host 部署

Frontend 使用 React 19、TypeScript、Vite、Tailwind CSS v4、shadcn/Radix 与 Cytoscape.js。
浏览器只使用同源相对 `/api/*` URL，不使用外部 CDN、遥测或 raw HTML 渲染。开发模式为：

```text
Browser
↓
Vite :5173
↓ /api proxy
Rust Studio :8080
```

production-like 模式为：

```text
Browser
↓
Rust Studio :8080
├── /api/*
└── Vite static dist + SPA fallback
```

未知 `/api/*` 固定返回 404，不进入 SPA fallback。Host 默认只监听 `127.0.0.1:8080`，
不提供认证；非本机暴露必须由部署层增加认证。

Explainability Run 与 Event 默认持久化到项目下的
`.graphloom-studio/explainability.sqlite`。Query Result V1 仍是 bounded、process-local FIFO
registry：Run History 与 Explainability 可跨重启保留，而最终 Result 重启或淘汰后可返回 410。
`ExplainabilityContentMode` 只控制解释过程的内容披露，不控制 Result API 中用户主动请求的最终回答。

Graph Preview V2 每次请求 bounded Overview 或 Query-linked Subgraph，Relationship endpoint 已由
backend 在完整 snapshot 上解析成 stable Entity ID。浏览器不循环 page 加载 whole graph，也不再按
title 猜边；这不改变 `ParquetGraphDataSource` 当前 whole-table DataFrame read 的后端边界。每个新
HTTP request 重新读取 snapshot，当前未增加 cache。

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

已实现 Query-visible V1：

1. typed snapshot 与 backend-independent datasource trait；
   2.Entity list/detail；
   3.Relationship list/detail；
   4.report-backed Community list/detail；
   5.Community Report summary/detail；
   6.stable-id pagination；
   7.exact filters；
   8.基于 Parquet Provider 与 Core adapters 的实现。

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

已实现 MVP：

1. Local Query composer；
   2.Run History；
   3.Live/historical Explainability Timeline；
   4.Query Result Answer panel；
   5.Query-visible Graph Explorer 与 Cytoscape preview；
   6.Explainability stable ID → Graph highlight；
   7.feature-gated Rust API/static host。

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

### Future：Provenance Graph（V3 Phase 2，尚未实现）

未来 Studio 可以建立独立的 provenance visualization model：

```text
Document → Text Unit → Entity / Relationship → Community → Report
```

这些 Document/Text Unit/Community/Report visualization nodes 需要新的、明确的 Studio
数据合同。本模型用于解释知识来源与聚合路径，不表示 GraphRAG 原生 Entity/Relationship
graph 已经包含所有这些 node type，也不改变 GraphRAG Query 或 indexing semantics。Phase 1
不读取 raw Parquet 来绕过 API，亦不实现 provenance Graph API。

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
        ┌─────────────────────┴─────────────────────┐
        │                                           │
     tracing                              ExplainabilityRecord
        │                                           │
   ┌────┴────┐                              ┌───────┴────────┐
   │         │                              │                │
Logging     OTLP                           CLI             Studio
                                                graphloom-studio
        GraphLoom Index Output                     │
                 │                   ┌──────────────┼──────────────┐
        Query read_indexer_*          │              │              │
                 │           ExplainabilityStore  Live Hub   GraphDataSource
                 │                   │              │              │
          GraphDataSnapshot      SQLite/SSE     Live SSE      Graph Explorer
                 └───────────────────────────────────────────────┐
                                                                 ▼
                                                       Studio Frontend MVP
```

最终决策如下：

1. 统一使用名称 **Explainability**。
2. 日志和 OpenTelemetry 共用 `tracing` 插桩。
3. Explainability 与普通日志分离。
4. GraphLoom Lib 提供 Explainability Event、Query data models 与兼容 adapters；Studio 提供
   `GraphDataSource` 和 Graph Explorer HTTP DTO。
5. CLI 提供日志、OTLP 和 JSONL Adapter。
6. Studio 第一版使用 HTTP + SSE。
7. 实时展示和离线回放共用同一持久化事件流。
8. Studio 使用 `ExplainabilityStore` 抽象。
9. 第一版默认存储使用 SQLite。
10. 后续可增加 Turso 作为远程同步实现。
11. DuckDB 用于跨 Run 分析和 Benchmark，不作为默认实时 Event Store。
12. 第一阶段只完整实现 Local Query Explainability。
