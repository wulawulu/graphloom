# Prompt-tune 真实 LLM 兼容 runner

此仅本地 runner 将参考固定到 Microsoft GraphRAG 3.1.0 commit
`7fc6607edda3d387d23e52ededbf8a75b6730f97`，支持 Top、Random、Auto：

1. 使用配置的真实模型运行所选模式的 GraphRAG prompt tune；字节相同的
   并发请求通过 single-flight 合并为一次 provider 调用。
2. 精确记录逻辑 message role/content 和原始响应内容。
3. GraphLoom 按完整 message 字节而非 FIFO 到达顺序回放相同响应。
4. 精确比较所选请求身份和三个输出 prompt。

默认使用已提交的确定性 fixture 输入与 settings。可显式选择输入 pattern、
选择模式、chunk 和 Auto 参数。Completion/embedding 配置来自指定 settings。
GraphLoom 不执行第二次真实 completion；Auto 在两边都使用配置的真实
embedding model。

## 无网络校验

```bash
env -u PYTHONPATH PYTHONNOUSERSITE=1 \
  uv run --project tests/compat --locked \
  python tests/compat/prompt_tune_real_llm.py \
  --check \
  --settings /path/to/settings.yaml \
  --env-file /path/to/.env \
  --graphrag-source ../graphrag
```

校验会解析凭证，但只打印 provider 和 model 名，不写运行目录。

## 执行验收

先构建 GraphLoom，再显式允许网络调用：

```bash
cargo build -p graphloom
TARGET_DIR="$(cargo metadata --no-deps --format-version 1 |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
env -u PYTHONPATH PYTHONNOUSERSITE=1 \
  uv run --project tests/compat --locked \
  python tests/compat/prompt_tune_real_llm.py \
  --run \
  --settings /path/to/settings.yaml \
  --env-file /path/to/.env \
  --graphrag-source ../graphrag \
  --graphloom-bin "$TARGET_DIR/debug/graphloom" \
  --run-name typed-top
```

仓库的 live target 有意关闭 entity-type discovery，因此拒绝 GraphRAG
JSON Schema 请求的 provider 不会阻塞 selection-mode 验收：

```bash
make prompt-tune-real-llm-check
make prompt-tune-update-debug RUN_NAME=update-debug-top
make prompt-tune-random-real-llm
make prompt-tune-auto-real-llm
```

`--no-discover-entity-types` 是真实 GraphRAG CLI/API 模式：跳过 discovery
请求并生成 untyped extraction template。已提交的 typed Top fixture 单独
覆盖 discovery。GraphLoom 的普通 discovery 路径不发送 provider-specific
`response_format`，而是在客户端校验返回 JSON。若要执行 live discovery，
请直接运行脚本且不传 `--no-discover-entity-types`；此时 provider 必须接受
GraphRAG 带 schema 的参考请求。

GraphRAG 3.1.0 prompt-tune 使用配置的 embedding model tokenizer 创建
chunker，而非 `chunking.encoding_model`。GraphLoom 对 Top/Random 也遵循
同一规则，即使不调用 embedding API。对于 update-debug 的
`ollama/bge-m3`，两边都使用 LiteLLM-compatible `cl100k_base` fallback，
同时保留 fixture 的 `chunking.encoding_model: o200k_base`。

Random 与 Auto 对 `first.txt` 使用 1,000-token chunk-size 配置，从而只
产生一个 eligible candidate，避免伪装 Python pandas RNG 与 Rust RNG 可
共享实现无关 seed。多候选算法由确定性离线测试覆盖；live 验收验证每种
模式的真实编排、completion 请求和逐字节 prompt。Auto 还在两边执行真实
`ollama/bge-m3` embedding。

通用 `prompt-tune-real-llm-run` target 接受 `SETTINGS`、`ENV_FILE`、
`GRAPHRAG_SOURCE`、`SELECTION_METHOD`、`INPUT_DIR`、`INPUT_FILE_PATTERN`、
`LIMIT`、`CHUNK_SIZE`、`OVERLAP`、`ENCODING_MODEL`、`N_SUBSET_MAX`、`K`
和 `RUN_NAME`。只有明确要替换该限定 run-name 目录时才设 `CLEAN=1`。

脚本直接运行时，默认输出为仓库相对路径
`prompt_tune_real_llm/typed-top`；通用 Make target 在未设置 `RUN_NAME` 时
使用 `prompt_tune_real_llm/prompt-tune-real-llm`。已有运行不会覆盖；
`--clean`（Make target 对应 `CLEAN=1`）只删除指定 run-name 目录。

被忽略的运行目录包含真实请求/响应记录、脱敏临时项目、比较证据、日志、
生成 prompt 和 `REPORT.md`，可能含敏感模型内容，不得提交或上传。API key、
Authorization header 和原始 provider envelope 不会记录。

GraphRAG 3.1.0 在并发 relationship example 中复用可变 message builder，
因此多个逻辑调用可能有相同请求字节。Single-flight 让它们共享一个真实
响应，在不按到达顺序分配响应的前提下保持 request-aware replay。
