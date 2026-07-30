# GraphRAG 兼容性矩阵

最后审阅：2026-07-31

参考基线：Microsoft GraphRAG 3.1.0，源码提交
`7fc6607edda3d387d23e52ededbf8a75b6730f97`

本文是 GraphLoom 兼容范围的权威清单。它描述已经验证的契约，并不表示两个
项目的实现、API、依赖或持久化字节完全相同。

## GraphLoom 基线身份

机器可读合同位于
[`tests/compat/compatibility-baseline.toml`](../tests/compat/compatibility-baseline.toml)。

带注释的 Git tag `graphrag-3.1.0-compat-v1` 标识这份兼容基线对应的
GraphLoom 源码快照。此后内部实现可以继续变化，但支持范围的变化仍必须受本矩阵
和兼容门禁约束。

## 状态定义

| 状态 | 含义 |
|---|---|
| 已兼容 | 所述范围具有可复现的跨实现测试或 golden 证据。 |
| 批准差异 | 差异是有意的、已记录的，并被兼容契约接受。 |
| 未支持 | GraphLoom 会拒绝该功能，或不对其作互操作承诺。 |
| 待验证 | 实现或兼容声明仍需更广证据，或需要明确的兼容决策。 |

标记为 **CI 门禁** 的证据由 `make test-compat` 执行；**离线**证据确定、
无需网络，但可能只是较窄的 Rust/Python 测试；**显式真实模型**证据需要
外部模型配置，不属于默认 CI 门禁。

## 已兼容

| ID | 范围 | 已兼容内容 | 证据 |
|---|---|---|---|
| C-01 | 标准索引 | 固定 fixture 的 workflow 决策和七张标准逻辑 Parquet 表，包括 schema、引用、层级、请求、cache 复用和三个受管向量 collection。 | CI 门禁：`tests/compat/test_compat.py` |
| C-02 | 增量更新 | `previous`、`delta`、no-op、八个 merge workflow、最终逻辑表、请求顺序/契约、ID rebasing 和最终向量 manifest。 | CI 门禁：`tests/compat/test_compat.py` |
| C-03 | 跨生产者更新 | 任一实现都能读取另一实现的七张标准 Parquet，并创建消费者原生最终向量。 | CI 门禁：`test_cross_producer_parquet_should_support_bidirectional_native_updates` |
| C-04 | 逻辑表互操作 | PyArrow、pandas、GraphRAG typed `DataReader` 和 GraphLoom table reader 都能在逻辑 schema 层读取标准表。 | CI 门禁：`tests/compat/test_compat.py`；离线：`compat_table_reader` |
| C-05 | LLM cache 协议 | 可复用 GraphRAG 3.1.0 `extract_graph` cache，并通过单独固定的较新 `79ab7c9...` key/envelope 协议 golden。 | CI 门禁：`tests/compat/test_compat.py`；`crates/graphloom-llm/tests/cache_compat.rs` |
| C-06 | 逻辑向量记录 | Collection 名、ID、维度、float32 值、by-ID/ANN 读取，以及通过版本化 manifest 的双向导入导出。 | CI 门禁：`tests/compat/test_query_interop.py`；离线：`compat_vector_manifest` |
| C-07 | Basic Query | CLI 契约、context、provider stage、最终响应、streaming，以及两个生产/消费方向的生产者 Parquet 和逻辑向量。 | CI 门禁：`tests/compat/test_query_compat.py`、`tests/compat/test_query_interop.py` |
| C-08 | Local Query | 两个方向的 Local context 表、特殊字符、history、provider stage、向量、结果和 streaming。 | CI 门禁：同上 Query suites |
| C-09 | Global 与 Dynamic Global Query | 两个方向的静态 map/reduce、动态 rating/traversal/map/reduce、公开 streaming 开关；无需向量库。 | CI 门禁：同上 Query suites |
| C-10 | DRIFT Query | HyDE、primer、candidate/action 不变量、深度、Local action、reduce、streaming，以及共享确定性位置轨迹。 | CI 门禁：同上 Query suites；离线 trajectory golden |
| C-11 | Prompt-tune Top | Typed/untyped prompt 生成、逻辑请求及次数、chunk 身份、tokenizer 边界、响应 replay，三个输出文件逐字节一致。 | CI 门禁：`tests/compat/test_prompt_tune_top_reference.py` |
| C-12 | Prompt-tune Random | GraphRAG 3.1.0 选择语义和不变量；一个候选时完成真实模型编排验收。多候选精确样本身份见 AD-06。 | 离线 Rust 测试；显式真实模型：`make prompt-tune-random-real-llm` |
| C-13 | Prompt-tune Auto | Embedding model tokenizer、真实 embedding 调用、质心排序、GraphRAG 位置映射特殊行为、请求 replay 和生成 prompt。多候选精确样本身份见 AD-06。 | 离线 Rust 测试；显式真实模型：`make prompt-tune-auto-real-llm` |
| C-14 | OpenAI-compatible adapter | 使用 GraphRAG `openai`、`deepseek`、`ollama` provider 名配置 completion/embedding，包括 provider 默认 API base 和已验证的 `cl100k_base` fallback；更广 tokenizer mapping 见 V-09。 | Rust integration test；CI 使用确定性 OpenAI-compatible server |
| C-15 | Query 只读行为 | Query 不修改生产者 Parquet、生产者/bridge 向量、prompt、settings 或 cache；延迟 SSE 验证首个 delta 及时输出。 | CI 门禁：`tests/compat/test_query_interop.py` |

## 批准差异

| ID | 范围 | 批准的差异 | 理由与边界 |
|---|---|---|---|
| AD-01 | 生成身份 | 两次独立运行可以产生不同 UUID 和不透明 Leiden community label。 | 比较使用语义身份，同时验证全部引用和生产者内部 ordinal；不允许缺失、重复或错误链接记录。 |
| AD-02 | Parquet 字节 | Rust Arrow/Parquet 不要求与 Python/PyArrow 逐字节相同；列 metadata、物理表示和压缩可以不同。 | 逻辑 schema、值、null、multiplicity、会影响行为的顺序及引用仍是强门禁。 |
| AD-03 | Query 请求 transport | GraphLoom 可以显式发送 `stream=false` 或等价 JSON response format，而 GraphRAG 省略字段；非流式 DRIFT 可以直接发送非流式 reduce，而不是内部 stream 后 buffer。 | Presence-aware 差异固定在已审阅请求契约中；prompt 文本、模型输入、操作次数和公开行为仍须兼容。 |
| AD-04 | DRIFT 随机路径 | 生产运行可以选择不同但合法的 follow-up 子集。 | 两边必须选择规定数量的不重复 incomplete candidate、遵守深度和请求契约，并通过共享确定性状态迁移轨迹。 |
| AD-05 | Prompt 发布与路径安全 | `init` 和 prompt-tune 以事务方式发布受管文件，且 GraphLoom 更严格拒绝 symlink/reparse point 和路径重叠。 | 成功时受管文件内容保持兼容；更强的失败原子性和边界校验是有意的安全保证。 |
| AD-06 | Prompt-tune RNG 身份 | Python 与 Rust 不承诺相同 PRNG 或 shuffle，因此不要求无约束多候选 Random/Auto 选中完全相同的 chunk。 | 离线测试选择语义和不变量；真实验收使用一个候选，不能据此宣称多候选 RNG 精确一致。 |
| AD-07 | Prompt-tune structured response transport | GraphRAG 把 Python `EntityTypesResponse` 类型传给模型抽象；GraphLoom 不发送 `response_format`，而是使用 provider-neutral 请求并在客户端验证返回 JSON。GraphRAG 还传递关闭的 relationship JSON-object flag，GraphLoom 则省略该字段。 | 两项差异都已写入 Top fixture manifest；逻辑 message、response 内容、entity type、请求次数和生成 prompt 字节仍是强制不变量。 |
| AD-08 | 生产者本地 ordinal | 两个独立生产者可能为 document、text unit、covariate 分配不同 `human_readable_id`，因为 ordinal 来自生产者本地枚举。 | 跨生产者比较使用 document/text/covariate 语义，并继续严格检查 multiplicity 与引用；同一生产者 update 前后保留记录的 ordinal 必须稳定。 |

## 未支持

| ID | 范围 | 当前边界 |
|---|---|---|
| U-01 | 其他 GraphRAG release | 完整 workflow 兼容只声明到 3.1.0；较新 cache 协议 golden 是窄例外，不代表兼容较新 release。 |
| U-02 | 其他模型 provider 与身份 | 被依赖的模型仅支持 GraphRAG `openai`、`deepseek`、`ollama` provider 名；Azure、Anthropic、其他 LiteLLM provider 名及 Azure managed identity 会被拒绝。未使用的 model entry 不会扩大受支持范围。 |
| U-03 | 其他存储、向量、cache 与 reporting provider | 未实现远程 blob storage、CosmosDB、Azure AI Search、memory/blob cache 和非 file reporting；支持的基线是 file input/output/reporting、JSON 或关闭 cache，以及 LanceDB。 |
| U-04 | 其他输入格式 | 未实现 CSV、JSON、JSONL 输入；支持 UTF-8 文本文件。 |
| U-05 | Query 结果 cache | 未实现 Query 结果 cache；LLM cache 兼容是另一项已支持协议。 |
| U-06 | LanceDB 跨版本目录直读 | Python LanceDB 0.24.3 与 Rust lancedb 0.31.0 不互相直接打开目录；支持的是逻辑向量 manifest 和消费者原生 materialization。 |
| U-07 | 任意 GraphRAG 扩展 | 第三方 workflow、plugin、notebook 和私有 Python API 不在兼容契约内，除非后续在本矩阵中加入证据。 |
| U-08 | 其他模型认证与 retry strategy | 被依赖的模型支持 `api_key` 认证和 `exponential_backoff` retry；其他认证及 retry strategy 会被拒绝。 |
| U-09 | Fast/NLP 索引 | 未实现 GraphRAG 的 `fast`、`fast-update` method、`extract_graph_nlp` workflow 和 NLP extractor 配置。GraphLoom 支持 `standard` 索引方法和标准增量更新。 |

## 待验证

| ID | 范围 | 已知缺口 | 退出条件 |
|---|---|---|---|
| V-01 | Windows/macOS 完整兼容门禁 | Rust build/test 覆盖三个 CI 平台，但 Python GraphRAG 跨实现门禁只在 Ubuntu 运行。 | 在 Windows/macOS 运行隔离的固定 Python 门禁，或记录经过审阅的平台差异。 |
| V-02 | 多候选真实 Random/Auto | 真实验收有意只保留一个候选；多候选只在不变量/单元测试层确定。 | 增加可复现的注入轨迹或分布/不变量真实验收，且不要求 Python/Rust PRNG 输出相同。 |
| V-03 | 真实 provider 矩阵 | 确定性门禁验证 OpenAI-compatible 协议，但不会调用每个支持的 provider/model 组合。 | 为代表性的 OpenAI、DeepSeek、Ollama completion/embedding 配置保存可重复、已脱敏的验收记录。 |
| V-04 | 跨实现故障注入 | 已门禁成功更新和 no-op，但尚未逐一比较 table/vector/model 各部分失败边界的行为。 | 增加带明确恢复状态契约的故障注入，或批准并记录差异。 |
| V-05 | Claim extraction 模型错误 | GraphRAG 记录单文档 claim extraction 错误后继续；GraphLoom 当前使 workflow 失败。 | 决定目标契约；如果要求对齐则实现，并加入跨实现错误 fixture。 |
| V-06 | 更广 corpus/配置空间 | 强门禁有意保持小而确定，无法覆盖全部 prompt override、token budget、层级形态、重复模式或语料规模。 | 真实语料发现新兼容类别时加入最小回归 fixture，不能只依赖被忽略的 debug artifact。 |
| V-07 | 等大 LCC tie-break | 启用 `use_lcc` 时，GraphRAG 按输入顺序保留第一个等大连通分量；GraphLoom 为保持 shuffle 稳定而有意保留字典序最前的分量。当前跨实现 fixture 未覆盖该 tie。 | 增加固定的等大分量 fixture；随后在兼容模式复现 GraphRAG，或在明确下游不变量后把字典序规则升级为批准差异。 |
| V-08 | 模型 rate limit 与 metrics | GraphLoom 会解析但不执行旧式 `tokens_per_minute`、`requests_per_minute` 字段；对 GraphRAG nested `rate_limit` 与 `metrics` 设置则忽略而非应用。 | 实现并跨实现验证 middleware 语义，或显式拒绝非默认设置并把该边界移到未支持。 |
| V-09 | Model-specific tokenizer mapping | GraphLoom 优先使用显式 model `encoding_model`，否则使用 `cl100k_base`；GraphRAG LiteLLM tokenizer 可为已知 model ID 选择专属 encoding。现有跨实现 prompt-tune 证据覆盖 `text-embedding-3-small`/`cl100k_base` 与 `ollama/bge-m3` fallback，不覆盖完整 LiteLLM catalog。 | 增加可维护的 provider/model mapping 或其他兼容 resolver，再跨 indexing、Query、prompt tune 门禁代表性的非 `cl100k_base` chunk 与 token-budget 边界。 |

## 维护规则

1. 任何受支持 workflow、请求契约、schema、向量记录、provider、输入/存储
   类型或兼容测试变化，都必须在同一变更中更新本矩阵。
2. 只有具有可复现且已提交的测试或 golden 时，条目才能进入**已兼容**。
   显式真实模型证据必须始终保留该标签。
3. **批准差异**必须说明允许什么不同、哪些不变量仍然强制。测试
   normalization 不得超过该边界。
4. **未支持**功能必须显式失败；静默 fallback 不属于兼容。
5. 每个**待验证**条目必须有具体退出条件。关闭条目时应移动或删除条目，
   而不是只改措辞。
6. 如果兼容要求保留 GraphRAG 的缺陷、低效或意外行为，必须同步新增或更新
   [兼容优化清单](optimization-opportunities-zh.md)。
7. 每次依据代码和测试审计清单时，更新审阅日期和固定基线。

实现和测试细节见[兼容测试指南](python-compatibility-testing-zh.md)、
[Prompt Tune 指南](prompt-tuning-zh.md)和
[Query 录制/回放指南](query-record-replay-zh.md)。
