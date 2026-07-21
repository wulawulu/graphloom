# GraphLoom Phase 2：Query 兼容实现规范

> 本文件用于指导 Codex 在 GraphLoom 中实现 Query。
>
> 本阶段的目标不是实现一个“类似 GraphRAG 的 RAG 查询”，而是以 Microsoft GraphRAG `v3.1.0` 为行为基准，实现能够读取 GraphRAG/GraphLoom Phase 1 索引产物，并在 CLI、配置、数据适配、上下文构造、模型调用和结果返回方面保持兼容的 Query 子系统。

---

## 0. 基本信息

| 项目           | 内容                                   |
| ------------ | ------------------------------------ |
| 项目名称         | GraphLoom                            |
| 当前阶段         | Phase 2：Query                        |
| 前置阶段         | Phase 1：Standard Index 已完成           |
| 实现语言         | Rust                                 |
| GraphLoom 基线 | Phase 1 完成后的 `dev` 分支                |
| 参考项目         | Microsoft GraphRAG                   |
| 兼容基线         | `v3.1.0`                             |
| tag object     | `2077c4205add901e6594aced159fca81b7a6d522` |
| 固定基准提交       | `7fc6607edda3d387d23e52ededbf8a75b6730f97` |
| 公开查询方法       | `global`、`local`、`drift`、`basic`     |
| 正式表格式        | GraphRAG 3.1.0-compatible Parquet    |
| 正式向量后端       | LanceDB                              |
| Query Prompt | GraphRAG 3.1.0 原始业务文本，仅做 Tera 变量语法适配 |

本规范替代旧 `specs/README.md` 中所有仅针对 Phase 1，或把 Phase 2 简化为 Local/Global 的描述。

旧文件应改名为：

```text
specs/phase1.md
```

本文件保存为：

```text
specs/phase2.md
```

---

# 1. Phase 2 目标

Phase 2 完成后，GraphLoom 必须提供：

```bash
graphloom query "<query>"
```

并支持：

```text
global
local
drift
basic
```

四种 GraphRAG 3.1.0 查询方法。

必须同时提供：

1. 非流式 CLI；
2. 流式 CLI；
3. 统一 Rust Query API；
4. 与 GraphRAG API 对应的四组 method-specific API；
5. Query callbacks；
6. Query result、context 和 usage 统计；
7. GraphRAG 3.1.0 Query 配置；
8. GraphRAG 3.1.0 Query Prompt；
9. GraphRAG/GraphLoom 索引产物互读；
10. 对真实 OpenAI-compatible provider 的端到端查询；
11. 确定性 mock/golden 兼容测试。

Phase 2 不是只把检索结果拼进一个 Prompt。

必须完整实现每种方法对应的：

```text
读取索引产物
→ 转换为 Query data model
→ 构造检索上下文
→ 控制 token budget
→ 调用 embedding/completion
→ 解析中间结构
→ 生成或流式生成最终答案
→ 返回 context 和统计
```

---

# 2. 对旧规格的修正

## 2.1 Phase 2 不再只包含 Local Search 和 Global Search

GraphRAG 3.1.0 的 `query` CLI 公开支持：

```text
global
local
drift
basic
```

因此本阶段必须覆盖四种方法。

允许按以下顺序实现：

```text
Basic
→ Local
→ Global
→ Dynamic Global
→ DRIFT
```

但 Phase 2 的完成条件是四种方法全部可用。

不得把 `basic` 或 `drift` 留成：

```text
TODO
stub
unsupported
future phase
```

## 2.2 Query 配置必须从动态 section 升级为强类型配置

Phase 1 的 `GraphRagConfig.sections` 可以保留未知配置，但以下区块必须变成正式字段：

```yaml
local_search:
global_search:
drift_search:
basic_search:
```

不得继续只把它们保存在 `serde_json::Value` 中。

## 2.3 Query 必须补齐两个 Phase 1 尚未提供的基础能力

当前 Phase 1 的抽象不足以完成 GraphRAG-compatible Query：

1. `VectorStore` 缺少相似度检索；
2. `CompletionModel` 缺少真实流式返回。

Phase 2 必须先扩展这两个公共边界，再实现搜索算法。

不得在 Query 内部绕开现有 trait，直接创建一套私有 LanceDB/OpenAI client。

## 2.4 Query 不使用 IndexPipeline

Query 是一次读取现有正式索引并生成回答的操作，不是 workflow pipeline。

不得为了复用 Phase 1 而把 Query 伪装为 IndexWorkflow。

应增加独立的：

```text
QueryRuntime
QueryRuntimeFactory
QueryDataLoader
QueryEngine
```

但应复用 Phase 1 已有的：

```text
LoadedProject
ProjectPaths
GraphRagConfig loader
ModelFactory / ModelRegistry
ParquetTableProvider
VectorStore factory
PromptRepository
Tokenizer
错误脱敏
日志初始化
```

## 2.5 Query 不得执行任何破坏性准备

Query runtime：

* 不清理 `output/`；
* 不 reset LanceDB；
* 不创建新的空索引来掩盖缺失数据；
* 不修改 Parquet；
* 不修改向量表；
* 不重新执行 index workflow。

Query 对索引产物是只读的。

---

# 3. 兼容性契约

## 3.1 固定上游版本

实现和测试必须以：

```text
Microsoft GraphRAG v3.1.0
```

为准。

不得参考 GraphRAG `main`/`HEAD` 后静默引入新版本行为。

代码注释和测试 fixture 必须注明来源版本。

## 3.2 行为兼容优先级

优先级从高到低：

1. CLI 参数、默认值和 method dispatch；
2. 必需表和可选表；
3. Query Prompt 业务文本；
4. 上下文表头、字段、顺序和 token 截断；
5. embedding 与 completion 请求；
6. map/reduce、entity mapping、DRIFT 状态推进；
7. callback 顺序；
8. 最终答案和 context 结构；
9. usage 统计语义；
10. 内部 Rust 类型命名。

内部实现不要求逐行翻译 Python，但可观察行为必须一致。

## 3.3 不要求的兼容

不要求：

* Python 对象类型完全相同；
* pandas 与 Polars 内存表示相同；
* 异步任务调度顺序在无语义影响时完全相同；
* 非确定性模型文本逐字相同；
* DRIFT 的随机选择在真实运行中逐次相同；
* 错误堆栈文本完全相同；
* 模块目录与 Python 包逐一对应。

## 3.4 必须保留的上游细节

即使某些行为看起来不够理想，也不得未经测试擅自“优化”，例如：

* Basic Search 取得向量近邻 ID 后，按 `text_units` 输入表顺序构造上下文；
* LanceDB score 使用 GraphRAG 3.1.0 的换算语义；
* Global Search map 结果按 score 降序进入 reduce；
* score 为 0 的 global map point 不进入 reduce；
* Dynamic Community Selection 的 threshold 是“大于等于”；
* DRIFT 使用 HyDE 式 query expansion；
* Local Search 的 context budget 按 community/local/text-unit 比例切分；
* token 超限时回退到加入当前记录前的 context；
* `community_level` 过滤规则是 `level <= community_level`；
* 非 dynamic report 选择包含 community roll-up。

若认为上游存在 bug，先增加兼容测试，再单独记录偏离，不得直接改行为。

---

# 4. 范围与非目标

## 4.1 本阶段范围

* 四种 Query method；
* CLI；
* Rust API；
* Query 配置；
* Query Prompt；
* Query data model；
* Indexer-to-query adapter；
* Parquet 读取；
* LanceDB 相似度查询；
* completion streaming；
* callback；
* context/usage/result；
* GraphRAG-compatible 测试；
* README/示例更新。

## 4.2 本阶段非目标

* Standard Update Workflows；
* 增量索引；
* Prompt Tune CLI；
* HTTP Server；
* Web UI；
* 持久化多轮会话；
* Azure OpenAI；
* Azure AI Search；
* CosmosDB；
* 生产级 MemoryVectorStore；
* Query result cache；
* 对 GraphRAG `main` 的滚动兼容；
* standalone question-generation CLI；
* 引入新的 workspace crate。

`question_gen_system_prompt.txt` 仍应由 `init` 生成，以匹配 GraphRAG 项目资产，但 standalone question generation 不作为 Phase 2 完成条件。

---

# 5. Workspace 与模块边界

保持 Phase 1 workspace，不新增 crate。

## 5.1 `graphloom-llm`

负责新增：

* completion stream 类型；
* completion chunk/delta；
* `CompletionModel::stream`；
* OpenAI-compatible streaming；
* mock streaming；
* completion request 参数兼容；
* structured response 支持；
* 流式错误传播。

不得放入：

* Query context；
* Search method；
* Parquet adapter；
* GraphRAG-specific map/reduce 算法。

## 5.2 `graphloom-vectors`

负责新增：

* 向量近邻查询 trait；
* LanceDB ANN 查询；
* GraphRAG-compatible score；
* query vector 维度校验；
* top-k 参数校验；
* provider 返回解码；
* 搜索 contract tests。

不得放入：

* query text embedding；
* Entity mapping；
* Basic/Local Search；
* GraphRAG table 读取。

## 5.3 `graphloom-storage`

继续提供只读 Parquet table 能力。

除非现有接口无法安全完成 Query，不应增加 Query-specific API。

需要保证：

```rust
TableProvider::read_dataframe
TableProvider::has
TableProvider::list
```

可以用于已完成索引的只读加载。

## 5.4 顶层 `graphloom`

新增内部模块建议：

```text
src/
  api/
    query.rs
  cli/
    query.rs
  query/
    mod.rs
    callbacks.rs
    context.rs
    data_loader.rs
    data_model.rs
    engine.rs
    error.rs
    indexer_adapters.rs
    result.rs
    runtime.rs
    basic/
    local/
    global/
    drift/
```

目录可以按项目现状调整，但职责边界必须保留。

`graphloom` 负责：

* typed query config；
* Query runtime assembly；
* Query Prompt；
* data model；
* indexer adapters；
* context builders；
* search engines；
* CLI/API dispatch；
* callback chain；
* compatibility tests。

---

# 6. CLI 规范

## 6.1 命令

```bash
graphloom query [OPTIONS] <QUERY>
```

`QUERY` 是 positional argument。

## 6.2 参数

必须支持：

```text
-r, --root <ROOT>
-m, --method <METHOD>
-v, --verbose
-d, --data <DATA>
--community-level <LEVEL>
--dynamic-community-selection
--no-dynamic-selection
--response-type <TYPE>
--streaming
--no-streaming
```

## 6.3 默认值

| 参数                            | 默认值                   |
| ----------------------------- | --------------------- |
| `root`                        | `.`                   |
| `method`                      | `global`              |
| `verbose`                     | `false`               |
| `data`                        | 未设置                   |
| `community_level`             | `2`                   |
| `dynamic_community_selection` | `false`               |
| `response_type`               | `Multiple Paragraphs` |
| `streaming`                   | `false`               |

Method 枚举：

```rust
pub enum SearchMethod {
    Global,
    Local,
    Drift,
    Basic,
}
```

clap value 必须是：

```text
global
local
drift
basic
```

## 6.4 `--data`

`--data` 只覆盖 Query 读取 Parquet table 的目录。

它不应自动重写：

```yaml
vector_store.db_uri
```

LanceDB 路径仍来自已解析的项目配置。

若用户需要使用其他 LanceDB，应通过 settings 配置。

## 6.5 输出

### 非流式

stdout 只输出最终回答。

进度、debug、warning 和 usage 写入 stderr/tracing/reporting。

不得把日志混进答案。

### 流式

每收到一个 completion delta：

1. 立即写 stdout；
2. 立即 flush；
3. 调用 `on_llm_new_token`；
4. 不等待完整回答；
5. 最终补一个终端换行，但不得在 chunk 之间额外插入换行。

### 退出码

* 成功：`0`
* 配置、缺表、缺向量表、Prompt、模型、解析或运行失败：非 `0`

不得在失败后输出伪造的空成功回答。

---

# 7. Query 配置

## 7.1 LocalSearchConfig

```rust
pub struct LocalSearchConfig {
    pub prompt: Option<String>,
    pub completion_model_id: String,
    pub embedding_model_id: String,
    pub text_unit_prop: f64,
    pub community_prop: f64,
    pub conversation_history_max_turns: usize,
    pub top_k_entities: usize,
    pub top_k_relationships: usize,
    pub max_context_tokens: usize,
}
```

默认值：

```text
text_unit_prop = 0.5
community_prop = 0.15
conversation_history_max_turns = 5
top_k_entities = 10
top_k_relationships = 10
max_context_tokens = 12000
completion_model_id = default_completion_model
embedding_model_id = default_embedding_model
```

校验：

```text
0 <= text_unit_prop <= 1
0 <= community_prop <= 1
text_unit_prop + community_prop <= 1
top_k_entities > 0
top_k_relationships > 0
max_context_tokens > 0
model ids 非空且存在
```

## 7.2 GlobalSearchConfig

```rust
pub struct GlobalSearchConfig {
    pub map_prompt: Option<String>,
    pub reduce_prompt: Option<String>,
    pub knowledge_prompt: Option<String>,
    pub completion_model_id: String,
    pub max_context_tokens: usize,
    pub data_max_tokens: usize,
    pub map_max_length: usize,
    pub reduce_max_length: usize,
    pub dynamic_search_threshold: i64,
    pub dynamic_search_keep_parent: bool,
    pub dynamic_search_num_repeats: usize,
    pub dynamic_search_use_summary: bool,
    pub dynamic_search_max_level: i64,
}
```

默认值：

```text
max_context_tokens = 12000
data_max_tokens = 12000
map_max_length = 1000
reduce_max_length = 2000
dynamic_search_threshold = 1
dynamic_search_keep_parent = false
dynamic_search_num_repeats = 1
dynamic_search_use_summary = false
dynamic_search_max_level = 2
completion_model_id = default_completion_model
```

校验：

```text
max_context_tokens > 0
data_max_tokens > 0
map_max_length > 0
reduce_max_length > 0
dynamic_search_num_repeats > 0
dynamic_search_max_level >= 0
completion model 存在
```

## 7.3 BasicSearchConfig

```rust
pub struct BasicSearchConfig {
    pub prompt: Option<String>,
    pub completion_model_id: String,
    pub embedding_model_id: String,
    pub k: usize,
    pub max_context_tokens: usize,
}
```

默认值：

```text
k = 10
max_context_tokens = 12000
completion_model_id = default_completion_model
embedding_model_id = default_embedding_model
```

## 7.4 DriftSearchConfig

```rust
pub struct DriftSearchConfig {
    pub prompt: Option<String>,
    pub reduce_prompt: Option<String>,
    pub completion_model_id: String,
    pub embedding_model_id: String,

    pub data_max_tokens: usize,
    pub reduce_max_tokens: Option<u32>,
    pub reduce_temperature: f64,
    pub reduce_max_completion_tokens: Option<u32>,
    pub concurrency: usize,
    pub drift_k_followups: usize,
    pub primer_folds: usize,
    pub primer_llm_max_tokens: usize,
    pub n_depth: usize,

    pub local_search_text_unit_prop: f64,
    pub local_search_community_prop: f64,
    pub local_search_top_k_mapped_entities: usize,
    pub local_search_top_k_relationships: usize,
    pub local_search_max_data_tokens: usize,
    pub local_search_temperature: f64,
    pub local_search_top_p: f64,
    pub local_search_n: usize,
    pub local_search_llm_max_gen_tokens: Option<u32>,
    pub local_search_llm_max_gen_completion_tokens: Option<u32>,
}
```

默认值：

```text
data_max_tokens = 12000
reduce_max_tokens = None
reduce_temperature = 0
reduce_max_completion_tokens = None
concurrency = 32
drift_k_followups = 20
primer_folds = 5
primer_llm_max_tokens = 12000
n_depth = 3
local_search_text_unit_prop = 0.9
local_search_community_prop = 0.1
local_search_top_k_mapped_entities = 10
local_search_top_k_relationships = 10
local_search_max_data_tokens = 12000
local_search_temperature = 0
local_search_top_p = 1
local_search_n = 1
local_search_llm_max_gen_tokens = None
local_search_llm_max_gen_completion_tokens = None
completion_model_id = default_completion_model
embedding_model_id = default_embedding_model
```

`primer_folds` 配成 `0` 时，运行语义按 GraphRAG 处理为至少 `1` fold；配置校验也可以直接拒绝 `0`，但必须有测试明确选定行为。优先保持上游运行语义。

## 7.5 serde 兼容

必须接受 GraphRAG snake_case：

```yaml
completion_model_id:
embedding_model_id:
max_context_tokens:
text_unit_prop:
community_prop:
```

GraphLoom 现有 camelCase 形式可继续作为兼容别名，但 `init` 输出必须使用 GraphRAG canonical snake_case。

## 7.6 默认 settings.yaml

`graphloom init` 生成的 settings 至少包含：

```yaml
local_search:
  completion_model_id: default_completion_model
  embedding_model_id: default_embedding_model
  prompt: prompts/local_search_system_prompt.txt

global_search:
  completion_model_id: default_completion_model
  map_prompt: prompts/global_search_map_system_prompt.txt
  reduce_prompt: prompts/global_search_reduce_system_prompt.txt
  knowledge_prompt: prompts/global_search_knowledge_system_prompt.txt

drift_search:
  completion_model_id: default_completion_model
  embedding_model_id: default_embedding_model
  prompt: prompts/drift_search_system_prompt.txt
  reduce_prompt: prompts/drift_reduce_prompt.txt

basic_search:
  completion_model_id: default_completion_model
  embedding_model_id: default_embedding_model
  prompt: prompts/basic_search_system_prompt.txt
```

其余 knobs 可依赖 Rust default，不必全部展开到 init 文件。

---

# 8. Query Prompt

## 8.1 项目 Prompt 资产

`graphloom init` 必须新增：

```text
prompts/
  basic_search_system_prompt.txt
  drift_search_system_prompt.txt
  drift_reduce_prompt.txt
  global_search_map_system_prompt.txt
  global_search_reduce_system_prompt.txt
  global_search_knowledge_system_prompt.txt
  local_search_system_prompt.txt
  question_gen_system_prompt.txt
```

加上 Phase 1 的 5 个 Prompt，初始化后受管 Prompt 总数应为 13。

## 8.2 Prompt 来源

内容必须来自 GraphRAG `v3.1.0`：

```text
graphrag/prompts/query/
```

只允许：

* Python `.format` 占位符转为 Tera；
* 为转义字面花括号做必要处理；
* 统一换行符；
* 适配现有 PromptRepository。

不得：

* 翻译；
* 缩短；
* 改写语气；
* 删除 citation 规则；
* 改写 JSON 输出字段；
* “优化”提示词；
* 将多个 Prompt 合并为一个。

## 8.3 内置但不写入项目的 Prompt

以下上游内部 Prompt 不由 GraphRAG init 写成独立文件，应作为 Query 内部常量实现：

* DRIFT primer Prompt；
* DRIFT HyDE query expansion Prompt；
* Dynamic Community Selection rate Prompt；
* Global Search 无数据固定回答；
* 解析/修复 JSON 所需固定文本。

除非 GraphRAG 3.1.0 允许配置，否则不得擅自增加配置路径。

## 8.4 加载优先级

继续沿用 Phase 1：

1. 配置指定的文件或 inline Prompt；
2. 项目 `prompts/` 对应覆盖文件；
3. GraphLoom 内置默认模板。

相对路径基于 project root。

不得依赖 process cwd。

## 8.5 Prompt 变量

至少覆盖：

```text
context_data
response_type
max_length
report_data
global_query
followups
query
community_reports
```

缺少变量时必须报错，不得替换为空字符串。

---

# 9. Query 数据依赖

## 9.1 表依赖矩阵

| Method   | 必需表                                                                       | 可选表          |
| -------- | ------------------------------------------------------------------------- | ------------ |
| `global` | `entities`、`communities`、`community_reports`                              | 无            |
| `local`  | `entities`、`communities`、`community_reports`、`text_units`、`relationships` | `covariates` |
| `drift`  | `entities`、`communities`、`community_reports`、`text_units`、`relationships` | 无            |
| `basic`  | `text_units`                                                              | 无            |

文件名由 `TableProvider` 解析为对应 Parquet。

不得要求 Query 不使用的表存在。

例如 Basic Search 不应因为缺少 `entities.parquet` 而失败。

## 9.2 向量表依赖矩阵

| Method   | LanceDB index                                 |
| -------- | --------------------------------------------- |
| `global` | 无；dynamic selection 也不使用向量表                   |
| `local`  | `entity_description`                          |
| `drift`  | `entity_description`、`community_full_content` |
| `basic`  | `text_unit_text`                              |

缺少所需向量表时必须返回明确错误。

不得自动创建空表后继续生成无依据回答。

## 9.3 QueryDataLoader

实现只读 loader：

```rust
pub(crate) struct QueryDataLoader {
    table_provider: Arc<dyn TableProvider>,
}
```

建议 API：

```rust
async fn load_global(&self) -> Result<GlobalQueryData>;
async fn load_local(&self) -> Result<LocalQueryData>;
async fn load_drift(&self) -> Result<DriftQueryData>;
async fn load_basic(&self) -> Result<BasicQueryData>;
```

loader 负责：

* 按方法检查表；
* 读取 DataFrame；
* 调用 indexer adapters；
* 对可选 covariates 做存在性判断；
* 给出表名和路径上下文；
* 不修改原表；
* 不执行全目录扫描来猜表名。

---

# 10. Query Data Model 与 Indexer Adapters

不得直接让搜索算法在任意 DataFrame 列上到处做字符串索引。

应先转换为强类型内部模型：

```rust
Entity
Relationship
Community
CommunityReport
TextUnit
Covariate
```

若 Phase 1 已有语义相同类型，应复用或抽取公共字段；不得建立两个容易漂移的 public model。

## 10.1 Entity adapter

从 `entities` 与 exploded `communities.entity_ids` 构造。

行为：

1. 按 entity id left join community；
2. 应用 `community_level`：`level <= configured level`；
3. 未命中 community 使用 `-1`；
4. 每个 entity 聚合去重后的 community IDs；
5. community ID 转为十进制字符串；
6. 再与原 entity 表合并；
7. 每个 entity id 只保留一条。

字段映射：

```text
id                    → id
human_readable_id     → short_id
title                 → title
type                  → entity_type
description           → description
community aggregation → community_ids
degree                → rank
text_unit_ids         → text_unit_ids
```

## 10.2 Relationship adapter

字段映射：

```text
id                → id
human_readable_id → short_id
source            → source
target            → target
description       → description
weight            → weight
combined_degree   → rank
text_unit_ids     → text_unit_ids
```

## 10.3 Community Report adapter

行为：

1. 对 communities 和 reports 应用 `community_level`；
2. 非 dynamic 模式执行 roll-up：

   * explode entity membership；
   * 每个 entity 选择最大 community id；
   * 只保留这些 community 对应 report；
3. dynamic 模式不执行该 roll-up；
4. short id 使用 `community`。

字段：

```text
id
community
title
summary
full_content
rank
```

## 10.4 Community adapter

Dynamic Global Search 需要完整 hierarchy。

字段：

```text
id
community → short_id
title
level
parent
children
```

若 community 没有 report：

* 记录 warning；
* 从 dynamic selection 可用 community 集合移除；
* 不得 panic。

## 10.5 TextUnit adapter

字段：

```text
id
text
entity_ids
relationship_ids
covariate_ids
n_tokens
document_id
```

`short_id` 按 GraphRAG adapter 语义使用 DataFrame reset 后的行号字符串，而不是 UUID 截断。

## 10.6 Covariate adapter

`covariates.id` 先按字符串解释。

字段：

```text
id
human_readable_id → short_id
subject_id
type
object_id
status
start_date
end_date
description
```

Local Search 中按 covariate type 分组。

## 10.7 类型兼容

Adapter 至少接受 GraphRAG/GraphLoom 产出的：

* UTF-8/string；
* signed/unsigned 常用整数；
* float32/float64；
* list<string>；
* nullable scalar/list。

不得因 Polars 推断出 `UInt32` 而只接受 `Int64`。

字段缺失错误必须包含：

```text
method
table
column
expected type
actual type
```

---

# 11. Query Runtime

## 11.1 结构

当前实现把运行时分为长期快照与请求态：

```text
┌──────────────────────── QueryEngine snapshot ────────────────────────┐
│ method-keyed async once cells                                        │
│  ├─ adapted Parquet data + QueryDataIndex                            │
│  ├─ completion / embedding models + tokenizer                        │
│  ├─ compiled prompts                                                  │
│  └─ validated vector connection/schema                               │
└──────────────────────────────┬────────────────────────────────────────┘
                               │ Arc<QueryEngine>
                 ┌─────────────┴─────────────┐
                 ▼                           ▼
       request A orchestration     request B orchestration
       query/history/callbacks     query/history/callbacks
       usage/context/stream        usage/context/stream
```

每个 method/data key 在首次准备时成为显式 snapshot；该 key 的索引更新后由调用方重新创建
engine，不自动监听文件变化。尚未首次查询的方法仍在首次准备时读取当时的索引文件。
Global static 与 dynamic selection 的 report adaptation 不同，因此是两个独立 snapshot key。
每次请求的 project root 必须 canonicalize 到 engine load root；相对 data override 以该 root
解析，engine 不允许跨 project 查询。成功解析到同一目录的 data override 共享 snapshot；
无法解析的 override 不进入长期资源缓存，而是直接执行正常 runtime loading，由 runtime
返回原有 typed error，后续请求会重新解析该路径。若 unresolved 请求在加载时恰好变为有效
并成功完成，该次 runtime 仍不缓存；下一次请求通过 canonical key 建立正式 snapshot。
callback、conversation history、usage、streaming 与 DRIFT traversal 不得进入长期缓存。

Global Search 虽不使用向量表，也不应强制连接 LanceDB。

因此实际实现可以把 vector store 延迟创建或包装为：

```rust
enum QueryVectorStoreService {
    Required(Arc<dyn VectorStore>),
    Unused,
}
```

## 11.2 QueryRuntimeFactory

职责：

* 根据 method 计算 requirements；
* 只创建所需 completion/embedding model；
* 只检查所需表；
* 只连接所需向量后端；
* 加载 method 对应 Prompt；
* 每次请求组装 callback chain；
* 不创建 cache；
* 不修改 index 产物。

## 11.3 Method Requirements

公开 capability introspection：

```rust
pub(crate) struct QueryRequirements {
    tables: BTreeSet<QueryTable>,
    embeddings: BTreeSet<QueryEmbedding>,
    completion_models: BTreeSet<String>,
    embedding_models: BTreeSet<String>,
    prompts: BTreeSet<QueryPrompt>,
}
```

测试必须证明 Basic Search 不实例化不需要的模型、表和 Prompt。

`QueryRequirements` 不作为泛型 runtime loader。各 method 保持明确的 typed loading code，
exact-set tests 负责锁定矩阵；runtime 不使用脆弱的集合长度检查。

## 11.4 Query preflight

preflight 应检查：

* 配置文件和环境变量；
* 对应 model id；
* model provider/auth；
* 必需 Prompt；
* 必需 Parquet；
* vector db 路径；
* 必需 LanceDB table schema；
* 向量维度与 embedding model请求预期；
* `community_level >= 0`；
* method-specific config。

Query preflight 不应发送外部模型请求。

CLI 不需要新增 `--dry-run`，除非 GraphRAG 兼容范围另行修改。

---

# 12. Completion Streaming

## 12.1 Trait

在 `graphloom-llm` 增加：

```rust
pub type CompletionStream =
    Pin<Box<dyn Stream<Item = Result<CompletionChunk>> + Send>>;

#[async_trait]
pub trait CompletionModel: Send + Sync + Debug {
    fn validate_request(&self, request: &CompletionRequest) -> Result<()>;

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse>;

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionStream>;
}
```

## 12.2 Chunk 类型

至少保留：

```rust
pub struct CompletionChunk {
    pub id: Option<String>,
    pub model: Option<String>,
    pub choices: Vec<CompletionChunkChoice>,
    pub usage: Option<CompletionUsage>,
}

pub struct CompletionChunkChoice {
    pub index: usize,
    pub delta: CompletionDelta,
    pub finish_reason: Option<String>,
}

pub struct CompletionDelta {
    pub content: Option<String>,
}
```

只保留 Query 需要的字段也可以，但 provider-specific 类型不得泄漏到 public trait。

## 12.3 OpenAI adapter

真实 streaming 必须调用 provider 的 streaming API。

不得：

* 先完整 `complete`；
* 再按字符切分伪造流；
* 把整段答案当一个“流式” chunk，作为正式 OpenAI 实现。

Mock/custom model 可以使用默认单 chunk fallback，但测试必须区分真实 streaming adapter 与 fallback。

## 12.4 请求参数

Query 必须能够设置：

```text
temperature
top_p
n
max_tokens
max_completion_tokens
response_format
stream
```

Global map 和 DRIFT intermediate call 需要 structured JSON response。

现有 `CompletionRequest.response_format: Value` 可继续使用，但必须提供安全的 JSON Schema builder，不得手写不稳定 map。

## 12.5 Cache

Phase 2 Query 不启用 Phase 1 LLM cache。

理由：

* GraphRAG query CLI 没有 index cache 开关；
* 流式 completion 与现有 cache middleware 语义不同；
* Query 回答不应默认成为持久 index cache。

不得把 Query 请求写入 `cache/`。

后续若增加 Query cache，应另立规格。

---

# 13. VectorStore 相似度检索

## 13.1 Trait

扩展：

```rust
async fn similarity_search_by_vector(
    &self,
    schema: &VectorIndexSchema,
    query_vector: &[f32],
    k: usize,
    include_vectors: bool,
) -> Result<Vec<VectorSearchResult>>;
```

可增加 convenience 方法，但 text embedding 必须留在 Query 层：

```text
embed query
→ similarity_search_by_vector
```

`graphloom-vectors` 不依赖 LLM crate。

## 13.2 校验

* `k > 0`；
* query vector 非空；
* 维度等于 schema；
* 所有元素 finite；
* table 存在；
* table schema 匹配；
* `_distance` 可解码；
* id 非空。

## 13.3 LanceDB score

GraphRAG 3.1.0 LanceDB 使用：

```text
score = 1 - abs(_distance)
```

GraphLoom 必须保持这一行为。

不得未经兼容测试改为：

```text
-distance
1 / (1 + distance)
cosine similarity
```

## 13.4 顺序

保留 LanceDB 返回顺序。

不得在 store 层按 id 重排。

若测试需要稳定 tie，fixture 应避免完全相同的距离，或只在兼容测试明确证明上游 tie 无序后做受控处理。

## 13.5 include_vectors

* Entity/Basic mapping 通常不需要把向量复制到结果；
* DRIFT 读取 community report embedding 时需要完整向量；
* `include_vectors=false` 时结果 document vector 可为空。

现有 `get_by_id` 继续用于 DRIFT report embedding hydration。

---

# 14. 公共 Query Result 与 Callback

## 14.1 QueryResult

```rust
pub struct QueryResult {
    pub response: String,
    pub context: QueryContext,
    pub elapsed: Duration,
    pub usage: QueryUsage,
}
```

## 14.2 QueryUsage

```rust
pub struct QueryUsage {
    pub llm_calls: usize,
    pub prompt_tokens: usize,
    pub output_tokens: usize,
    pub categories: BTreeMap<String, QueryUsageCategory>,
}
```

category 至少支持：

```text
build_context
map
reduce
primer
action
response
```

## 14.3 QueryContext

需要同时表达：

* 实际进入 Prompt 的 context text；
* context records；
* Global 的多个 batch；
* DRIFT 每个 action 的 context。

可以使用：

```rust
pub enum QueryContextText {
    Empty,
    Text(String),
    Batches(Vec<String>),
    Named(BTreeMap<String, String>),
}

pub enum QueryContextRecords {
    Empty,
    Tables(BTreeMap<String, DataFrame>),
    Batches(Vec<DataFrame>),
    Named(BTreeMap<String, QueryContextRecords>),
}
```

不得只返回最终答案而丢弃 context。

## 14.4 Callback

```rust
pub trait QueryCallbacks: Send + Sync + Debug {
    fn on_context(&self, context: &QueryContext);
    fn on_map_response_start(&self, contexts: &[String]);
    fn on_map_response_end(&self, outputs: &[MapSearchResult]);
    fn on_reduce_response_start(&self, context: &ReduceContext);
    fn on_reduce_response_end(&self, output: &str);
    fn on_llm_new_token(&self, token: &str);
}
```

提供：

```text
NoopQueryCallbacks
QueryCallbackChain
Console/CLI callback（需要时）
```

Callback panic 不得破坏核心运行；若 Rust trait 无法隔离 panic，则文档明确 callback 不得 panic，并在测试 callback 中遵守。

---

# 15. Basic Search

## 15.1 流程

```text
query
→ embedding model
→ text_unit_text ANN，k = basic_search.k
→ 取得命中 id 集合
→ 从 text_units 选择命中行
→ 构造 Sources 表
→ token 截断
→ basic system prompt
→ completion
```

## 15.2 Context 顺序

为兼容 GraphRAG 3.1.0：

1. ANN 结果用于建立命中 ID 集合；
2. 遍历原 `text_units` 列表；
3. 选择 ID 在集合中的记录；
4. 按原表顺序生成上下文。

不得默认按 ANN score 重新排序。

## 15.3 Context 格式

逻辑表：

```text
id|text
```

其中 `id` 是 TextUnit `short_id`，即 adapter 生成的行号字符串。

使用：

```text
delimiter = |
escapechar = \
index = false
```

必须有 header。

token budget 从 header 开始计数。

逐行加入；若下一行导致：

```text
current_tokens + row_tokens > max_context_tokens
```

则停止并保留上一状态。

## 15.4 空 query

按 GraphRAG 行为，空 query 构造空 Sources 表，而不是执行 embedding search。

CLI positional argument 仍是必需的。

## 15.5 Prompt 与模型

system prompt variables：

```text
context_data
response_type
```

messages：

```text
system: rendered prompt
user: original query
```

流式 API 使用 `CompletionModel::stream`。

非流式 API 可以直接 `complete`，也可以消费相同 stream 聚合；优先让流式/非流式共享一套 orchestration，避免行为漂移。

---

# 16. Local Search

## 16.1 流程

```text
query
→ 可选 conversation history 拼接到 entity mapping query
→ query embedding
→ entity_description ANN
→ 映射 top-k entities
→ community context
→ entity/relationship/covariate context
→ text unit context
→ 拼接 mixed context
→ local system prompt
→ completion
```

## 16.2 Query-to-Entity mapping

参数：

```text
top_k = local_search.top_k_entities
oversample_scaler = 2
vector key = entity id
```

行为：

* ANN 返回 entity IDs；
* 映射到已加载 entity；
* 支持 include/exclude names 的内部 API；
* 保留 ANN 映射顺序；
* 找不到的向量 ID 记录 warning 并跳过；
* 不得将 title 当 vector key，默认 key 是 id。

Phase 2 CLI 不需要暴露 include/exclude flags，但内部 context builder 应保留扩展点。

## 16.3 Conversation history

Phase 2 CLI 不持久化 history。

Rust API 可以接受：

```rust
Option<ConversationHistory>
```

用于兼容 engine 行为。

Entity mapping query：

```text
current query
+ previous user questions
```

最多：

```text
conversation_history_max_turns
```

默认只加入 user turns。

History context 在 token budget 中优先分配。

## 16.4 Token budget

先扣除 history context。

剩余 budget：

```text
community_tokens = remaining * community_prop
local_tokens     = remaining * (1 - community_prop - text_unit_prop)
text_unit_tokens = remaining * text_unit_prop
```

转换为非负整数时保持明确、稳定的截断规则，优先使用与 Python `int()` 一致的向零取整。

## 16.5 Community context

1. 遍历 mapped entities 的 `community_ids`；
2. 统计每个 community 匹配的 entity 数；
3. 只选择有 report 的 community；
4. 按：

   ```text
   matches DESC
   rank DESC
   ```

   排序；
5. 使用 full content，除非内部参数要求 summary；
6. `shuffle_data=false`；
7. `single_batch=true`；
8. 默认不显示 community rank；
9. 达到 budget 后停止。

## 16.6 Entity context

默认包含：

```text
id
entity
description
number of relationships
```

实际 header 必须以 GraphRAG 3.1.0 helper 输出为准。

entity rank 使用 degree。

## 16.7 Relationship/Covariate context

逐个扩大已加入 entity 集合：

1. 构造与当前 entity 集合相关的 relationships；
2. 排序并截取 `top_k_relationships`；
3. 构造每种 covariate context；
4. 计算 entity + relationship + covariates 总 token；
5. 超限时回退到前一个 entity 集合；
6. 不把当前超限结果部分写入最终 context。

默认：

```text
include_relationship_weight = true
relationship_ranking_attribute = rank
```

## 16.8 TextUnit context

候选来自 mapped entities 的 `text_unit_ids`。

去重后排序：

```text
mapped entity order ASC
associated relationship count DESC
```

然后按 token budget 顺序加入。

## 16.9 Context 拼接

顺序：

```text
conversation history（存在时）
community reports
entities + relationships + covariates
sources
```

各 section 之间使用两个换行。

## 16.10 Local Prompt

普通 Local：

```text
context_data
response_type
```

DRIFT 内部复用 Local 时还要支持：

```text
global_query
followups
```

因此 Prompt renderer 不能只为普通 Local 写死变量集合。

---

# 17. Global Search

## 17.1 固定 Community Selection 流程

```text
加载 reports/entities/communities
→ community_level 过滤与 report roll-up
→ 构造多个 community context batch
→ 并行 map completion
→ 解析 points
→ 汇总、过滤和排序 key points
→ 构造 reduce context
→ reduce completion
```

## 17.2 Global Context

默认参数：

```text
use_community_summary = false
shuffle_data = true
include_community_rank = true
min_community_rank = 0
include_community_weight = true
normalize_community_weight = true
context_name = Reports
random_state = 86
max_context_tokens = global_search.max_context_tokens
single_batch = false
```

`shuffle_data=true` 必须使用可复现的固定随机状态 86。

不得使用 thread RNG 导致同一输入每次 batch 不同。

## 17.3 Community weight

按 GraphRAG helper 语义计算 entity occurrence weight，并规范化。

上下文 header 名必须保持：

```text
rank
occurrence weight
```

不得自行改为中文或其他字段名。

## 17.4 Map

每个 context batch 独立调用 completion。

并发上限：

```text
config.concurrent_requests
```

map prompt variables：

```text
context_data
max_length = global_search.map_max_length
```

user message：

```text
original query
```

map response 必须请求 JSON object，并解析：

```json
{
  "points": [
    {
      "description": "...",
      "score": 80
    }
  ]
}
```

转换为：

```rust
MapPoint {
    answer: description,
    score: i64,
}
```

解析规则：

* 完整 JSON；
* 可从 fenced/prose 中提取第一个 JSON object；
* `points` 缺失、不是 array 或元素缺字段时忽略无效元素；
* 整个 batch 解析失败时得到一条空 answer、score 0 的 fallback；
* provider/transport error 不得伪装成 JSON parse error，应返回 typed error，除非专门的兼容测试证明需按上游 batch fallback。

## 17.5 Reduce key points

对所有 map points 附加：

```text
analyst = map batch index
```

过滤：

```text
score > 0
```

排序：

```text
score DESC
```

格式：

```text
----Analyst {N}----
Importance Score: {score}
{answer}
```

`N` 从 1 开始。

按 `data_max_tokens` 顺序加入。

## 17.6 无数据回答

若没有任何 `score > 0` 的 point，且 `allow_general_knowledge=false`：

* 不调用 reduce LLM；
* 返回 GraphRAG 3.1.0 的固定 `NO_DATA_ANSWER`；
* reduce llm_calls 为 0。

Phase 2 public CLI 固定：

```text
allow_general_knowledge = false
```

`knowledge_prompt` 仍加载并保留 API 扩展能力，但 CLI 不新增开关。

## 17.7 Reduce

Prompt variables：

```text
report_data
response_type
max_length = global_search.reduce_max_length
```

最终回答应支持真实 streaming。

## 17.8 Callback 顺序

```text
on_map_response_start
map calls
on_map_response_end
on_context
on_reduce_response_start（开始 reduce 时）
on_llm_new_token*
on_reduce_response_end
```

若无数据直接返回，也应以测试定义合理的 reduce callback 行为，不得出现 start 无 end。

---

# 18. Dynamic Community Selection

仅用于：

```text
method = global
dynamic_community_selection = true
```

## 18.1 初始化

需要完整：

```text
Community
CommunityReport
```

建立：

```text
report by community short_id
community by short_id
level → community IDs
```

起始 queue：

```text
level 0 communities
```

## 18.2 Rating

对 queue 中每个 community：

* 选择 `summary` 或 `full_content`；
* 按 query 评分；
* 重复 `dynamic_search_num_repeats`；
* bounded concurrency；
* 汇总 rating；
* rating `>= threshold` 视为相关。

rate Prompt 和解析必须来自 GraphRAG 3.1.0。

## 18.3 Hierarchy traversal

相关 community：

* 加入结果；
* 将存在 report 的 children 放入下一 queue；
* `keep_parent=false` 时，在继续深入后移除 parent。

若当前 queue 耗尽且尚无任何 relevant community：

* 尝试下一 level 的全部 report；
* 最大不超过 `dynamic_search_max_level`。

## 18.4 输出

Dynamic selection 返回：

```text
selected reports
llm_calls
prompt_tokens
output_tokens
ratings
```

ratings 至少用于 debug/context metadata，不必直接输出到 CLI。

## 18.5 错误与空 hierarchy

* 缺 level 0 时返回明确 InvalidData；
* child 无 report 时 debug，不 panic；
* community/report 不一致时 warning；
* rating parse 失败按上游 rate helper 语义处理，并有 golden test。

---

# 19. DRIFT Search

DRIFT 是 Phase 2 中最后实现的方法，不得用“调用一次 Local Search”替代。

## 19.1 总流程

```text
query
→ HyDE expansion
→ expanded query embedding
→ community_full_content cosine ranking
→ top-k reports
→ primer folds
→ structured primer responses
→ 初始化 action graph
→ 多层 follow-up Local Search
→ action state serialization
→ final reduce
```

## 19.2 Query 要求

DRIFT 的空字符串 query 必须报错：

```text
DRIFT Search query cannot be empty
```

## 19.3 Community report embedding hydration

从：

```text
community_full_content
```

LanceDB table 按 report `id` 读取向量，写入内部 `CommunityReport.full_content_embedding`。

所有进入 DRIFT ranking 的 report 必须：

* 有 `full_content`；
* 有 embedding；
* embedding 类型和维度一致。

缺失数量必须出现在错误中。

## 19.4 HyDE expansion

从所有 reports 中随机选一个 `full_content` 作为结构模板。

向 completion model 请求：

```text
Create a hypothetical answer...
```

要求不引入 query 中不存在的新 named entities。

* completion 为空时 warning，并回退 original query；
* 对 expanded text 调 embedding；
* usage 计入 build_context；
* production 保持随机行为；
* 测试通过私有 RNG 注入固定 report，不得改变 public config。

## 19.5 Report ranking

GraphRAG 3.1.0 在内存中计算 cosine similarity：

```text
dot(query, doc) / (norm(query) * norm(doc))
```

按 similarity 取：

```text
drift_k_followups
```

输出字段：

```text
short_id
community_id
full_content
```

不得用 `entity_description` ANN 代替这一过程。

若 norm 为 0，应返回明确 InvalidEmbedding，不得生成 NaN 排序。

## 19.6 Primer folds

将 top-k reports 近似等分为：

```text
primer_folds
```

每个 fold 并行调用 primer。

structured response：

```json
{
  "intermediate_answer": "...",
  "score": 0,
  "follow_up_queries": ["...", "..."]
}
```

语义：

* intermediate answer 必须存在；
* follow-up queries 必须存在；
* 多 fold answer 用两个换行连接；
* follow-ups 展平；
* score 取 fold 平均值。

无 intermediate answer 或无 follow-up 必须报错。

## 19.7 Action Graph

内部实现：

```rust
struct DriftAction {
    query: String,
    answer: Option<String>,
    score: Option<f64>,
    follow_ups: Vec<DriftActionId>,
    metadata: DriftActionMetadata,
}

struct DriftQueryState {
    nodes: IndexMap<DriftActionId, DriftAction>,
    edges: Vec<DriftEdge>,
}
```

不要求引入新的 graph crate。

action identity 以 query 文本为语义键。

需要支持：

* add action；
* relate parent/child；
* incomplete actions；
* rank/shuffle incomplete actions；
* usage 汇总；
* context serialization；
* nodes/edges 序列化。

没有 scorer 时，上游会随机打乱 incomplete actions。

production 保留随机选择，测试使用注入 RNG。

## 19.8 DRIFT Local Search

每个 incomplete follow-up 使用 Local Search，参数来自 `drift_search.local_search_*`。

Local response 必须是 JSON：

```json
{
  "response": "...",
  "score": 80,
  "follow_up_queries": ["..."]
}
```

解析后更新 action：

```text
answer
score
follow-ups
context_data
usage
```

无 answer 时 warning。

无法解析时 score 使用负无穷语义，使其处于最低优先级。

## 19.9 Depth loop

最多执行：

```text
n_depth
```

每轮：

1. 获取 incomplete actions；
2. 无 scorer 时打乱；
3. 取前 `drift_k_followups`；
4. bounded concurrency 执行 Local Search；
5. 写回 action；
6. 增加 follow-up nodes 和 edges。

并发上限应使用：

```text
drift_search.concurrency
```

不得无界 spawn。

## 19.10 DRIFT reduce

Query state 可序列化为：

```json
{
  "nodes": [
    {
      "id": 0,
      "query": "...",
      "answer": "...",
      "score": 80,
      "metadata": {}
    }
  ],
  "edges": [
    {
      "source": 0,
      "target": 1,
      "weight": 1.0
    }
  ]
}
```

reduce 只收集有 answer 的 node answer。

Prompt variables：

```text
context_data
response_type
```

non-streaming 调 `complete`。

streaming 只让最终 reduce 流式；primer 和 follow-up action 先完成，再开始输出最终 token。

callback：

```text
on_reduce_response_start
on_llm_new_token*
on_reduce_response_end
```

---

# 20. Rust API

## 20.1 统一 API

```rust
pub async fn query(
    config: GraphRagConfig,
    options: QueryOptions,
) -> Result<QueryResult>;

pub fn query_stream(
    config: GraphRagConfig,
    options: QueryOptions,
) -> QueryEventStream;
```

建议：

```rust
pub struct QueryOptions {
    pub project_root: PathBuf,
    pub query: String,
    pub method: SearchMethod,
    pub data_dir: Option<PathBuf>,
    pub community_level: i64,
    pub dynamic_community_selection: bool,
    pub response_type: String,
    pub conversation_history: Option<ConversationHistory>,
    pub callbacks: Vec<Arc<dyn QueryCallbacks>>,
}
```

`streaming` 不放进核心 options；由选择 `query` 或 `query_stream` 表达。

## 20.2 Method-specific API

为对应 GraphRAG public API，公开：

```rust
global_search
global_search_streaming
local_search
local_search_streaming
drift_search
drift_search_streaming
basic_search
basic_search_streaming
```

可以接收已加载 typed data，使 library 用户不必经过 CLI/project loader。

统一 API 应复用 method-specific API，而不是复制算法。

## 20.3 Stream event

只返回 `String` chunk 无法同时可靠暴露 context/usage/error lifecycle。

建议：

```rust
pub enum QueryEvent {
    Context(QueryContext),
    Token(String),
    Completed(QuerySummary),
}
```

CLI 只打印 `Token`。

若为了 API 简洁另提供 `Stream<Item = Result<String>>`，内部仍应有 event stream，并由 callback/context holder 保留完整结果。

## 20.4 非流式与流式共享

每个 method 只能有一套 context 和 orchestration 逻辑。

允许：

* 非流式直接 complete；
* 非流式消费 stream 聚合；

但 map、context、parse、reduce 数据路径必须共享。

不得维护两套容易漂移的搜索实现。

---

# 21. 错误、日志和安全

## 21.1 Typed Error

Query error 至少区分：

```text
InvalidQueryConfig
MissingQueryTable
InvalidQueryTable
MissingVectorIndex
InvalidVectorIndex
QueryPrompt
QueryEmbedding
QueryCompletion
QueryParse
QueryContext
QueryRuntime
QueryMethod
```

错误应包含 method 和 operation。

## 21.2 上游 fallback 与 Rust 错误边界

应复现有明确业务含义的 fallback：

* Global map JSON 不可解析 → score 0 batch；
* Global 无正分 point → NO_DATA_ANSWER；
* DRIFT HyDE 空文本 → original query；
* Dynamic child 无 report → 跳过并记录；
* Local 找不到某个 ANN entity id → warning 并跳过。

不得复现 Python 的无差别 `except Exception` 来吞掉：

* 文件损坏；
* provider 鉴权失败；
* timeout；
* LanceDB schema 错误；
* Prompt 编译错误；
* 配置错误。

这些错误必须向调用者传播。

## 21.3 Secret redaction

继续沿用 Phase 1。

任何 Query log/error/debug 不得输出：

```text
api_key
Authorization
token
password
secret
```

允许 debug 输出 Prompt 的截断摘要，但默认日志不得完整记录用户私密 context。

## 21.4 query.log

Query 使用独立日志文件：

```text
logs/query.log
```

不得写入 index.log。

## 21.5 并发

所有并发必须有上限：

* Global map：`config.concurrent_requests`；
* Dynamic rating：`config.concurrent_requests`；
* DRIFT primer/action：`drift_search.concurrency`；
* provider client：现有 model concurrency layer。

不得在两层 semaphore 中造成永久占用或死锁。

---

# 22. 实现步骤

每个 Step 必须独立可测试、CI 通过后再进入下一步。

## Step 1：规格与骨架

* `spec/README.md` → `spec/phase1.md`；
* 新增 `spec/phase2.md`；
* 增加 `query` 内部模块；
* 增加 CLI enum/args，但可暂时返回明确 not implemented；
* 不修改 Phase 1 index 行为。

验收：

```text
cargo fmt
cargo clippy
cargo test
```

全部通过。

## Step 2：Typed Query Config 与 init assets

* 四个 typed config；
* GraphRagConfig 正式字段；
* defaults/validation；
* 8 个 Query PromptKind；
* settings query prompt paths；
* init 生成 13 个 Prompt；
* Prompt compile tests；
* snake_case round-trip tests。

## Step 3：Completion streaming

* chunk types；
* trait；
* OpenAI streaming；
* mock streaming；
* callback token test；
* provider error propagation；
* CLI flush utility test。

此 Step 不实现 Search。

## Step 4：Vector similarity search

* trait；
* LanceDB ANN；
* score conversion；
* dimension/top-k validation；
* include_vectors；
* contract tests。

## Step 5：Query data model、adapter 与 loader

* 六种 typed model；
* method dependency matrix；
* DataFrame adapters；
* community roll-up；
* optional covariates；
* GraphRAG fixture read tests；
* GraphLoom Phase 1 output read tests。

## Step 6：Basic Search

* Basic context；
* original table order；
* token cutoff；
* prompt；
* stream/non-stream；
* context callback；
* GraphRAG golden request/context test。

## Step 7：Local Search

* query-to-entity；
* mixed context；
* relationships/covariates；
* text units；
* conversation history；
* stream/non-stream；
* golden tests。

## Step 8：Global Search（固定选择）

* community batching；
* deterministic shuffle；
* parallel map；
* structured parse；
* reduce；
* no-data answer；
* callbacks；
* golden tests。

## Step 9：Dynamic Community Selection

* hierarchy adapter；
* rating；
* traversal；
* repeats；
* fallback levels；
* deterministic mock tests；
* 接入 Global。

## Step 10：DRIFT

* report embedding hydration；
* HyDE；
* cosine ranking；
* folds；
* structured primer；
* action graph；
* recursive Local；
* reduce；
* stream；
* deterministic RNG tests。

## Step 11：统一 API 与 CLI

* `api::query`；
* method-specific APIs；
* QueryEvent；
* CLI dispatch；
* stdout/stderr；
* query.log；
* help snapshots；
* exit-code tests。

## Step 12：兼容性与端到端

* GraphRAG index → GraphLoom query；
* GraphLoom index → GraphRAG query；
* OpenAI-compatible local stub；
* streaming stub；
* deterministic request recorder；
* Windows/Linux/macOS CI；
* README 更新。

---

# 23. 测试规范

## 23.1 Unit tests

至少覆盖：

* 每个 config default 和 invalid value；
* Prompt 文件、变量和原文 hash；
* DataFrame type adaptation；
* community level；
* report roll-up；
* hierarchy rebuild；
* token budget boundary；
* CSV escaping；
* entity mapping；
* relationship ranking；
* text unit dedupe/order；
* map JSON extraction；
* no-data answer；
* Dynamic threshold/parent/level；
* DRIFT folds/action state；
* vector score；
* stream chunk aggregation；
* callback order；
* missing table/index；
* secret redaction。

## 23.2 Request golden tests

使用 recording completion/embedding model 记录：

```text
method
model instance
messages
response_format
temperature
top_p
n
max_tokens
max_completion_tokens
stream
embedding input
```

对照 GraphRAG 3.1.0 同一 fixture。

重点不是只比较 final answer，而是比较 LLM 看到的请求。

## 23.3 Context golden tests

每种 method 保存：

```text
context text
context record column names
record order
batch count
token cutoff position
```

Basic/Local/Global 固定 fixture 应做到 context text 精确相等。

DRIFT 对注入固定 RNG 的 fixture 做精确相等。

## 23.4 Python 互操作

必须覆盖两个方向。

### GraphRAG 生成索引，GraphLoom 查询

```bash
uv run graphrag index --root <python-root>
cargo run -p graphloom -- query \
  --root <python-root> \
  --method basic \
  "..."
```

依次测试四种 method。

### GraphLoom 生成索引，GraphRAG 查询

```bash
cargo run -p graphloom -- index --root <rust-root>
uv run graphrag query \
  --root <rust-root> \
  --method basic \
  "..."
```

依次测试四种 method。

## 23.5 CLI matrix

至少：

```text
4 methods
× streaming true/false
× GraphRAG index / GraphLoom index
```

Dynamic Global 单独增加 true/false。

DRIFT 的真实模型测试可标记为较慢集成测试，但 mock 端到端必须进入普通 CI。

## 23.6 非确定性比较

真实模型下不比较逐字答案。

比较：

* command 成功；
* 所需表和向量被正确读取；
* 请求类型和参数；
* context citation 格式；
* 回答非空；
* streaming 产生多个 chunk；
* 无 secret；
* usage 合理。

确定性 mock 下比较完整文本。

---

# 24. 验收标准

Phase 2 只有在以下条件全部满足时才完成。

## 24.1 CLI

* `graphloom query --help` 与 GraphRAG 3.1.0 参数一致；
* 默认 method 是 global；
* 四种 method 均可运行；
* streaming 真正逐 chunk 输出；
* `--data` 正确覆盖 table directory；
* 缺表/缺 index 明确失败。

## 24.2 Config 与 init

* 四个 typed config；
* defaults 一致；
* init settings 含 Prompt path；
* init 写出 13 个 Prompt；
* 所有 Prompt 可编译；
* Query config 不再落入 dynamic `sections`。

## 24.3 数据

* 可读取 GraphRAG 3.1.0 Parquet；
* 可读取 GraphLoom Phase 1 Parquet；
* GraphRAG 可继续读取 GraphLoom Phase 1 输出；
* Query 不修改任何 index 文件。

## 24.4 模型和向量

* CompletionModel 支持真实 stream；
* LanceDB 支持 ANN；
* score 与 GraphRAG 一致；
* method 只连接所需资源；
* Query 不写 Phase 1 cache。

## 24.5 算法

* Basic、Local、Global、Dynamic Global、DRIFT 均有 golden；
* context builder 顺序和 token budget 兼容；
* Global map/reduce 兼容；
* DRIFT 不是简化实现。

## 24.6 工程质量

必须通过：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo nextest run --workspace
```

若项目 CI 有 Windows job，也必须通过。

不得：

* 新增未使用 crate；
* 引入 production MemoryVectorStore；
* 在 Query 中复制 provider client；
* 泄漏 secret；
* 用 `unwrap`/`expect` 处理运行时外部数据；
* 通过修改 Phase 1 schema 来迁就 Query；
* 依赖 process cwd；
* 让 streaming 退化为整段输出。

---

# 25. Definition of Done

满足以下最终场景：

```bash
graphloom init --root demo
# 配置 API key，并放入 input
graphloom index --root demo

graphloom query --root demo --method basic \
  "What are the main facts?"

graphloom query --root demo --method local \
  "What happened to entity X?"

graphloom query --root demo --method global \
  "What are the major themes?"

graphloom query --root demo --method global \
  --dynamic-community-selection \
  "What are the major themes?"

graphloom query --root demo --method drift \
  --streaming \
  "Explore the causes and consequences of X."
```

以上命令都能读取 Phase 1 正式索引，不重建、不修改索引，并产生与 GraphRAG 3.1.0 同语义的 context、模型请求和回答流程。

---

# 26. Phase 3 边界

Phase 2 的公共接口不得阻止 Phase 3 Standard Update Workflows。

需要保留：

* Project loader；
* output/previous output 分离能力；
* stable table adapters；
* vector store get/upsert/remove 的后续扩展空间；
* query runtime 对 active output 的只读抽象。

但 Phase 2 不实现 update，也不得为了未来 update 提前引入未使用的复杂机制。

---

# 附录 A：GraphRAG 3.1.0 参考文件

实现前至少逐一对照：

```text
graphrag/cli/main.py
graphrag/cli/query.py
graphrag/api/query.py
graphrag/query/factory.py
graphrag/query/indexer_adapters.py
graphrag/query/input/loaders/dfs.py
graphrag/query/context_builder/
graphrag/query/structured_search/base.py
graphrag/query/structured_search/basic_search/
graphrag/query/structured_search/local_search/
graphrag/query/structured_search/global_search/
graphrag/query/structured_search/drift_search/
graphrag/config/models/basic_search_config.py
graphrag/config/models/local_search_config.py
graphrag/config/models/global_search_config.py
graphrag/config/models/drift_search_config.py
graphrag/config/defaults.py
graphrag/config/init_content.py
graphrag/cli/initialize.py
graphrag/prompts/query/
graphrag-vectors/graphrag_vectors/vector_store.py
graphrag-vectors/graphrag_vectors/lancedb.py
```

不得只阅读 CLI 或 README 后凭印象实现。

---

# 附录 B：实现决策记录要求

实现过程中发现上游不一致或 bug 时，在 PR 中增加：

```text
Compatibility Decision
Upstream file/function
Observed behavior
GraphLoom behavior
Reason
Test covering the decision
```

任何有意偏离 GraphRAG 3.1.0 的行为都必须有：

* 明确说明；
* 独立测试；
* 用户可见文档；
* 不影响默认兼容模式。

默认原则仍是：

```text
先兼容，再改进。
```
