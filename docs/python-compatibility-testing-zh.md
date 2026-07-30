# Python GraphRAG 兼容测试

GraphLoom 提供可复现的跨语言门禁：

```bash
make test-compat
```

门禁构建真实 `graphloom` binary，以及仅测试使用的
`compat_vector_manifest`、`compat_table_reader` example，然后从 uv 锁定
的 `tests/compat` 项目运行发布版 `graphrag==3.1.0`。固定源码 commit 为
`7fc6607edda3d387d23e52ededbf8a75b6730f97`，v3.1.0 annotated tag object
为 `2077c4205add901e6594aced159fca81b7a6d522`。测试拒绝 editable install
及相邻源码 checkout。Session probe 校验 `graphrag==3.1.0`、
`graphrag-vectors==3.1.0`、`lancedb==0.24.3`、其 `direct_url.json`
metadata，以及所有 Query module 均来自 active uv 环境 `site-packages`。
全部兼容 subprocess 都移除 `PYTHONPATH` 并禁用 user site。

## 兼容契约

兼容性分四层：

1. **Workflow 行为：**workflow 顺序、prompt、解析、graph/community 决策
   和 Query 编排。
2. **协议互操作：**cache namespace、canonical request、key 和 response
   envelope。
3. **逻辑数据互操作：**table schema、引用、vector collection 名称、record
   ID、维度和值。
4. **物理存储互操作：**Parquet writer/Arrow 表示，以及 LanceDB 磁盘目录
   直接访问。

前三层在适用处是标准索引、增量更新、Query 和 prompt tune 的硬门禁；
第四层是独立存储加固边界。Python 环境使用 LanceDB 0.24.3、PyArrow
22.0.0；Rust workspace 使用 lancedb 0.31.0、Lance 8.0.0、Arrow 58.3.0。

互操作套件通过版本无关 manifest 和 consumer-native LanceDB materialization
验证逻辑向量记录，不宣称两个 LanceDB 版本的磁盘目录直接互开。

## 成对生产者索引

Session-scoped fixture 启动确定性本地 OpenAI-compatible HTTP server，用
相同两份已提交 UTF-8 文档分别创建 GraphLoom 与 GraphRAG 索引。Fixture
产生多个 document/text unit、entity、relationship、claim、多级 community、
report 和全部三个 vector collection。两边使用相同的四维、内容派生
embedding。

Phase 1 门禁包括：

- PyArrow/pandas 读取七张 GraphLoom 标准 Parquet；
- GraphRAG typed `DataReader` 读取这些表；
- 校验列序、逻辑类型、null、引用和 hierarchy；
- 比较 UUID-independent 语义记录；
- GraphRAG Global Search 消费 GraphLoom Parquet；
- GraphLoom 消费未修改的 GraphRAG v3.1.0 `extract_graph` cache；
- 较新固定 cache 协议 fixture 作为独立测试。

Query 不复制或转换生产者 Parquet；consumer 用 `--data` 直接指向原
`output`。

## 成对增量更新

门禁还创建新的一文档索引，加入相同第二份输入，再运行 `graphloom update`
和固定 `graphrag update`，比较：

- timestamped `previous` 和 `delta` provider；
- 七张合并最终表，使用 UUID-independent entity/relationship/community
  identity；
- completion/embedding operation 顺序和完整 prompt-derived input；
- 最终 vector row，包括重复 content-addressed community report ID；
- 保留旧 ID，以及 rebased document/text-unit/entity/relationship/
  community human-readable ID。

Opaque hierarchical Leiden cluster number 可以在语义等价 community-report
行间置换。门禁保留请求顺序与 batch 边界，比较全部 presence-aware
provider 字段；只规范化受影响 community-report embedding batch 内的输入
顺序，不把 opaque 数字标签当跨语言 identity。

另外两个 cross-producer case 只复制七张受管 Parquet：GraphRAG 标准输出
由 GraphLoom 更新，GraphLoom 标准输出由 GraphRAG 更新。每个 consumer
从空原生 vector store 开始，由最终 embedding pass 创建向量。这证明
Parquet update 互操作，而不宣称 Python/Rust LanceDB 目录兼容。

独立 no-op case clone 已索引项目，证明两边都会创建 `previous` 和空
`delta`、在 `load_update_documents` 后停止、发出零模型请求、保持最终
Parquet 及完整 vector manifest 不变。

GraphRAG 3.1.0 LanceDB provider 对每个 embedded field 以
`mode="overwrite"` 调用 `create_index()`。因此 delta pass 暂时替换每个
受管 collection，final pass 再替换；观察到的最终 LanceDB 不留下 delta
entity UUID。每次后续 flush 无条件 append，同 batch 或跨 batch 的重复 ID
都会保留。缺失来源表与未配置字段不 reset。Update-only manifest 允许重复
ID，因为最终 community-report source 可能有重复 content hash；普通 Query
互操作 manifest 仍校验 ID 唯一。

## Canonical vector manifest

`tests/compat/vector_manifest.py` 与
`crates/graphloom-vectors/examples/compat_vector_manifest.rs` 实现仅测试使用
的逻辑向量桥，不属于用户 CLI 或 production Query runtime。

稳定 JSON：

```json
{
  "format_version": 1,
  "collections": [
    {
      "name": "community_full_content",
      "dimension": 4,
      "records": [
        {
          "id": "producer-record-id",
          "vector": [1.0, 0.25, 0.5, 0.75]
        }
      ]
    }
  ]
}
```

Manifest 只含以下正式 collection，顺序固定：

```text
community_full_content
entity_description
text_unit_text
```

Record 按 ID 排序。校验拒绝未知/缺失 collection、不支持版本、空或重复 ID、
未排序记录、零或混合维度、非有限值。共享逻辑 schema 是 `id` 加完整
float32 `vector`；timestamp expansion column 是物理 store metadata，
Query 不消费。

### 生产者导出

- GraphRAG 使用固定 Python LanceDB client 读取真实生产者表；
- GraphLoom 通过公共 `VectorStore` 的 `ids`、`get_by_id` 和 ANN 方法读取
  `LanceDbVectorStore`。

两者都不从 Parquet 重建向量；导出前后 provider recorder offset 必须相同。

### 消费者导入

- GraphRAG 记录通过 `LanceDbVectorStore::ensure_index` 和
  `VectorStore::upsert_documents` 导入新 Rust-native database；
- GraphLoom 记录通过 GraphRAG `create_vector_store`、`create_index`、
  `load_documents` 导入新 Python-native database。

导入拒绝非空目标，保留全部 producer ID 和 float32 bit pattern，不过滤或
增加记录。Round-trip export 比较 collection、count、ID set、dimension 和
float32 value。每个 collection 都覆盖 by-ID 与 ANN probe；生产者记录对
自身向量 top-k score 必须为兼容值 `1.0`。Recorder offset 证明导入不发
HTTP 请求。

独立索引的 entity UUID 合法不同，所以每个 manifest 严格对照自己的
producer Parquet foreign key；跨 producer entity vector 按语义等价 title
比较。Content-addressed text-unit 与 community-report ID 可直接比较。

## Consumer view

每个 consumer 获得原生项目 view，包含自己的 `settings.yaml` 与 prompt
语法，`vector_store.db_uri` 指向 consumer-native bridge database：

```text
Parquet: 直接读取 producer output
Vectors: producer 逻辑记录，存入 consumer-native LanceDB
Prompts: consumer-native project view
```

这是逻辑向量互操作桥，不是物理 LanceDB migration；不会运行 index 或
embedding workflow。

## Query matrix 与 recorder

`tests/compat/test_query_interop.py` 运行 20 个真实 CLI 场景：

```text
2 producer/consumer directions
× 4 methods (Basic, Local, Global, DRIFT)
× streaming on/off
= 16

2 directions
× Dynamic Global
× streaming on/off
= 4
```

另有四个 Global/Dynamic Global smoke，把每个 consumer 指向不存在的
vector URI，验证不会打开或创建 LanceDB。

本地 provider 提供真实 `POST /v1/embeddings` 与
`POST /v1/chat/completions`、JSON completion、structured response、带
两个非空 delta 和 `[DONE]` 的 SSE、usage、model name、batch embedding、
有界并发和无 secret recorder。每个场景只分析自己 offset 后的请求。

断言覆盖：

- Basic：一次 query embedding、`Sources` context、最终 completion；
- Local：一次 query embedding，加 `Reports`、`Entities`、
  `Relationships`、`Sources`；
- Global：map/reduce，无 embedding；
- Dynamic Global：rating、map、reduce，无 embedding；
- DRIFT：HyDE completion、expanded-query embedding、structured primer、
  Local action、final reduce。

DRIFT 有两个互补层。普通 CLI/record-replay 使用 production randomness，
验证 candidate multiset、合法唯一选择、数量、depth、embedding input 和
请求契约，不同合法 action subset 诊断为预期非确定性。两种语言测试另读
`fixtures/query/drift_random_trajectory.json`：GraphRAG monkeypatch
positional shuffle，GraphLoom 注入 crate-private scripted random；两边
分别断言相同 selected query、state node/edge 和 Reduce-answer golden。
这不是完整固定轨迹 CLI run，也不宣称完整 Local message/context 或 Reduce
请求共享 exact golden。

`fixtures/query/query_interop_request_contract.json` 是从隔离 PyPI
`graphrag==3.1.0` 捕获并审查的请求契约，普通测试只读。它为每个 consumer、
method、Dynamic mode 和公开 streaming mode 固定完整 operation 顺序/数量、
endpoint、model、message role、embedding input 数量，以及
`response_format`、`temperature`、`top_p`、`n`、`max_tokens`、
`max_completion_tokens`、`stream` 的 presence-aware 值。契约显式记录：
GraphRAG 在部分 map/rating 省略 `response_format`/`stream`，GraphLoom
发送等价 JSON object/`stream=false`；GraphRAG 在公开非流式 DRIFT 中内部
流式并 buffer reduce，GraphLoom 发送 `stream=false`。

只有最终 provider response 进入公开 streaming 输出。每个 consumer 另有
delayed-SSE 测试：首个 delta 后暂停 server，在真实 CLI 结束前观察 delta，
再释放其余 delta 与 `[DONE]`。

## 只读证明

Query matrix 前 snapshot producer Parquet 文件集/hash/size/mtime、producer
vector 逻辑状态、两个 bridge database 和 consumer settings/prompts；结束
后必须一致。不能出现 consumer cache，不能新增、替换或 reset vector row；
允许 Query-specific log。

## Prompt-tune 兼容

同一门禁运行已提交的 typed/untyped prompt-tune Top reference fixture。
Fixture 从固定 GraphRAG 3.1.0 生成，包含：

- 精确逻辑 completion message 和 multiplicity；
- request-aware replay 的确定性响应字节；
- chunk identity、顺序、token 数、字节和 digest；
- 三个逐字节 expected prompt；
- provenance 与请求契约 manifest。

GraphLoom 必须复现 chunk、按正确 multiplicity 消费全部响应，并逐字节生成
三个文件。Fixture 也覆盖 GraphRAG 并发 relationship example 的共享可变
message-builder 行为。

Top manifest 记录两项批准的 transport 差异。Entity-type discovery 中，
GraphRAG 传入 Python response schema，GraphLoom 省略 `response_format`
并在本地校验返回 JSON；relationship example 中，GraphRAG 传递关闭的
JSON-object flag，GraphLoom 省略这个等价的关闭选项。逻辑 message、响应、
multiplicity 和生成 prompt 字节仍受门禁约束。

Top/Random/Auto 另有显式启用的真实模型 runner：记录 GraphRAG live
completion 并向 GraphLoom replay。所有模式的 chunking 都使用 embedding
model 有效 tokenizer，而非 `chunking.encoding_model`；Auto 还在两边调用
真实 embedding。

Random/Auto live case 只有一个候选，以验证真实编排和模型契约而不要求
Python/Rust 共用 RNG。多候选行为由确定性 unit/integration test 覆盖。

离线：

```bash
make test-compat
```

无网络校验真实模型前置条件：

```bash
make prompt-tune-real-llm-check
```

Live target 独立于默认门禁：

```bash
make prompt-tune-update-debug
make prompt-tune-random-real-llm
make prompt-tune-auto-real-llm
```

详见 [Prompt Tune 指南](prompt-tuning-zh.md)与
[真实模型 runner 指南](../tests/compat/PROMPT_TUNE_REAL_LLM-zh.md)。

## 运行聚焦检查

```bash
cargo build -p graphloom
cargo build -p graphloom-vectors --example compat_vector_manifest
cargo build -p graphloom-storage --example compat_table_reader
cargo test -p graphloom-vectors --example compat_vector_manifest

TARGET_DIR="$(cargo metadata --no-deps --format-version 1 | \
  python -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"

env -u PYTHONPATH \
PYTHONNOUSERSITE=1 \
GRAPHLOOM_BIN="$TARGET_DIR/debug/graphloom" \
GRAPHLOOM_VECTOR_MANIFEST_BIN="$TARGET_DIR/debug/examples/compat_vector_manifest" \
GRAPHLOOM_TABLE_READER_BIN="$TARGET_DIR/debug/examples/compat_table_reader" \
uv run --project tests/compat --locked \
pytest -vv tests/compat/test_query_interop.py
```

Query golden 与 Phase 1：

```bash
env -u PYTHONPATH \
PYTHONNOUSERSITE=1 \
GRAPHLOOM_BIN="$TARGET_DIR/debug/graphloom" \
GRAPHLOOM_VECTOR_MANIFEST_BIN="$TARGET_DIR/debug/examples/compat_vector_manifest" \
GRAPHLOOM_TABLE_READER_BIN="$TARGET_DIR/debug/examples/compat_table_reader" \
uv run --project tests/compat --locked \
pytest -vv tests/compat/test_query_compat.py

env -u PYTHONPATH \
PYTHONNOUSERSITE=1 \
GRAPHLOOM_BIN="$TARGET_DIR/debug/graphloom" \
GRAPHLOOM_VECTOR_MANIFEST_BIN="$TARGET_DIR/debug/examples/compat_vector_manifest" \
GRAPHLOOM_TABLE_READER_BIN="$TARGET_DIR/debug/examples/compat_table_reader" \
uv run --project tests/compat --locked \
pytest -vv tests/compat/test_compat.py
```

PowerShell：

```powershell
$oldPythonPath = $env:PYTHONPATH
$oldPythonNoUserSite = $env:PYTHONNOUSERSITE
$targetDir = (cargo metadata --no-deps --format-version 1 |
  ConvertFrom-Json).target_directory
Remove-Item Env:PYTHONPATH -ErrorAction SilentlyContinue
$env:PYTHONNOUSERSITE = "1"
$env:GRAPHLOOM_BIN = Join-Path $targetDir "debug\graphloom.exe"
$env:GRAPHLOOM_VECTOR_MANIFEST_BIN = Join-Path $targetDir `
  "debug\examples\compat_vector_manifest.exe"
$env:GRAPHLOOM_TABLE_READER_BIN = Join-Path $targetDir `
  "debug\examples\compat_table_reader.exe"
uv run --project tests/compat --locked pytest -q tests/compat/test_query_interop.py
if ($null -ne $oldPythonPath) {
  $env:PYTHONPATH = $oldPythonPath
}
if ($null -eq $oldPythonNoUserSite) {
  Remove-Item Env:PYTHONNOUSERSITE -ErrorAction SilentlyContinue
} else {
  $env:PYTHONNOUSERSITE = $oldPythonNoUserSite
}
```

`make test-compat` 运行全部 Python/Rust 兼容检查、Ruff 和 cache golden；
还真正执行五个 Rust manifest parser test 及离线 prompt-tune verifier，而
非只编译 example。同一显式 example test 命令在 Ubuntu、Windows、macOS
Rust CI matrix 中运行。

## 已知物理存储缺口

套件不要求任一 LanceDB 版本打开另一版本的目录，也不要求 Parquet
逐字节相同。未来可评估双方支持的 LanceDB 版本、显式离线转换工具和更多
Arrow writer conformance。任何工作都不能在 Query 中静默迁移 database，
或让 Query 写 producer artifact。
