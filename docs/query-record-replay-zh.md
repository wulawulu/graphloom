# Query LLM 录制/回放兼容

此本地工作流比较 GraphRAG 3.1.0 与 GraphLoom，不让 LLM 随机性掩盖或
虚构兼容性。OpenAI-compatible proxy 使用规范化语义请求作为 cache key。
Miss 会调用 LiteLLM 并持久保存成功响应；hit 返回同一响应。请求匹配和
最终答案比较仍由 test runner 负责。

固定输入为：

```text
GraphRAG  -> ../graphrag/debug
GraphLoom -> ./debug
DeepSeek API key -> ./debug/.env:GRAPHRAG_API_KEY
```

用一个问题和新 case 名运行四种非流式方法：

```bash
make query-record-replay \
  CASE=jinpingmei-01 \
  QUERY='西门庆和武松之间有什么联系？'
```

通过 `METHOD=basic`、`local`、`global` 或 `drift` 只运行一种方法。输出
被 Git 忽略，并写到 `debug/query-record-replay/<CASE>/<METHOD>/`：

```text
cache.jsonl
graphrag-requests.jsonl
graphloom-requests.jsonl
graphrag.stdout / graphrag.stderr
graphloom.stdout / graphloom.stderr
report.json
```

`report.json` 报告 `requests.matchEqual`、`requests.exactEqual`、每项 raw
差异及 stdout equality。语义匹配比较 multiset，所以无害的 completion
顺序差异不会使并发运行失败；仅顺序不同仍记录为 `$.requestOrder`。
`affectsMatch` 区分语义差异与被忽略的 transport/options 字段。请求不同时
仍展示答案比较，但无法隔离本地 post-LLM 逻辑。Consumer settings 只存在
于自动删除的临时目录，并保留配置的 concurrency。Authorization header
和 API key 不写入 cache、请求 transcript、报告、stdout 或 proxy error。

`indexArtifactPresence` 列出两边 Parquet 文件名并突出只存在于一边的文件。
它是诊断前置条件，不宣称同名 Parquet 包含相同逻辑行。

请求或答案比较失败时 Make target 返回非零；这是兼容性发现，不一定是
proxy/provider 失败。使用报告中的两个进程退出码区分比较失败和执行失败。

Proxy 也可独立启动：

```bash
make llm-cache-proxy \
  CASSETTE=debug/query-record-replay/manual/cache.jsonl \
  COMPLETION_PROVIDER=deepseek \
  EMBEDDING_PROVIDER=ollama \
  EMBEDDING_API_BASE=http://localhost:11434
```

Match view 有意保持精简。Chat key 包含 endpoint、model、`messages` 和是否
真正启用 streaming；embedding key 包含 endpoint、model、`input` 与同一
stream flag。缺失 `stream` 等价于 `stream: false`。`encoding_format`、
`response_format`、`temperature`、`top_p`、`n` 保留在 raw transcript，
但不拆分 cache。Message 文本、空白、Unicode、role、顺序、embedding
input、model、endpoint 和 `stream: true` 仍有语义。

DRIFT HYDE prompt 会嵌入随机选择的 community report。Match view 只把该
限定随机模板槽替换为 marker；query 和固定 prompt 文本仍有语义。全部原始
内容保留在 JSONL，差异以 `affectsMatch: false` 报告。不同 key 可并发调用
LiteLLM；同 key 的并发 miss 会合并为一次上游调用。

DRIFT 还会在截取 `drift_k_followups` 前 shuffle 每层未完成 follow-up
action，因此两次 GraphRAG 运行也不保证执行同一子集。默认 production
random 模式下，`driftBehavior` 从观察到的 Primer 与 action 响应重建两边
action graph。它严格比较 HyDE/Primer 契约、Primer answer/score/follow-up
multiset、请求参数及对齐的 candidate set。每边都验证选择项属于 incomplete
candidate、数量为 `min(incomplete_action_count, drift_k_followups)` 且
唯一、action embedding input 匹配所选 query、遍历不超过 `n_depth`。
不同合法子集报告为 `expected nondeterminism`；candidate 不同、非法或
重复选择、数量/depth/请求契约错误仍会失败。合法随机分支分叉后，下游
Local context、state、Reduce input 和最终文本可以不同而不使默认比较失败。

共享的位置轨迹
`tests/compat/fixtures/query/drift_random_trajectory.json` 是确定性状态转换
检查，不是端到端 CLI 请求比较。GraphRAG monkeypatch `random.shuffle`，
GraphLoom 注入 crate-private `ScriptedDriftRandom`。两种语言分别断言相同
selected query、state node/edge 和 Reduce-answer golden。Rust 测试还会
拒绝耗尽或非法 report/action 轨迹，而非回退系统随机。当前没有完整 Local
message、Local context 或 Reduce 请求的共享跨语言 golden。真实 CLI
录制/回放因此继续使用默认系统随机和上述约束比较。轨迹记录位置而非 seed，
因为 Python 与 Rust 不保证相同 seed 使用相同 PRNG 或 shuffle。生产入口
继续实例化 `SystemDriftRandom`。

Provider adaptation 只在请求已生成 key 并被观察后发生。例如 LiteLLM 可能
丢弃不支持的 embedding 字段，DeepSeek `json_schema` 以上游
`json_object` 发送；cassette 和比较 transcript 仍保留原始请求。

两个 debug 目录必须表示等价索引。Runner 不隐藏由不同 artifact/settings
造成的 context 差异。例如只在一边存在的 `covariates.parquet` 会改变
Local/DRIFT token budget 和最终 `messages`；对所选测试输入而言，这仍是
语义不兼容。
