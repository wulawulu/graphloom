# Prompt-tune Top 兼容 fixture

该 fixture 记录 Microsoft GraphRAG 3.1.0 的真实 prompt-tune Top 流程，
并将其响应回放给 GraphLoom。参考版本固定为 tag `v3.1.0`、commit
`7fc6607edda3d387d23e52ededbf8a75b6730f97`。

`typed` 与 `untyped` 使用相同的两份输入、token chunk 配置和前三个
chunk，仅 entity-types 响应不同：typed 返回 `person` 与
`organization`，untyped 返回空数组。

每个场景包含：

- `requests.json`：完整逻辑 message role/content、字节长度和 SHA-256；
- `responses.json`：带字节证据的确定性测试响应；
- `selected_chunks.json`：路径、文档/chunk 序号、token 数、Top 顺序、
  文本字节和 digest；
- `expected/`：三个逐字节精确的 GraphRAG 输出 prompt；
- `manifest.json`：来源、计数、hash 及两个显式批准的请求契约差异。

GraphRAG 3.1.0 创建 relationship coroutine 时复用一个可变
`CompletionMessagesBuilder`。`asyncio.gather` 启动后，三个调用都看到同一
份累积的 system 加三条 user message。Fixture 因此为一个精确请求身份
记录 multiplicity 3，并要求每次使用相同响应字节。Replay 校验全部出现
次数，不依赖请求到达或任务完成顺序。

## 验证

```bash
cargo build -p graphloom
TARGET_DIR="$(cargo metadata --no-deps --format-version 1 |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
env -u PYTHONPATH PYTHONNOUSERSITE=1 \
  uv run --project tests/compat --locked \
  python tests/compat/prompt_tune_top_reference.py \
  --verify \
  --graphloom-bin "$TARGET_DIR/debug/graphloom"
```

`make test-compat` 会运行此 verifier；它不需要 API key，也不访问网络。

## 更新

只能从包含固定 release commit 的本地 Git 仓库更新。脚本使用
`git archive`，不会 checkout、reset 或修改 GraphRAG 工作区。

```bash
TARGET_DIR="$(cargo metadata --no-deps --format-version 1 |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
GRAPHRAG_API_KEY=compat-test-key \
  env -u PYTHONPATH PYTHONNOUSERSITE=1 \
  uv run --project tests/compat --locked \
  python tests/compat/prompt_tune_top_reference.py \
  --update \
  --graphrag-source ../graphrag \
  --graphloom-bin "$TARGET_DIR/debug/graphloom"
```

更新会打印每个变更文件的新旧 SHA-256，并立即执行同一 GraphLoom replay
验证。生成 JSON 使用排序 key 和单个结尾 LF；prompt 字节不会 trim 或
normalize。
