# 研究：GraphRAG 索引生命周期进度

状态：已完成 · 维护者：graphloom · 日期：2026-07-15 · 源码固定：
tag object `2077c4205add901e6594aced159fca81b7a6d522`（GraphRAG v3.1.0
commit `7fc6607edda3d387d23e52ededbf8a75b6730f97`）

## 研究原因

GraphLoom 已报告索引 workflow 内部进度，但加载配置、校验模型连通性、
初始化 runtime/storage 和发布受管文件时仍可能看似无响应。本研究追踪
GraphRAG v3.1 兼容基线如何暴露这些阶段，从而区分应保持的兼容行为与值得
改进的行为。

研究只读使用用户提供的相邻 checkout。以下引用均为固定到 v3.1.0 commit
的永久上游链接。

## 架构图

```text
GraphRAG CLI                    GraphRAG API                  Pipeline runtime
    │                               │                              │
    │ load_config                   │                              │
    │   (无 logger/callback)         │                              │
    │                               │                              │
    │ init_loggers                  │                              │
    │ validate completion models ──▶ external LLM                  │
    │ validate embedding models ───▶ external embedding service    │
    │   (仅调用后日志；无实时 progress callback)                     │
    │                               │                              │
    │ build_index ─────────────────▶│ log "Initializing..."        │
    │                               │ create pipeline              │
    │◀──────────────────────────────│ pipeline_start(names)         │
    │   print workflow list         │                              │
    │                               │ run_pipeline ───────────────▶│ create storage/cache
    │                               │                              │ load context.json
    │                               │                              │ (无 phase callback)
    │◀──────────────────────────────┼──────────────────────────────│ workflow start/progress/end
    │                               │                              │ direct output writes
    │◀──────────────────────────────│ pipeline_end(results)         │ (无 publish phase)
```

GraphRAG 有两套覆盖范围不同的可观测机制：标准日志记录粗粒度生命周期，
`WorkflowCallbacks` 记录 pipeline/workflow event。Preflight 和 publication
不建模为进度阶段。

## 热路径

1. `index_cli` 在 `_run_index` 初始化日志、console callback 存在之前加载并
   解析配置；慢配置加载没有生命周期输出
   （[index.py 44-53、88-104][index-cli]）。
2. `_run_index` 在构造 `ConsoleWorkflowCallbacks`、调用 API 前校验每个
   completion/embedding model（[index.py 110、125-130][index-cli]）。
   校验顺序发出真实请求：每个 completion model 一次 completion，每个
   embedding model 一次 async embedding。只有成功后日志，首次失败即退出；
   没有 start event、spinner、per-model counter 或 callback
   （[validate_config.py 22-49][validate-config]）。
3. `build_index` 记录 `Initializing indexing pipeline...`、构造 pipeline，
   然后才发 `pipeline_start`。相邻源码注释明确指出把 initialization 传播
   给 CLI 会更清楚，但需要 API 变化
   （[API index.py 69-76][api-index]）。
4. Console callback 打印 workflow 名称和每个 start/end。`progress` 总以
   `\r` 原地打印 `completed / total`；`verbose` 只控制是否 dump 完成结果，
   不控制进度是否可见
   （[console_workflow_callbacks.py 21-50][console-callbacks]）。
5. `pipeline_start` 后，`run_pipeline` 同步构造 input/output storage、
   table provider、cache，再异步读取 `context.json`；均无 phase/item
   callback（[run_pipeline.py 39-48][run-pipeline]）。增量索引复制全部旧
   输出表也不报告进度（[run_pipeline.py 55-73、182-188][run-pipeline]）。
6. Workflow loop 才是真正 callback 边界：发 `workflow_start`，让 workflow
   报 item progress，再发 `workflow_end`。Workflow 前后及之间的 stats/
   context 写入无进度事件
   （[run_pipeline.py 127-151][run-pipeline]）。

## 关键数据结构

`Progress` 只有可选 description、total、completed 三个字段。
`ProgressTicker` 增加本地 counter，可选写包含描述与计数的标准日志，并把
值转发给一个 callback。它没有 phase ID、indeterminate state、elapsed、
rate 或 nested task model（[progress.py 16-70][progress]）。

`ConsoleWorkflowCallbacks` 是简单 terminal sink。Pipeline/workflow 生命周期
打印整行，item progress 用 carriage return 覆盖一行；索引不使用 Rich、
`tqdm` 或 persistent multi-progress renderer
（[console_workflow_callbacks.py 13-50][console-callbacks]）。

## 关键算法

进度为 `round(completed / total * 100)`。百分比被用作点填充字符串的字段
宽度，字符串以 `completed / total` 开头；这是视觉近似，不是定宽进度条。
可选字段缺失时，total 用 1、completed 用 0
（[console_workflow_callbacks.py 44-50][console-callbacks]）。

正常索引生命周期没有 publication 算法。标准索引直接为配置的 output
storage 构造 provider 并交给 workflow；state/stats 也直接写入。因此没有
isolated-generation commit/activation phase 可供 UI 表示
（[run_pipeline.py 41-48、93-105、160-179][run-pipeline]）。

## Index-to-query 可见性契约

v3.1.0 CLI 依赖命令顺序，而非存储 publication 协议：

```text
User / operator              Standard index                    Query CLI
      │                            │                               │
      │ graphrag index ──────────▶ │ 逐表直接写配置 output          │
      │ success exit ◀──────────── │                               │
      │ graphrag query ──────────────────────────────────────────▶ │
      │                            │   顺序加载所需表，再打开向量库    │
      │                            │   无 lock/generation/ready check
```

文档 happy path 明确要求索引完成后再查询
（[getting started 97-127](https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/docs/get_started.md#L97-L127)），
CLI 不强制。标准索引直接把 `output_storage` provider 给 workflow context
（[run_pipeline.py](https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/index/run/run_pipeline.py#L39-L107)）。
File backend 直接打开目标写入，不发布 staged sibling
（[file_storage.py](https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag-storage/graphrag_storage/file_storage.py#L98-L108)）。

失败时 `_run_pipeline` 返回 error result，不恢复旧表
（[run_pipeline.py](https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/index/run/run_pipeline.py#L126-L157)），
CLI 将其转换为退出码 1
（[index.py](https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/cli/index.py#L124-L135)）。
后续 Query 不因该状态被阻止，只打开 output provider 并顺序加载所需表
（[query.py](https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/cli/query.py#L374-L397)）；
`DataReader` 独立校验每张表，不校验同一 index generation
（[data_reader.py](https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/data_model/data_reader.py#L20-L71)）。
Local Search 另行打开 embedding store
（[query API](https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/api/query.py#L291-L318)）。

可观察语义：

- 索引成功后查询：预期且有文档说明的路径；
- 索引替换表时查询：可能读到新旧表混合；
- 部分写入后索引失败再查询：若所需文件仍可解析，可能消费部分输出；
- Query CLI 加载 DataFrame 后，后续 Parquet replacement 不修改内存 frame，
  但 vector-store 仍是独立 live dependency。

两条入口都不检查 ready marker、generation pointer 或跨命令 lock。GraphLoom
直接写 active output 是对正常 GraphRAG 生命周期的复现。更强 online
reindex guarantee 是显式 GraphLoom 扩展，不是兼容要求。

## 采用

- 区分 pipeline/workflow start/end 与计数型 item progress，让 callback
  简单，并让库调用者选择 renderer。
- total 已知时在普通 CLI 显示计数进度；GraphRAG 不用 `verbose` 隐藏。
- 同时保留 structured logging，便于非交互运行审计生命周期。

## 避免

- 不复制 GraphRAG 静默 connectivity wait；外部网络调用应显示当前 model
  和 indeterminate running state。
- 不把一行 `pipeline_start` 当作 storage/cache/state initialization 的
  完整覆盖；无数值 total 时也应有生命周期事件。
- 不把 carriage-return 点打印器当进度抽象；它无法表达并发、嵌套或
  indeterminate work。
- 不从 GraphRAG 推断 publication。GraphLoom `init` 有真实 staged 受管文件
  publication，而当前索引直接写 output；进度应描述真实语义，不能虚构
  index activation phase。

## 开放问题

比较范围内没有。Renderer 选择和 GraphLoom 生命周期事件 schema 应在进度
功能设计中决定，而非本 prior-art 研究。

[index-cli]: https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/cli/index.py
[validate-config]: https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/index/validate_config.py
[api-index]: https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/api/index.py
[console-callbacks]: https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/callbacks/console_workflow_callbacks.py
[run-pipeline]: https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/index/run/run_pipeline.py
[progress]: https://github.com/microsoft/graphrag/blob/7fc6607edda3d387d23e52ededbf8a75b6730f97/packages/graphrag/graphrag/logger/progress.py
