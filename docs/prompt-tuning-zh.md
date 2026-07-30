# Prompt Tune

GraphLoom 通过 `graphloom prompt-tune` CLI 和公共 Rust API 实现 Microsoft
GraphRAG 3.1.0 prompt-tune 流程。它读取项目配置的文本输入、选择 chunk、
调用配置模型并生成三个索引 prompt：

```text
extract_graph.txt
summarize_descriptions.txt
community_report_graph.txt
```

## CLI

最小调用：

```bash
graphloom prompt-tune --root ./demo
```

项目必须包含有效的 `settings.yaml`（或其他受支持格式）、匹配
`input.file_pattern` 的输入文件，以及配置好的默认 completion model。
输出默认为项目根目录下的 `prompts`。

选择模式：

| 模式 | 行为 | 主要参数 |
|---|---|---|
| `top` | 按稳定文档/chunk 顺序选取最前面的 chunk | `--limit` |
| `random` | 无放回均匀随机抽样 | `--limit` |
| `auto` | 复现 GraphRAG 3.1.0 embedding-centroid 选择及其位置映射 | `--n-subset-max`、`--k` |

CLI 默认使用 Random。常用参数：

```text
--domain <DOMAIN>
--language <LANGUAGE>
--selection-method <top|random|auto>
--limit <N>                    # 默认：15
--n-subset-max <N>             # 默认：300
--k <N>                        # 默认：15
--max-tokens <N>               # 默认：2000
--min-examples-required <N>    # 默认：2
--chunk-size <TOKENS>          # 默认：1200
--overlap <TOKENS>             # 默认：100
--[no-]discover-entity-types
--output <DIRECTORY>           # 默认：prompts
```

`--chunk-size` 和 `--overlap` 是本次命令覆盖值。实际 tokenization 方面，
GraphRAG 3.1.0 使用配置的 embedding model tokenizer 创建 prompt-tune
chunker；GraphLoom 在 Top、Random 和 Auto 中都一致，即使只有 Auto 调用
embedding API。模型没有已知 tokenizer 映射时使用 provider 的有效兼容
fallback；`chunking.encoding_model` 不决定 prompt-tune chunk 边界。

Auto 有意保留 GraphRAG 3.1.0 的特殊行为：随机抽取最多
`n_subset_max` 个 chunk，对样本 embedding，按到质心的欧氏距离排序样本
位置，然后把这些位置索引应用到原始、未抽样的 chunk 列表。它不会返回
样本行本身。该行为看似意外，但修改会破坏固定兼容基线。

未提供 `--domain` 或 `--language` 时由 completion model 推断。默认开启
entity-type discovery，并使用 GraphRAG structured JSON 请求。
`--no-discover-entity-types` 跳过该请求并采用 untyped extraction 模板，
适用于拒绝 JSON Schema response format 的 provider。

CLI 禁用 prompt-tune cache，与 GraphRAG 3.1.0 默认一致。三个文件事务式
发布：旧目标在 staged file rename 期间备份，失败则恢复。输出目录和目标
文件不能是符号链接或 reparse point。

## Rust API

`graphloom::api::generate_indexing_prompts` 返回生成字符串，不写文件：

```rust,no_run
use graphloom::api::{
    DocSelectionType, GenerateIndexingPromptsOptions, generate_indexing_prompts,
};

# async fn example() -> graphloom::Result<()> {
let options = GenerateIndexingPromptsOptions::new("./demo")
    .with_selection_method(DocSelectionType::Auto)
    .with_n_subset_max(300)
    .with_k(15);

let generated = generate_indexing_prompts(&options).await?;
assert!(!generated.extract_graph.is_empty());
assert!(!generated.summarize_descriptions.is_empty());
assert!(!generated.community_report_graph.is_empty());
# Ok(())
# }
```

API 默认值匹配 GraphRAG 3.1.0：Random、limit 15、最大 prompt 2,000
tokens、开启 entity discovery、至少两个 example、Auto subset 300、
`k=15`。chunk size/overlap 未设置时使用项目配置。

API 的 `with_cache(true)` 是 GraphLoom 扩展，默认 false。启用 cache 会
改变相对参考实现的模型调用行为，不应在建立精确兼容基线时使用。

## 兼容证据

离线门禁包含两个 Top 场景：

- typed entity discovery；
- discovered type 为空，进入 untyped 路径。

Fixture 固定 GraphRAG 3.1.0 commit
`7fc6607edda3d387d23e52ededbf8a75b6730f97`，比较完整逻辑请求字节及
multiplicity、chunk 身份、replay 响应字节，并逐字节比较三个输出：

```bash
make test-compat
```

显式启用的真实模型 runner 覆盖三种模式：

```bash
make prompt-tune-real-llm-check
make prompt-tune-update-debug
make prompt-tune-random-real-llm
make prompt-tune-auto-real-llm
```

Top 使用所选真实 completion model。Random/Auto live case 只保留一个
候选，防止 Python 与 Rust RNG 实现细节制造假不兼容；多候选不变量由
确定性离线测试覆盖。Auto 还在两端调用配置的真实 embedding model。

Runner 只让 GraphRAG 执行真实 completion，记录精确逻辑响应内容并向
GraphLoom replay，再精确比较请求身份和 prompt。配置、安全与产物规则见
[`tests/compat/PROMPT_TUNE_REAL_LLM-zh.md`](../tests/compat/PROMPT_TUNE_REAL_LLM-zh.md)。

## 兼容边界

验收证据支持本文所述 GraphRAG 3.1.0-compatible prompt-tune 编排、请求
构造、tokenizer 选择、输出组装及 Top/Random/Auto，但不宣称：

- 不同 RNG 实现在无约束多候选 live run 中选出相同随机样本；
- 不经 request-aware replay 时任意模型输出逐字节相同；
- 与固定 3.1.0 以外的 GraphRAG release 兼容；
- 启用 GraphLoom-only API cache 扩展后仍等价。
