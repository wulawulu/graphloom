# GraphLoom 实现规范

## 0. 文档信息

| 项目             | 内容                                         |
| -------------- | ------------------------------------------ |
| 项目名称           | GraphLoom                                  |
| 当前阶段           | Phase 1：Standard Index                     |
| 状态             | 已确认，可执行                                    |
| 实现语言           | Rust                                       |
| 参考项目           | `wulawulu/graphrag`                        |
| 参考分支           | `learn`                                    |
| 固定基准提交         | `79ab7c9ad586856e82635264c200d8a1eb3c63d9` |
| 基准 GraphRAG 版本 | `3.1.0`                                    |

本规范用于指导 Codex 实现 GraphLoom。

本规范定义：

1. 当前阶段必须实现的能力。
2. GraphLoom 与 Microsoft GraphRAG 的兼容边界。
3. crate 职责和依赖方向。
4. workflow、存储、LLM、Prompt、向量库和数据表契约。
5. 测试和验收标准。

当本规范与参考 GraphRAG 源码存在冲突时，优先级如下：

1. 本规范明确规定的偏差。
2. 固定基准提交中的 Python 实现。
3. GraphLoom 当前代码。
4. Codex 的自行推断。

不得参考基准提交之后的 GraphRAG 代码来改变本阶段行为，除非规范被显式更新。

---

# 1. 项目目标

GraphLoom 是一个独立的 Rust GraphRAG 实现。

目标不是重新设计一种 GraphRAG，而是：

> 使用 Rust 实现与固定版本 Microsoft GraphRAG 行为和数据产物兼容的索引、查询和更新系统。

项目分为三个阶段：

| 阶段      | 内容                              |
| ------- | ------------------------------- |
| Phase 1 | 完整实现 Standard Index             |
| Phase 2 | 实现 Local Search 和 Global Search |
| Phase 3 | 实现 Standard Update Workflows    |

当前只实施 Phase 1，但 Phase 1 的公共接口不得阻止 Phase 2 和 Phase 3 的实现。

---

# 2. 兼容目标

## 2.1 必须兼容

GraphLoom 必须尽可能兼容固定基准提交中的：

* 配置字段名称和语义；
* Standard Index workflow 名称与执行顺序；
* 表名称；
* 最终表列名；
* 最终表列顺序；
* Parquet 数据的逻辑类型；
* ID 和 `human_readable_id` 的生成规则；
* null、空列表和缺失字段语义；
* LLM Prompt 的业务语义；
* entity、relationship、covariate 和 community report 的解析规则；
* hierarchical Leiden 输入预处理和结果转换；
* LanceDB collection 名称与向量记录结构；
* snapshot 配置行为；
* cache 命名空间和缓存键语义；
* workflow 配置覆盖机制。

兼容的目标包括：

1. Python GraphRAG 能读取 GraphLoom 生成的标准 Parquet 表。
2. GraphLoom 能读取 Python GraphRAG 生成的标准 Parquet 表。
3. 两个实现使用相同输入和确定性 Mock LLM 时，确定性字段应产生等价结果。
4. UUID、当前日期、LLM 文本等非确定性字段允许不同，但类型、关联关系和语义必须一致。

不要求：

* Parquet 文件二进制完全相同；
* 行组大小和压缩后的字节完全相同；
* LLM 生成文本逐字相同；
* Rust 内部模块逐行复制 Python 实现；
* Rust 内部错误信息逐字匹配 Python。

## 2.2 明确偏差

GraphLoom 不实现：

* Azure OpenAI；
* Azure Blob Storage；
* Azure AI Search；
  -其他 Azure 专用功能；
* `extract_graph_nlp`；
* `prune_graph`；
* Fast Index Pipeline；
  -基于 spaCy、TextBlob、NLTK 等 NLP 组件的图提取；
* Phase 1 中的 query；
* Phase 1 中的 update workflows；
* Phase 1 中的 prompt tuning。

LLM 仅通过 OpenAI-compatible API 使用。

API 地址必须可配置，不得绑定 `api.openai.com`。

---

# 3. 仓库与 crate 结构

必须保持现有 workspace 和 crate 划分。

未经规范修改，不得新增、合并或删除 crate。

```text
crates/
├── graphloom
├── graphloom-cache
├── graphloom-chunking
├── graphloom-common
├── graphloom-input
├── graphloom-llm
├── graphloom-storage
└── graphloom-vectors
```

它们与 Python package 的对应关系如下：

| Rust crate           | Python package      |
| -------------------- | ------------------- |
| `graphloom`          | `graphrag`          |
| `graphloom-cache`    | `graphrag-cache`    |
| `graphloom-chunking` | `graphrag-chunking` |
| `graphloom-common`   | `graphrag-common`   |
| `graphloom-input`    | `graphrag-input`    |
| `graphloom-llm`      | `graphrag-llm`      |
| `graphloom-storage`  | `graphrag-storage`  |
| `graphloom-vectors`  | `graphrag-vectors`  |

顶层 `graphloom` crate 同时提供：

* library；
* CLI binary；
* GraphRagConfig；
* pipeline；
* workflows；
* index operations；
* data model；
* graph operations；
* callbacks；
* reporting；
* prompt 加载和渲染；
  -各个 provider 的最终组装。

推荐在 `crates/graphloom/src` 下按照 Python 包结构建立模块：

```text
src/
├── lib.rs
├── main.rs
├── cli/
├── config/
├── callbacks/
├── data_model/
├── graphs/
├── index/
│   ├── operations/
│   ├── typing/
│   ├── utils/
│   └── workflows/
├── logger/
├── prompts/
├── reporting/
└── tokenizer/
```

模块名称应尽量与基准 Python 文件对应，以便对照源码。

允许根据 Rust 语言特性调整单个文件内部结构，但不得把所有 workflow 和 operation 合并为一个大文件。

---

# 4. crate 职责

## 4.1 `graphloom-common`

负责与具体 GraphRAG 业务无关的公共能力：

-通用配置文件读取辅助；
-环境变量展开；
-YAML 和 JSON 公共工具；
-通用错误辅助；
-路径处理；
-敏感字段脱敏；
-少量跨 crate 公共类型。

不得放入：

-具体 workflow；

* GraphRagConfig；
  -表 schema；
* OpenAI 实现；
* Parquet 实现；
* LanceDB 实现；
  -索引业务逻辑。

## 4.2 `graphloom-cache`

负责：

* `Cache` trait；
* `MemoryCache`；
* `FileCache`；
* cache namespace；
* `child()`；
  -缓存记录序列化；
  -原子写入；
  -缓存命中、未命中和失效处理。

缓存键的创建逻辑由顶层 `graphloom` 提供，缓存 crate 只负责键值存取。

## 4.3 `graphloom-chunking`

负责：

* `Chunker` trait；
* `ChunkingConfig`；
* chunker factory；
* token overlap chunker；
* metadata prepend transformer；
* chunk 数据类型。

Phase 1 必须实现 GraphRAG 默认 token chunking：

* `size`；
* `overlap`；
* `encoding_model`；
* `prepend_metadata`。

保留通过字符串类型注册其他 chunker 的能力。

未实现的 chunker 类型必须返回明确错误，不得静默回退。

## 4.4 `graphloom-input`

负责：

* `InputConfig`；
* `TextDocument`；
* `InputReader` trait；
* input reader factory；
  -文件发现；
  -输入格式解析；
  -字符编码；
  -输入字段映射；
  -原始 metadata 保留。

文件型输入行为应参考固定基准提交的 `graphrag-input`。

InputReader 应提供异步文档流，不要求一次性把全部文档加载到内存。

## 4.5 `graphloom-llm`

负责：

* completion model 抽象；
* embedding model 抽象；
* tokenizer 抽象；
  -模型配置；
* OpenAI-compatible adapter；
* `async-openai` bridge；
  -重试；
  -并发控制；
  -响应 usage；
* completion 和 embedding 的 Mock 实现。

`async-openai` 类型不得泄露到其他 crate 的公共接口。

## 4.6 `graphloom-storage`

负责：

* `Storage` trait；
* `FileStorage`；
* `MemoryStorage`；
* `TableProvider` trait；
* `Table` trait；
* `ParquetTableProvider`；
* `MemoryTableProvider`；
* Arrow schema；
* Polars DataFrame 和 Arrow 之间的适配；
  -逐行流式表读写；
* table namespace；
* append、truncate 和原子替换。

该 crate 不得包含任何实体提取、社区生成或 workflow 业务。

## 4.7 `graphloom-vectors`

负责：

* `VectorStore` trait；
* vector store factory；
* LanceDB 实现；
  -测试用 MemoryVectorStore；
* collection schema；
  -记录写入和 upsert；
  -维度验证；
* future query 所需的检索接口。

Phase 1 的正式后端是 LanceDB。

## 4.8 `graphloom`

负责：

* GraphLoom CLI；
* GraphRagConfig；
* Provider 组装；
* PipelineFactory；
* Standard Index；
  -所有 standard workflows；
  -所有 indexing operations；
* Prompt 加载和 Tera 渲染；
* GraphRAG 最终数据模型；
* hierarchical Leiden 调用；
* workflow callbacks；
  -运行统计；
  -错误汇总和日志。

---

# 5. 依赖方向

依赖必须保持单向。

```text
graphloom-common
        ↑
        ├── graphloom-cache
        ├── graphloom-chunking
        ├── graphloom-input
        ├── graphloom-llm
        ├── graphloom-storage
        └── graphloom-vectors
                    ↑
                graphloom
```

允许子 crate 根据实际需要依赖 `graphloom-common`。

所有子 crate 均不得依赖顶层 `graphloom`。

配置类型遵循以下规则：

* `CacheConfig` 位于 `graphloom-cache`；
* `ChunkingConfig` 位于 `graphloom-chunking`；
* `InputConfig` 位于 `graphloom-input`；
* `ModelConfig` 位于 `graphloom-llm`；
* `StorageConfig` 和 `TableProviderConfig` 位于 `graphloom-storage`；
* `VectorStoreConfig` 位于 `graphloom-vectors`；
* `GraphRagConfig` 位于顶层 `graphloom`，并组合以上配置。

所有共享依赖版本必须在 workspace 根 `Cargo.toml` 中统一声明。

尤其必须统一 Arrow、Parquet、Polars 和 LanceDB 相关版本，避免出现不兼容的 Arrow 主版本。

---

# 6. 基础技术选型

| 能力                  | 技术                          |
| ------------------- | --------------------------- |
| Async runtime       | Tokio                       |
| 序列化                 | Serde                       |
| 配置                  | serde_yaml                  |
| 表计算                 | Polars                      |
| 列式模型                | Apache Arrow                |
| 文件表                 | Parquet                     |
| Prompt 模板           | Tera                        |
| OpenAI adapter      | async-openai                |
| Vector store        | LanceDB                     |
| Community detection | graspologic-native          |
| Tokenizer           | tiktoken-compatible Rust 实现 |
| Hash                | SHA-512                     |
| ID                  | UUID v4                     |
| 日志                  | tracing                     |
| CLI                 | clap                        |
| library error       | thiserror                   |
| CLI error context   | anyhow                      |

允许根据依赖兼容性选择具体 crate 版本，但必须统一放入 workspace dependencies。

不得为了暂时绕过实现而使用：

* Python 子进程运行 GraphRAG；
  -通过 HTTP 调用 Python GraphRAG；
  -把所有表存成 JSON 代替 Parquet；
  -用 SQLite 代替 TableProvider；
  -自行实现简化版 Leiden；
  -直接依赖 Azure SDK。

---

# 7. 核心架构原则

## 7.1 表是 workflow 之间的正式契约

Pipeline 不是把一个巨大的内存对象从第一步传递到最后一步。

每个 workflow 必须：

1. 从 `PipelineRunContext` 中获取 provider。
2. 从具名表读取输入。
3. 执行转换。
4. 将正式输出写回具名表。
5. 返回少量结果，仅用于日志、测试或流程状态。

不得实现一个最终统一的 `ParquetWriter` workflow。

## 7.2 WorkflowFunctionOutput 不是下游数据源

Workflow 返回值应包含：

```rust
pub struct WorkflowFunctionOutput {
    pub result: Option<WorkflowResult>,
    pub stop: bool,
}
```

`result` 仅用于：

-日志；
-显示少量 sample；
-测试；
-运行统计。

下游 workflow 不得依赖上游 `result` 传递正式数据。

## 7.3 Pipeline 是有序 workflow 列表

Phase 1 不实现通用 DAG 调度器。

Pipeline 应与基准实现一致，表现为：

```text
Vec<(WorkflowName, Workflow)>
```

PipelineFactory 必须支持：

-注册 workflow；
-批量注册 workflow；
-注册 pipeline；
-按名称构建 pipeline；
-使用配置中的 `workflows` 覆盖内置 pipeline 顺序；
-未知 workflow 名称返回明确配置错误。

---

# 8. 核心 trait

以下是行为契约，不要求逐字使用相同签名。

## 8.1 CompletionModel

Completion 与 Embedding 必须是两个独立 trait。

```rust
#[async_trait]
pub trait CompletionModel: Send + Sync {
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse>;

    fn tokenizer(&self) -> Arc<dyn Tokenizer>;
}
```

`CompletionRequest` 至少支持：

* messages；
* model；
* temperature；
* top_p；
* max completion tokens；
* stop sequences；
* plain text 输出；
* JSON 输出；
  -调用 metadata。

`CompletionResponse` 至少包含：

-生成内容；

* finish reason；
* token usage；
* provider request ID；
  -可选原始 metadata。

Phase 1 不要求 completion streaming。

## 8.2 EmbeddingModel

```rust
#[async_trait]
pub trait EmbeddingModel: Send + Sync {
    async fn embed(
        &self,
        request: EmbeddingRequest,
    ) -> Result<EmbeddingResponse>;

    fn tokenizer(&self) -> Arc<dyn Tokenizer>;
}
```

Embedding 应支持：

-单文本；
-批量文本；
-批量最大 token 数；
-批量大小；
-并发限制；
-返回顺序与输入顺序一致。

## 8.3 Tokenizer

Tokenizer 至少支持：

```rust
pub trait Tokenizer: Send + Sync {
    fn encode(&self, text: &str) -> Result<Vec<u32>>;
    fn decode(&self, tokens: &[u32]) -> Result<String>;
    fn count_tokens(&self, text: &str) -> Result<usize>;
}
```

## 8.4 Storage

Storage 用于通用文件和二进制对象，不等同于 TableProvider。

至少提供：

* `get`；
* `set`；
* `delete`；
* `has`；
* `list`；
* `child`；
  -可选文本辅助方法。

`FileStorage` 必须防止路径穿越。

## 8.5 TableProvider

至少提供：

```rust
read_dataframe(table_name)
write_dataframe(table_name, dataframe)
has(table_name)
list()
open(table_name, transformer, truncate)
child(namespace)
```

整表读写使用 Polars DataFrame。

表持久化边界使用 Arrow schema 和 Parquet。

## 8.6 Table

Table 用于逐行异步流式读取和写入。

至少支持：

-返回异步行流；
-写入一行；
-查询长度；
-关闭并提交；
-失败时放弃临时输出。

TableRow 必须表达：

* null；
* bool；
* integer；
* float；
* string；
* list；
* struct/object。

不得把列表统一序列化成逗号字符串。

## 8.7 Cache

至少支持：

* `get`；
* `set`；
* `has`；
* `remove`；
* `clear`；
* `child(namespace)`。

## 8.8 VectorStore

至少支持：

* `connect`；
* `create_or_open_collection`；
* `upsert`；
* `delete_by_ids`；
* `get_by_ids`；
* `similarity_search`；
* `close`。

即使 similarity search 在 Phase 1 不被 CLI 使用，也应保留接口供 Phase 2 使用。

## 8.9 InputReader

InputReader 应是异步 `TextDocument` 流。

## 8.10 Chunker

Chunker 接收文本和可选 transformer，返回有序 chunk。

---

# 9. PipelineRunContext

`PipelineRunContext` 必须包含：

```text
stats
input_storage
output_storage
output_table_provider
previous_table_provider
cache
callbacks
state
```

含义：

| 字段                        | 作用                                     |
| ------------------------- | -------------------------------------- |
| `stats`                   | 当前运行统计                                 |
| `input_storage`           | 输入文档存储                                 |
| `output_storage`          | snapshot 和其他长期文件输出                     |
| `output_table_provider`   | 当前索引正式表                                |
| `previous_table_provider` | Phase 3 更新时读取旧表，Phase 1 为 None         |
| `cache`                   | LLM 和其他可缓存结果                           |
| `callbacks`               | progress、error、warning 和 workflow 生命周期 |
| `state`                   | 当前运行内的任意共享状态                           |

Phase 1 不实现 update workflow，但不得删除：

* `previous_table_provider`；
* `TableProvider::child`；
* append/truncate 语义；
* `period` 字段；
* pipeline state。

为兼容性测试，应允许注入：

* UUID generator；
* Clock；
  -随机种子。

生产环境默认使用 UUID v4 和 UTC 当前日期。

---

# 10. 配置

## 10.1 配置入口

GraphLoom 从项目根目录读取：

```text
settings.yaml
.env
prompts/
input/
output/
cache/
lancedb/
```

环境变量必须支持从 `.env` 和进程环境读取。

敏感字段不得出现在普通日志中。

## 10.2 GraphRagConfig

Phase 1 至少支持以下配置区块：

```text
completion_models
embedding_models
concurrent_requests
input
input_storage
chunking
output_storage
update_output_storage
table_provider
cache
reporting
vector_store
workflows
embed_text
extract_graph
summarize_descriptions
cluster_graph
extract_claims
community_reports
snapshots
```

为后续兼容，配置解析不应因为存在以下已知但 Phase 1 未使用的区块而失败：

```text
local_search
global_search
drift_search
basic_search
async_mode
```

配置对象应允许保留 provider 自定义字段。

不得对所有结构使用严格的未知字段拒绝策略。

## 10.3 ModelConfig

Completion 和 Embedding 配置分开注册，但可以使用同一种 `ModelConfig` 数据结构。

OpenAI-compatible 配置至少支持：

* `type`；
* `model`；
* `api_key`；
* `api_base`；
* `organization`；
* `timeout`；
* `max_retries`；
* `retry_strategy`；
* `tokens_per_minute`；
* `requests_per_minute`；
* `encoding_model`。

如果 provider 类型为 Azure，必须返回明确的“不支持 Azure”配置错误。

不得静默转换成普通 OpenAI。

## 10.4 Provider factory

以下组件应提供按字符串类型创建实现的 factory：

* cache；
* chunker；
* input reader；
* storage；
* table provider；
* completion model；
* embedding model；
* vector store；
* workflow；
* pipeline。

Phase 1 不要求动态加载 `.so` 插件，但 library 用户必须能够通过 Rust API 注册自定义实现。

---

# 11. Prompt

## 11.1 模板引擎

Prompt 使用 Tera。

默认模板存放在仓库根目录：

```text
prompts/
```

模板内容应从固定基准提交复制，并只做 Tera 语法适配，不得随意缩短、改写或“优化”。

例如 Python 格式变量：

```text
{entity_types}
```

可以转换为 Tera：

```text
{{ entity_types }}
```

但业务文本和输出协议不得改变。

## 11.2 Prompt 加载优先级

Prompt 加载优先级：

1. 配置中明确指定的文件；
2. 项目根目录 `prompts/` 中的覆盖文件；
3. GraphLoom 内置默认模板。

缺失模板变量必须返回错误，不得替换为空字符串。

## 11.3 Phase 1 Prompt

至少包括：

* entity/relationship extraction；
* extraction continuation；
* extraction loop check；
* entity/relationship description summarization；
* claim/covariate extraction；
* community report generation。

## 11.4 Prompt 输出解析

解析器必须参考 Python 基准实现，支持：

-实体记录；
-关系记录；
-completion delimiter；
-record delimiter；
-tuple delimiter；

* gleaning；
  -重复实体和关系合并；
* community report JSON；
* claim JSON 或结构化记录。

对于 LLM JSON：

1. 先执行严格 JSON 解析。
2. 可修正常见、有限、可预测的格式错误。
3. 修复后仍不合法时返回带上下文的错误。
4. 不得吞掉错误并写入空对象。

---

# 12. Standard Index Pipeline

内置 standard pipeline 名称为：

```text
standard
```

严格执行以下顺序：

```text
load_input_documents
create_base_text_units
create_final_documents
extract_graph
finalize_graph
extract_covariates
create_communities
create_final_text_units
create_community_reports
generate_text_embeddings
```

不得改变顺序。

配置中的 `workflows` 非空时，可以覆盖内置顺序，但所有名称必须已注册。

每个 workflow 开始和结束时必须触发 callback 并写入日志。

workflow 失败时：

-停止执行后续 workflow；
-保留已经成功提交的前序表；
-不自动回滚整个索引；
-当前 workflow 尚未提交的临时文件必须删除；
-错误中必须包含 workflow 名称。

---

# 13. Workflow 详细契约

## 13.1 `load_input_documents`

输入：

* `input_storage`；
* `InputConfig`；
* InputReader。

输出：

```text
documents
```

行为：

-逐个读取 `TextDocument`；
-写入标准字段；
-按读取顺序分配从 0 开始的 `human_readable_id`；
-缺少 `raw_data` 时写 null；
-记录文档数量；
-返回最多 5 行 sample；
-未读到任何文档时失败。

初始 document 至少包含：

```text
id
human_readable_id
title
text
creation_date
raw_data
```

## 13.2 `create_base_text_units`

读取：

```text
documents
```

写入：

```text
text_units
```

行为：

-使用 tokenizer 和 token overlap chunker；
-支持 metadata prepend；
-保留 chunk 顺序；
-计算 `n_tokens`；
-设置 `document_id`；
-使用基准 SHA-512 规则生成 text unit ID；
-逐行流式写入；
-返回最多 5 行 sample。

初始 text unit：

```text
id
document_id
text
n_tokens
```

## 13.3 `create_final_documents`

读取：

```text
documents
text_units
```

覆盖写入：

```text
documents
```

行为：

-构建 `document_id -> text_unit_ids` 映射；
-为每个 document 填充 `text_unit_ids`；
-按照最终 documents schema 和列顺序写回；
-同表读写必须使用临时文件和原子替换。

## 13.4 `extract_graph`

读取：

```text
text_units
```

写入：

```text
entities
relationships
```

可选 snapshot：

```text
raw_entities
raw_relationships
```

行为：

1. 使用 CompletionModel 从每个 text unit 提取实体和关系。
2. 支持 `entity_types`。
3. 支持 `max_gleanings`。
4. 合并同名实体。
5. 合并相同 source/target 的关系。
6. 对重复描述调用 description summarization。
7. 保留 `text_unit_ids`。
8. 生成频率和权重等中间字段。
9. 实体为空时失败。
10. 关系为空时失败。
11. `snapshots.raw_graph` 开启时写 raw 表。

不得使用 NLP 提取。

## 13.5 `finalize_graph`

读取并覆盖写入：

```text
entities
relationships
```

行为：

-把关系视为无向边计算 degree；
-标准化反向边后去重；
-实体按 title 去重；
-关系按 `(source, target)` 去重；
-为实体生成 UUID；
-为关系生成 UUID；
-分配从 0 开始的 `human_readable_id`；
-计算 entity `degree`；
-计算 relationship `combined_degree`；
-裁剪并排序为最终列；
-同表读写使用原子替换。

可选 snapshot：

```text
graph.graphml
```

当 `snapshots.graphml` 开启时生成 GraphML。

## 13.6 `extract_covariates`

读取：

```text
text_units
```

条件写入：

```text
covariates
```

当 `extract_claims.enabled == false` 时：

-不调用 LLM；
-不要求创建 covariates 表；
-workflow 正常完成。

开启时：

-使用 CompletionModel；
-提取 claim；
-支持 gleaning；
-写入 `text_unit_id`；
-生成 UUID；
-分配 `human_readable_id`；
-按照最终 covariates schema 写入。

## 13.7 `create_communities`

读取：

```text
entities
relationships
```

写入：

```text
communities
```

行为：

1. 对 relationship 边进行标准化。
2. 支持 `use_lcc`。
3. 调用 graspologic-native hierarchical Leiden。
4. 保留 level、community、parent。
5. 聚合 community 的 entity IDs。
6. 仅聚合社区内部 relationship IDs。
7. 聚合 text unit IDs。
8. 计算 children。
9. 设置 title 为 `Community {community_id}`。
10. 设置 period 为 UTC 当前日期。
11. 设置 size 为实体数量。
12. 生成 UUID。
13. `human_readable_id` 等于 community ID。

不得自行实现简化 Leiden。

## 13.8 `create_final_text_units`

读取：

```text
text_units
entities
relationships
covariates（可选）
```

覆盖写入：

```text
text_units
```

行为：

-构建 text unit 到 entity IDs 的反向映射；
-构建 text unit 到 relationship IDs 的反向映射；
-启用 claims 时构建 covariate IDs 映射；
-按 text unit 顺序分配 `human_readable_id`；
-按照最终 text unit schema 覆盖写入；
-没有关联时写空列表，不写 null。

## 13.9 `create_community_reports`

读取：

```text
communities
entities
relationships
covariates（可选）
```

写入：

```text
community_reports
```

行为：

1. 展开 community 和 entity 关联。
2. 为缺失 description 填入 `No Description`。
3. 构建 node details。
4. 构建 edge details。
5. 构建可选 claim details。
6. 构建 community local context。
7. 按层级处理社区。
8. 在 token 限制内构建 prompt context。
9. 使用 CompletionModel 生成报告。
10. 解析结构化报告。
11. 生成 summary、findings、rank 和 rating explanation。
12. 生成 `full_content` 和 `full_content_json`。
13. 关联 parent、children、period 和 size。
14. 按最终 schema 写入。

社区报告生成失败不得静默生成空报告。

应通过 callback 报告单个社区错误；是否终止整个 workflow 应与基准行为一致。

## 13.10 `generate_text_embeddings`

读取：

```text
text_units
entities
community_reports
```

写入 LanceDB collection。

至少支持以下 embedding：

```text
text_unit_text_embedding
entity_description_embedding
community_full_content_embedding
```

逻辑来源：

| Embedding                          | 来源                                 |
| ---------------------------------- | ---------------------------------- |
| `text_unit_text_embedding`         | `text_units.text`                  |
| `entity_description_embedding`     | entity title 和 description 的兼容变换结果 |
| `community_full_content_embedding` | `community_reports.full_content`   |

行为：

-根据配置选择实际启用的 embedding；
-源表不存在时记录 warning 并跳过；
-按 token 和 batch 限制分批；
-使用 EmbeddingModel；
-保持输入输出 ID 对应关系；
-向 LanceDB 执行 upsert；
-校验 embedding 维度；
-支持并发；
-支持 cache。

当 `snapshots.embeddings` 开启时，还写入：

```text
embeddings.text_unit_text
embeddings.entity_description
embeddings.community_full_content
```

具体名称必须与基准配置中的 embedding 名称一致。

---

# 14. 最终表 schema

列名和列顺序是兼容契约。

不得根据 Rust struct 字段顺序自动决定 Parquet 列顺序。

## 14.1 `documents`

```text
id
human_readable_id
title
text
text_unit_ids
creation_date
raw_data
```

## 14.2 `text_units`

```text
id
human_readable_id
text
n_tokens
document_id
entity_ids
relationship_ids
covariate_ids
```

## 14.3 `entities`

```text
id
human_readable_id
title
type
description
text_unit_ids
frequency
degree
```

## 14.4 `relationships`

```text
id
human_readable_id
source
target
description
weight
combined_degree
text_unit_ids
```

## 14.5 `covariates`

```text
id
human_readable_id
covariate_type
type
description
subject_id
object_id
status
start_date
end_date
source_text
text_unit_id
```

## 14.6 `communities`

```text
id
human_readable_id
community
level
parent
children
title
entity_ids
relationship_ids
text_unit_ids
period
size
```

## 14.7 `community_reports`

```text
id
human_readable_id
community
level
parent
children
title
summary
full_content
rank
rating_explanation
findings
full_content_json
period
size
```

## 14.8 类型规则

最低逻辑类型要求：

* ID 和文本字段：UTF-8 string；
* `human_readable_id`：整数；
* `community`、`level`、`parent`、`size`：整数；
* `frequency`、`degree`、`combined_degree`：整数；
* `weight`、`rank`：浮点数；
* ID 集合和 children：Arrow List；
* findings：兼容的结构化列表；
* raw_data：null 或结构化对象；
  -缺失列表字段：空列表；
  -真正缺失的可选标量：null。

具体 Arrow physical type 和 nullability 应通过 Python 基准生成的 Parquet fixture 验证。

不得自行把：

* list 转为字符串；
* struct 转为 debug 文本；
  -整数无条件写为浮点数；
  -null 转为空字符串。

---

# 15. Parquet TableProvider

表名不包含扩展名。

例如：

```text
entities
```

对应：

```text
{output_dir}/entities.parquet
```

`open(table_name, truncate=true)` 必须支持：

1. 从原始文件读取。
2. 写入同目录临时文件。
3. workflow 成功关闭时 flush、sync 并原子 rename。
4. workflow 失败时删除临时文件。
5. 在替换完成前保留原始文件。

这保证以下场景安全：

```text
读取 entities
同时生成新的 entities
最后替换 entities.parquet
```

`truncate=false` 表示追加语义。

`child(namespace)` 应创建命名空间视图，而不是复制 provider。

Parquet 写入前必须执行 schema 验证和列顺序规范化。

---

# 16. Polars 与 Arrow

Polars 用于：

* select；
* join；
* group by；
* aggregation；
* explode；
* sort；
* null handling；
  -列表列变换。

Arrow 用于：

-明确 schema；

* Parquet 边界；
* LanceDB 边界；
  -跨组件交换 RecordBatch；
  -兼容性测试。

对于简单流式操作，优先使用 Table 逐行处理，不必为了使用 Polars 而加载整表。

对于基准实现本身使用 DataFrame join/groupby 的 operation，可以使用 Polars 实现。

CPU 密集型 Polars 或图计算不得长时间阻塞 Tokio worker，应在必要时使用 blocking task。

---

# 17. LLM Bridge

业务代码只能依赖：

```text
CompletionModel
EmbeddingModel
Tokenizer
```

只有 `graphloom-llm` 的 OpenAI adapter 可以直接依赖 `async-openai`。

禁止以下调用：

```text
workflow -> async-openai
operation -> async-openai
CLI -> async-openai
storage -> async-openai
```

正确关系：

```text
workflow
    ↓
CompletionModel / EmbeddingModel
    ↓
OpenAICompletion / OpenAIEmbedding
    ↓
async-openai
```

Model factory 应按模型实例名称创建并复用 client。

同一模型实例不得为每个 text unit 重新创建 HTTP client。

---

# 18. 并发、重试和限流

所有 LLM 并发必须受 `concurrent_requests` 限制。

使用 Tokio Semaphore 或等价机制。

重试仅适用于可重试错误，例如：

-连接中断；
-timeout；

* HTTP 429；
* HTTP 5xx；
* provider 明确标识的临时错误。

不得重试：

-认证失败；
-配置错误；
-prompt 模板错误；
-确定性的响应解析错误，除非基准流程要求重新请求；
-输入超出无法截断的硬限制。

重试使用带 jitter 的指数退避。

错误中必须保留：

-模型实例名；
-operation；
-workflow；
-重试次数；
-provider request ID；
-底层错误来源。

不得记录 API key。

---

# 19. Cache

Completion cache key 至少由以下内容确定：

-模型实例；
-模型名；

* messages；
* temperature；
* top_p；
* max tokens；
* response format；
  -prompt 内容；
  -业务 cache namespace。

Embedding cache key至少包含：

-模型实例；
-模型名；
-输入文本；
-embedding 参数。

缓存键必须是确定性 hash。

不同业务 operation 必须使用 child namespace，例如：

```text
extract_graph
summarize_descriptions
extract_claims
community_reports
embed_text
```

缓存读取失败可以记录 warning 后重新请求。

缓存写入失败是否终止，应由错误类型决定；不得让损坏缓存返回错误结果。

---

# 20. Community Detection

直接使用：

```text
graspologic-native
```

如果 crates.io 没有满足要求的版本，可使用固定 Git commit 依赖，但必须锁定版本。

调用前必须与基准行为一致：

1. 将 source/target 规范化成无向边。
2. 对反向重复边去重。
3. 保留基准要求的最后一条边属性。
4. 可选计算 stable largest connected component。
5. weight 缺失时使用 `1.0`。
6. 边列表按 source、target 排序。
7. 调用 hierarchical Leiden。

调用参数应对齐基准：

```text
max_cluster_size = config
seed = config
starting_communities = none
resolution = 1.0
randomness = 0.001
use_modularity = true
iterations = 1
```

输出必须保留：

```text
node
cluster
parent_cluster
level
is_final_cluster
```

并转换成 GraphRAG communities 表。

---

# 21. LanceDB

默认数据库路径来自 `vector_store` 配置。

每个 embedding 配置对应独立 collection。

每条向量记录至少包含：

```text
id
text
vector
```

允许包含额外 metadata，但不得破坏未来 query 所需字段。

要求：

-重复运行时按 ID upsert；
-不因重跑产生重复向量；

* collection 已存在时校验 schema；
  -维度变化时返回明确错误；
  -写入失败不得留下半批次不可识别状态；
  -连接只在需要时创建；
  -workflow 结束时正确释放资源。

MemoryVectorStore 仅用于单元测试和 workflow 测试。

---

# 22. CLI

Phase 1 提供：

```text
graphloom init
graphloom index
```

## 22.1 `graphloom init`

至少支持：

```text
graphloom init --root <path>
```

行为：

-创建项目目录；
-创建 `input/`；
-创建 `output/`；
-创建 `cache/`；
-创建 LanceDB 目录；
-生成默认 `settings.yaml`；
-生成 `.env` 示例；
-复制默认 prompts；
-已存在文件默认不覆盖；
-显式 `--force` 时才覆盖。

## 22.2 `graphloom index`

至少支持：

```text
graphloom index --root <path>
```

行为：

1. 加载 `.env`。
2. 加载并校验 `settings.yaml`。
3. 创建 providers。
4. 注册 built-in workflows。
5. 创建 standard pipeline。
6. 依次运行 workflows。
7. 输出 progress。
8. 输出最终统计。
9. 任一步失败时返回非零退出码。

应支持：

```text
--verbose
--dry-run
```

`--dry-run` 只校验配置、provider 和 workflow，不调用 LLM、不修改输出。

Phase 1 不暴露未实现的 query/update 命令，或者命令被调用时明确返回“当前版本未实现”，不得伪装成功。

---

# 23. 日志、Callbacks 和 Stats

使用结构化 tracing。

每个 workflow 日志至少包含：

```text
workflow_name
event
elapsed
input_rows
output_rows
```

LLM operation 还应记录：

```text
model_instance
attempt
cache_hit
input_tokens
output_tokens
```

不得默认记录完整文档、完整 prompt 或 API key。

`WorkflowCallbacks` 至少支持：

* workflow started；
* workflow completed；
* progress；
* warning；
* error；
* LLM retry；
* LLM usage。

`PipelineRunStats` 至少记录：

-文档数量；
-text unit 数量；
-实体数量；
-关系数量；
-community 数量；
-report 数量；

* LLM 请求数量；
  -cache hit/miss；
  -input/output token；
  -总耗时；
  -每个 workflow 耗时。

---

# 24. 错误处理

library 代码使用具名错误类型。

不得在生产路径使用无上下文的：

```rust
unwrap()
expect()
panic!()
```

允许在测试和明确不可违反的编译期不变量中使用。

错误类型至少区分：

* configuration；
* input；
* storage；
* table schema；
* cache；
* completion；
* embedding；
  -prompt rendering；
  -response parsing；
  -community detection；
  -vector store；
  -workflow；
  -pipeline。

错误必须保留 source chain。

CLI 使用 `anyhow` 增加运行上下文，但不得抹掉底层类型。

---

# 25. 测试要求

## 25.1 单元测试

每个 crate 必须覆盖核心 trait 和实现。

重点包括：

* cache child namespace；
* overlap chunking；
* tokenizer token 数；
  -输入字段映射；
* prompt 渲染；
* extraction parser；
* SHA-512 ID；
  -表 schema；
* Parquet round trip；
  -同表原子替换；
* relationship degree；
* LCC；
  -community hierarchy 转换；
  -LanceDB upsert；
  -重试判定。

## 25.2 Workflow 测试

每个 standard workflow 都必须有独立测试。

使用：

* MemoryStorage；
* MemoryTableProvider 或临时 Parquet；
* MockCompletionModel；
* MockEmbeddingModel；
  -固定 UUID generator；
  -固定 Clock；
  -固定 Leiden seed。

每个测试验证：

-读取了正确的输入表；
-写入了正确的输出表；
-列顺序正确；
-字段关联正确；
-错误路径正确；
-result 只包含 sample；
-未通过内存 result 向下游传递数据。

## 25.3 Standard Pipeline 集成测试

准备一组小型固定文档。

Mock LLM 返回固定：

-实体；
-关系；
-描述总结；
-claim；
-community report；
-embedding。

运行完整 standard pipeline 后验证存在：

```text
documents.parquet
text_units.parquet
entities.parquet
relationships.parquet
communities.parquet
community_reports.parquet
```

启用 claim 时还应存在：

```text
covariates.parquet
```

验证 LanceDB collection 已建立且记录数量正确。

## 25.4 Python 兼容测试

建立独立命令：

```text
make test-compat
```

该测试：

1. checkout 固定 Python GraphRAG commit；
2. 安装其依赖；
3. 使用相同 fixture；
4. 使用等价 Mock LLM 响应；
5. 分别运行 Python 和 Rust standard index；
6. 比较表名；
7. 比较列名和列顺序；
8. 比较 Arrow schema；
9. 比较归一化后的内容；
10. 交叉读取对方生成的 Parquet。

比较前允许归一化：

* UUID；
* period；
  -无业务含义的行顺序；
  -浮点精度；
* LLM 自然语言空白。

不得归一化：

-实体名称；
-关系 source/target；
-列表关联；
-community level/parent；
-weight；
-degree；

* ID 引用关系；
  -null 和空列表差异。

兼容测试可以在标准功能完成后实现，但 Phase 1 不得在兼容测试持续失败时标记完成。

---

# 26. 实现顺序

以下顺序是工程实施顺序，不是缩减版 MVP。

每一步完成后必须保持 workspace 可编译、已有测试可通过。

## Step 1：Workspace 基础

-统一 workspace dependencies；
-建立公共错误处理；
-建立 tracing；
-建立基础配置加载；
-完成 crate public API 骨架；
-确认依赖方向无循环。

## Step 2：Storage 和 TableProvider

* Storage trait；
* FileStorage；
* MemoryStorage；
* TableProvider trait；
* Table trait；
* MemoryTableProvider；
* ParquetTableProvider；
* Arrow schema；
  -原子表替换；
  -schema round-trip 测试。

## Step 3：Cache、Input 和 Chunking

* FileCache；
* MemoryCache；
* InputReader；
  -文件输入 reader；
* TextDocument；
  -tokenizer；
  -token overlap chunker；
  -metadata prepend；
  -输入与 chunking 测试。

## Step 4：LLM 和 Prompt

* CompletionModel；
* EmbeddingModel；
* Tokenizer；
* async-openai adapter；
* Mock models；
* Tera prompt loader；
  -cache key；
  -重试与并发；
  -提取结果 parser。

## Step 5：Pipeline 基础

* GraphRagConfig；
* PipelineRunContext；
* WorkflowFunctionOutput；
* Workflow registry；
* PipelineFactory；
* callbacks；
  -stats；
* `load_input_documents`；
* `create_base_text_units`；
* `create_final_documents`。

## Step 6：Graph Extraction

* extract graph operation；
* gleaning；
  -实体聚合；
  -关系聚合；
  -description summarization；
* `extract_graph`；
* `finalize_graph`；
* GraphML snapshot。

## Step 7：Covariates 和 Communities

* claim extraction；
* `extract_covariates`；
* stable LCC；
  -graspologic-native integration；
* `create_communities`；
* `create_final_text_units`。

## Step 8：Community Reports

-community context；
-token budget；
-level context；
-community report generation；

* JSON parsing；
  -report finalization；
* `create_community_reports`。

## Step 9：Embedding 和 LanceDB

-vector store factory；
-LanceDB；
-embedding batching；
-embedding snapshot；

* `generate_text_embeddings`。

## Step 10：CLI 和完整测试

* `graphloom init`；
* `graphloom index`；
  -默认 settings；
  -默认 prompts；
  -standard pipeline integration test；
  -Python compatibility test；
  -README 使用说明。

Codex 可以在步骤内部继续细分任务，但不得跳过尚未实现的正式能力并使用假实现完成后续步骤。

---

# 27. 禁止的实现方式

以下实现不被接受：

-仅创建 trait 和空实现；
-使用 `todo!()`、`unimplemented!()` 作为已完成代码；
-返回固定空 DataFrame 让 pipeline 继续；
-捕获所有错误后返回成功；
-为了通过测试而跳过 LLM workflow；
-把所有 workflow 合并进 CLI；
-使用最终统一 writer；
-直接在 workflow 中操作具体 Parquet 文件路径；
-直接在 workflow 中调用 async-openai；
-直接在 workflow 中调用 LanceDB 具体 client；
-使用 JSON 文件冒充 GraphRAG 表；
-把 list 字段存成字符串；
-用单层社区替代 hierarchical Leiden；
-删除 update 所需公共接口；
-修改最终列名；
-使用最新 GraphRAG main 代替固定 commit；
-未经规范允许改变 crate 划分；
-通过 Python 子进程代替 Rust 实现。

---

# 28. Phase 1 验收标准

Phase 1 只有同时满足以下条件才算完成。

## 构建质量

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo deny check
```

全部通过。

## CLI

以下命令可用：

```text
graphloom init --root ./example
graphloom index --root ./example
```

## 功能

-可以索引真实文本集合；
-运行完整 standard workflow；
-使用真实 OpenAI-compatible completion 和 embedding API；
-生成所有启用的标准表；
-生成 hierarchical communities；
-生成 community reports；
-写入 LanceDB；
-第二次运行不会产生损坏表；
-启用 cache 后可以复用 LLM 结果；

* claims 开启和关闭都能正确运行；
  -snapshots 配置正确工作；
  -错误时返回非零退出码。

## 数据

-最终列名正确；
-列顺序正确；
-列表字段保持列表；
-null 语义正确；
-ID 引用完整；
-Parquet 可以被 Python GraphRAG 读取；
-Python Parquet 可以被 GraphLoom 读取。

## 架构

-workflow 逐步写表；
-没有最终统一 writer；
-业务代码不直接依赖 async-openai；
-业务代码不直接依赖具体 LanceDB client；
-各 crate 职责符合本规范；
-没有循环依赖；
-没有未完成占位实现。

## 文档

README 至少包含：

-项目定位；
-与 Microsoft GraphRAG 的关系；
-安装方法；
-配置方法；

* `init` 示例；
* `index` 示例；
* OpenAI-compatible endpoint 示例；
  -输出表说明；
  -已知不支持能力；
  -兼容基准 commit。

---

# 29. 后续阶段边界

Phase 2 将新增：

* Local Search；
* Global Search；
  -查询侧数据模型；
  -LanceDB 检索；
  -上下文构建；
  -答案生成；
  -query CLI。

Phase 3 将新增：

* `load_update_documents`；
* Standard Update Workflows；
* previous table provider；
  -delta 表；
  -period 合并；
  -community 更新；
  -report 更新；
  -vector update；
  -clean state。

在对应阶段规范建立前，不实现猜测性的 query 或 update 业务。

但 Phase 1 的 trait、schema 和 provider 不得阻碍这些能力。
