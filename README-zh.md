# GraphLoom

GraphLoom 是一个 Rust 实现，兼容 Microsoft GraphRAG 的标准索引、增量更新、
Query 和 prompt-tune 行为。兼容基线为 Microsoft GraphRAG 3.1.0；对于已经
演进的缓存协议，另以固定的较新 GraphRAG 源码进行验证。

## 安装

```bash
cargo install --path crates/graphloom
```

开发环境：

```bash
cargo run -p graphloom -- --help
```

## 架构

`graphloom` crate 同时提供 Rust 库与命令行程序。

- `graphloom::api` 暴露索引、Query 和 prompt-tune API。`build_index`
  执行标准索引，`update_index` 执行 GraphRAG 3.1.0 标准增量更新；两者
  都返回结构化 workflow 输出与统计。只读 Query API 包括 `query`、
  `query_stream`、Basic、Local、Global、DRIFT 及其流式版本。
  `build_index` 始终完整校验并直接写入配置的输出；后续 workflow 失败时
  不回滚已完成写入。`generate_indexing_prompts` 返回三个生成的索引
  prompt，不写磁盘。
- `graphloom::query::QueryEngine` 面向服务、agent 和 REPL。每种方法按需
  准备并复用不可变模型、tokenizer、prompt、Parquet 适配数据和向量连接。
  每个方法/数据键在首次查询时形成快照；静态与动态 Global 因报告集合不同
  而使用独立快照。替换已加载文件后应新建 engine。每次请求的
  `project_root` 必须解析为 engine 的已有项目目录，相对数据覆盖路径从该
  根目录解析。`Arc<QueryEngine>` 支持并发请求；callback、history、
  usage、遍历和流状态均为请求局部状态。
- `graphloom::cli` 将 CLI 参数、控制台输出、日志和退出码适配到 API。
  `graphloom index` 与 `graphloom update` 加载配置并执行 CLI 校验；
  `graphloom prompt-tune` 调用公共 API，并事务式发布三个 prompt 文件。
- `graphloom init` 是仅 CLI 的项目脚手架命令，写入默认 settings、`.env`、
  `input/` 和 prompt。`--model` 与 `--embedding` 通过结构化 YAML
  序列化写入，而非字符串替换。

## 初始化项目

```bash
graphloom init --root ./demo
# 使用兼容 GraphRAG 的中文 prompt：
graphloom init --root ./demo-zh --language chinese
```

生成：

```text
demo/
├── settings.yaml
├── .env
├── input/
└── prompts/
```

默认 prompt 内嵌于二进制，基于 MIT License 的 GraphRAG 3.1.0 prompt。
默认语言为英文；`--language chinese`（也可用 `zh`/`zh-cn`）生成中文
版本。模板使用 Tera/Jinja 双花括号，例如 `{{ input_text }}`。
社区报告 prompt 为 `prompts/community_report_graph.txt` 和
`prompts/community_report_text.txt`。`init` 共生成 13 个由 GraphRAG
3.1.0 管理的索引和 Query prompt，覆盖 Basic、Local、Global、DRIFT
及问题生成。

`init` 会在创建目录或写文件前执行路径与符号链接预检。项目路径、
`input/`、`prompts/` 或受管文件目标不安全时，不留下部分脚手架。

## API Key

编辑：

```text
demo/.env
```

设置：

```dotenv
GRAPHRAG_API_KEY=<your API key>
```

请勿提交 `.env` 或 API key。

## 输入

GraphLoom 当前支持 UTF-8 文本文件：

```bash
echo "Alice works with Bob." > demo/input/document.txt
```

`input.file_pattern` 匹配统一使用 `/` 分隔的逻辑存储路径，在 Windows
上也一样。例如 `^subdir/.*\.txt$` 可匹配
`demo/input/subdir/document.txt`。

## Prompt Tune

使用项目自己的输入生成索引 prompt：

```bash
graphloom prompt-tune --root ./demo
```

默认写入 `demo/prompts/`：

```text
extract_graph.txt
summarize_descriptions.txt
community_report_graph.txt
```

`--output <directory>` 可指定其他目录；相对路径从项目根解析。GraphLoom
先暂存三个文件并校验所有目标，再作为一个事务替换旧文件；发布失败会恢复
原文件。符号链接或 reparse-point 目标会被拒绝。

默认选择方法是 `random`：

```bash
# 按文档顺序取前 N 个 chunk
graphloom prompt-tune --root ./demo --selection-method top --limit 15

# 均匀随机抽取 N 个 chunk
graphloom prompt-tune --root ./demo --selection-method random --limit 15

# 复现 GraphRAG 的 embedding-centroid Auto 选择
graphloom prompt-tune --root ./demo --selection-method auto \
  --n-subset-max 300 --k 15
```

`--chunk-size` 与 `--overlap` 默认分别为 1200 和 100，只覆盖本次命令的
项目分块配置。`--domain` 和 `--language` 可跳过相应检测调用。默认发现
实体类型；completion provider 不接受 GraphRAG JSON Schema 请求时，可用
`--no-discover-entity-types`。

Prompt tune 使用默认 completion model；Auto 还使用 embedding model。
三种模式都使用 embedding model 的有效 tokenizer（包括兼容回退）确定
chunk 边界，而不使用 `chunking.encoding_model`，与 GraphRAG 3.1.0
一致。CLI 默认不使用 LLM cache。Auto 还保留 GraphRAG 3.1.0 的位置映射
特性：按随机样本 embedding 到质心的距离排序后，把排序位置应用到原始
chunk 列表，而不是返回样本行。

Rust API：

```rust,no_run
use graphloom::api::{
    DocSelectionType, GenerateIndexingPromptsOptions, generate_indexing_prompts,
};

# async fn example() -> graphloom::Result<()> {
let options = GenerateIndexingPromptsOptions::new("./demo")
    .with_selection_method(DocSelectionType::Top)
    .with_limit(15);
let prompts = generate_indexing_prompts(&options).await?;
assert!(!prompts.extract_graph.is_empty());
# Ok(())
# }
```

API 只返回字符串，不发布文件；它另提供显式 cache opt-in 作为 GraphLoom
扩展，启用后可能偏离参考默认行为。完整契约见
[Prompt Tune 指南](docs/prompt-tuning-zh.md)。

## 索引

```bash
graphloom index --root ./demo
```

完整标准 workflow：

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

## 更新

```bash
graphloom update --root ./demo
```

`--method standard` 是默认且唯一支持的索引方法。命令先把当前输出表复制到
带时间戳的 `previous`，只把 title 不在旧 `documents` 表中的输入索引到
`delta`，再执行 GraphRAG 3.1.0 的八个合并 workflow：

```text
demo/update_output/
└── 20260724-153000/
    ├── previous/
    └── delta/
```

时间戳格式为 `%Y%m%d-%H%M%S`。修改更新根目录：

```yaml
update_output_storage:
  type: file
  base_dir: update_output
```

增量检测只比较 document `title`：同 title 文本变化会被忽略，删除输入也
不会删除索引记录。无变化更新仍创建时间戳目录并复制 `previous`；
`load_update_documents` 报告零新文档后停止，不执行模型调用，最终
Parquet/向量不变。

更新有意保持 GraphRAG 3.1.0 的非事务语义。Delta embedding 在表合并
完成前直接写最终向量库。GraphRAG LanceDB 的 `create_index()` 对每个
存在来源表的已配置字段使用 overwrite；之后每次 flush 无条件 append，
所以重复 ID 会保留。最终 embedding 用合并表再次覆盖 delta collection。
未配置字段和缺失来源表的字段不 reset。后续失败时，`previous`、`delta`、
已完成的表以及最后完成 embedding 字段的向量状态都会保留用于诊断。

Rust API：

```rust,no_run
use graphloom::{
    GraphRagConfig,
    api::{CacheMode, IndexingMethod, UpdateIndexOptions, update_index},
};

# async fn example(config: GraphRagConfig) -> graphloom::Result<()> {
let result = update_index(
    config,
    UpdateIndexOptions {
        project_root: "./demo".into(),
        method: IndexingMethod::Standard,
        cache_mode: CacheMode::Configured,
        callbacks: Vec::new(),
    },
)
.await?;
println!("new documents: {}", result.stats.update_document_count);
# Ok(())
# }
```

## Dry Run

```bash
graphloom index --root ./demo --dry-run
```

Dry run 执行与真实索引相同的非破坏性前置校验，包括所需模型连通性和存储
路径可写性，然后输出脱敏配置摘要及 workflow 顺序。它会向 active
workflow 所需的 completion/embedding model 各发送一次短小、不缓存的
请求，可能消耗少量 token。之后在创建运行资源前退出：不执行 workflow，
不创建索引输出或日志，不写模型响应 cache，也不创建、连接或修改
LanceDB。未使用模型不会被访问。它验证非破坏性前置条件，但不保证后续
每个 provider 构造或 workflow 操作一定成功。

## Query

支持 GraphRAG 3.1.0 的全部公开 Query 方法：

```bash
graphloom query --root ./demo --method basic "What are the main facts?"
graphloom query --root ./demo --method local "What happened to Alice?"
graphloom query --root ./demo --method global "What are the major themes?"
graphloom query --root ./demo --method global \
  --dynamic-community-selection "What are the major themes?"
graphloom query --root ./demo --method drift \
  --streaming "Explore the causes and consequences."
```

默认方法为 `global`。`--streaming` 在 provider delta 到达时刷新最终答案，
并输出一个结尾换行。中间 context、Global map/rating、DRIFT action 和
usage 不进入 stdout。未加 `--verbose` 时，成功 Query 的 stderr 为空，
stdout 只有答案；生命周期诊断追加到 `logs/query.log`。

`--data <directory>` 只覆盖生产者 Parquet 目录，从进程工作目录解析；
LanceDB 仍使用项目 settings 的 `vector_store.db_uri`。Query 严格只读：
不写 Parquet、不修改向量、不创建 Query cache、不运行索引 workflow。

重复查询 API：

```rust,no_run
use std::{path::PathBuf, sync::Arc};

use graphloom::{
    GraphRagConfig,
    query::{QueryEngine, QueryOptions, SearchMethod},
};

# async fn example(config: GraphRagConfig) -> graphloom::Result<()> {
let root = PathBuf::from("./demo");
let engine = Arc::new(QueryEngine::load(config, &root).await?);
let options = QueryOptions::new(root, "What are the main facts?".into(), SearchMethod::Basic);
let result = engine.query(options).await?;
assert!(!result.response.is_empty());
# Ok(())
# }
```

```text
QueryEngine snapshot
├── Basic resources ─┐
├── Local resources ─┼── 按需、不可变、跨请求共享
├── Global resources ┤
└── DRIFT resources ─┘
          │
          ├── request A: query + callbacks + history + usage + stream state
          └── request B: query + callbacks + history + usage + stream state
```

Callback 在异步 token 路径上同步执行，不能阻塞。常见适配方式是用
`tokio::sync::mpsc::Sender::try_send` 把 owned event 交给专用 worker。

## 跳过可选校验

```bash
graphloom index --root ./demo --skip-validation
```

`--skip-validation` 是仅 CLI 的外部资源与可选预检逃生口，跳过可能依赖
环境的模型配置/连通性、prompt、输入存在性和 tokenizer 检查，也跳过
存储可写探测及可选向量校验。它不会跳过配置解析、workflow 名称、路径
安全或破坏性输出安全。与 `--dry-run` 一起可在输入和凭证就绪前打印计划。
公共 `graphloom::api::build_index` 始终完整校验。未来应用若需要 skip
模式，应使用独立受控 API，而不是削弱公共默认行为。

## 禁用 Cache

```bash
graphloom index --root ./demo --no-cache
```

`--no-cache` 只禁用本次运行，不删除已有 cache。

## 强制初始化

```bash
graphloom init --root ./demo --force
```

Force init 覆盖 `settings.yaml`、`.env` 及同名 GraphLoom 受管默认 prompt；
不会删除 `input/`、用户输入、根目录未知文件或额外 prompt。所有受管文件
先完整暂存；发布失败会恢复旧文件并删除不完整脚手架。

## 输出

成功索引写入：

```text
demo/output/documents.parquet
demo/output/text_units.parquet
demo/output/entities.parquet
demo/output/relationships.parquet
demo/output/covariates.parquet
demo/output/communities.parquet
demo/output/community_reports.parquet
demo/output/lancedb/
demo/cache/
demo/logs/indexing-engine.log
demo/logs/query.log
```

只有启用 claim extraction 才写 `covariates.parquet`。只有 active workflow
需要时才准备 LanceDB；只有 cache 启用时才准备 cache；日志是 CLI 产物，
不是库 API 输出。仅 Query CLI 运行时创建 `query.log`。

`graphloom index` 直接写配置输出，与 GraphRAG 正常生命周期一致。每个
workflow 在写入时替换自己拥有的表，不清空整个 output，也没有隔离
generation 或最终 publish。未触及的文件和表保留。后续失败时已完成输出
不会回滚，可能形成部分结果；cache 保留。

`generate_text_embeddings` active 时，GraphLoom 在 pipeline 开始前 reset
其受管 LanceDB 表；其他表和数据库目录中的无关文件不删除。

统一索引校验覆盖所需 provider 配置与连通性、active 向量 schema，以及
output、logs、启用的 cache 和 active 向量库普通写权限。校验后才构造
storage、cache、table、model 和 vector provider。向量路径通过已有祖先
解析，不能借助符号链接或 reparse point 逃逸项目布局。

Output 与向量库是受管写路径，拒绝任何符号链接或 reparse-point 组件。
Input、cache 和 logs 可为符号链接，但 overlap 检查使用其真实路径。
Output 必须与 input、cache、reporting 分离，不能互相包含。Windows 使用
不区分大小写的路径语义（包括未解析后缀），Unix 区分大小写；向量库位于
output 内的判断使用同一平台规则。

`update_output_storage` 也受符号链接、reparse point 和文件系统根保护，
并必须与 input、最终 output、cache、logs、向量库分离；更新与最终输出
不能互相包含。

Home 安全检查按 `HOME`、`USERPROFILE`、`HOMEDRIVE`+`HOMEPATH` 顺序解析。
输出和向量库可以位于 home 中的普通项目下，但不能等于 home 或成为其祖先。

最终 `text_units` Parquet 使用 GraphRAG 3.1.0 canonical schema，其中
`document_id: String`；`documents.text_unit_ids` 仍为 `List(String)`。

## GraphRAG 兼容状态

GraphLoom 首先追求行为兼容：等价输入、配置、prompt 和模型响应应做出相同
workflow 决策并产生逻辑等价数据，之后才引入 GraphLoom 优化。

自动化 `make test-compat` 让 GraphLoom 与 uv 锁定的 PyPI GraphRAG 3.1.0
共享一个确定性 OpenAI-compatible HTTP server。它覆盖标准索引和标准更新，
包括 `previous`、`delta`、最终表、完整 provider 请求契约和 canonical
向量 manifest。双向更新只复制七张生产者 Parquet，再由消费者创建原生
向量。门禁通过 PyArrow、pandas 和 GraphRAG typed `DataReader` 检查七张
表，比较 UUID 无关语义与引用，并验证 GraphLoom 复用 GraphRAG
`extract_graph` cache。较新 `79ab7c9...` cache 协议另有 golden 门禁。

Prompt tune 的确定性 fixture 覆盖 typed/untyped Top：记录 GraphRAG 3.1.0
完整逻辑请求，向 GraphLoom replay 相同响应，比较 chunk 身份，并要求三个
输出逐字节相同。显式启用的真实模型 runner 覆盖 Top、Random、Auto。
Random/Auto live case 只提供一个候选以隔离 RNG 实现差异，多候选算法由离线
测试覆盖；Auto 在两边都调用真实 embedding provider。见
[Prompt Tune 指南](docs/prompt-tuning-zh.md)和
[真实模型验收指南](tests/compat/PROMPT_TUNE_REAL_LLM-zh.md)。

同一门禁运行 20 个跨实现 Query CLI 场景：两种生产者方向、Basic/Local/
Global/DRIFT 流式与非流式，再加 Dynamic Global 两种模式。生产者 Parquet
直接读取。Basic/Local/DRIFT 通过版本化逻辑向量 manifest 把原始 `id` 与
float32 `vector` 写入消费者原生 LanceDB，不重新 embedding。测试覆盖
collection、ID、维度、向量、by-ID/ANN、provider stage、context、延迟
stream flush 和只读快照。请求契约固定 operation 数量及 presence-aware
模型参数，不保存 prompt 或凭证。

这不表示持久产物逐字节互换。Rust Parquet writer/Arrow 表示与 Python
不同，但标准表可在逻辑 schema 层交叉读取。逻辑向量桥不承诺 Python
LanceDB 0.24.3 与 Rust lancedb 0.31.0 的磁盘目录直接互开。详见
[兼容测试指南](docs/python-compatibility-testing-zh.md)。

`extract_graph` 当前复现 GraphRAG 的两阶段异常行为：先按
`(title,type)` 聚合，summary 丢失 `type`，title-only left join 可能产生
多对多笛卡尔积，之后 `finalize_graph` 保留首个 title。更严格的一对一
关联仅作为未来优化，见
[输出语义研究](docs/research/study-graphrag-extract-graph-output-zh.md)和
[优化清单](docs/optimization-opportunities-zh.md)。

## 当前支持

已支持：

- 标准索引；
- CLI 与 Rust API 的 GraphRAG 3.1.0 标准增量更新；
- UTF-8 文本输入、文件存储、JSON 文件 cache；
- 使用 GraphRAG `openai`、`deepseek`、`ollama` provider 名称配置的
  OpenAI-compatible completion/embedding；
- LanceDB 向量存储；
- Rust API/CLI 的 Basic、Local、Global、Dynamic Global、DRIFT；
- provider-native streaming；
- Rust API/CLI 的 GraphRAG 3.1.0-compatible prompt tune，包括 Top、
  Random 和 GraphRAG-compatible Auto；
- Linux、Windows、macOS Rust CI；
- 跨平台构建和兼容门禁通过后，由 Ubuntu release job 单次发布 tag。

尚未支持：

- Azure OpenAI 或 Azure managed identity；
- 远程 blob storage、CosmosDB、Azure AI Search；
- Query 结果 cache；
- 跨版本 LanceDB 磁盘互操作；
- CSV、JSON 或 JSONL 输入。

Settings、prompt、workflow、cache 协议、逻辑 Parquet schema 和向量记录
schema 均以 GraphRAG 兼容为目标。当前自动化与手工互操作构成行为基线，
LanceDB 磁盘互操作仍是后续加固项。

## License

本项目采用 MIT License。
