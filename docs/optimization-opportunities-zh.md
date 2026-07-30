# GraphRAG 3.1.0 兼容优化机会

最后审阅：2026-07-30

以下每一项都是 GraphLoom 为 GraphRAG 3.1.0 兼容基线而有意保留的行为。
它们是兼容性债务，不是意外遗留的 TODO：如果没有显式模式和新证据，直接
改变默认路径会破坏当前契约。

相关功能的权威状态由
[GraphRAG 兼容性矩阵](compatibility-matrix-zh.md)维护。

## 优先级与维护策略

| 优先级 | 含义 |
|---|---|
| P0 | 发布、恢复或 stale data 风险，可能使索引产生误导或内部不一致。 |
| P1 | 正确性、结果质量或显著模型/向量成本问题。 |
| P2 | 影响范围较窄的效率、易用性或可维护性问题。 |

在具备迁移方案和独立测试前，优化行为只能通过名称明确的非兼容模式替代兼容
行为。变更必须保持默认兼容 fixture，记录产物差异，并在同一变更中更新本
清单和兼容性矩阵。

## 清单摘要

| ID | 范围 | 保留的兼容性债务 | 优先级 | 优化模式 |
|---|---|---|---|---|
| O-01 | Extract graph | Entity summary 只按 title 连接。 | P1 | 未实现 |
| O-02 | Update | 抽取与更新使用不同 entity 身份。 | P1 | 未实现 |
| O-03 | Update | 保留 entity degree，不重新计算。 | P1 | 未实现 |
| O-04 | Update | 只按 title 检测文档变化。 | P1 | 未实现 |
| O-05 | Update | 已删除输入仍留在表和向量中。 | P0 | 未实现 |
| O-06 | Update | No-op 更新仍复制 `previous`。 | P2 | 未实现 |
| O-07 | Update | 复制每张 provider 表。 | P2 | 未实现 |
| O-08 | Update | Delta 与 final 记录分别 embedding。 | P1 | 未实现 |
| O-09 | Vectors | 每次 embedding pass 覆盖受管 collection。 | P1 | 未实现 |
| O-10 | Update | Final 表完成前 delta 向量已经可见。 | P0 | 未实现 |
| O-11 | Update | Community `children` ID 不 remap。 | P1 | 未实现 |
| O-12 | Update | Community report title 保留旧编号。 | P2 | 未实现 |
| O-13 | Update | 顺序 merge 失败留下部分最终输出。 | P0 | 未实现 |
| O-14 | Prompt tune | Auto 排序样本，却选择原始行位置。 | P1 | 未实现 |
| O-15 | Prompt tune | 过大 Random limit 回退到 15 后仍可能失败。 | P2 | 未实现 |
| O-16 | Prompt tune | 并发 example 复用一个累积 message list。 | P1 | 未实现 |
| O-17 | Claims | 请求 gleaning response 却不解析。 | P1 | 未实现 |
| O-18 | Community reports | Claim 未进入 graph context。 | P1 | 未实现 |
| O-19 | Query | 静态 community roll-up 按 title 合并 entity。 | P1 | 未实现 |
| O-20 | Index publication | 标准索引直接写 active output。 | P0 | 未实现 |
| O-21 | Community reports | 标准执行不会进入子报告 context 替换分支。 | P1 | 未实现 |

## 1. 实体摘要仅按 title 连接

**GraphRAG 行为：**实体抽取按 `(title,type)` 分组，但摘要只按 `title`
连接回去；同 title 多 type 时形成笛卡尔积。

**问题：**描述可能关联错误 type，行数乘法增长。

**GraphLoom 兼容行为：**精确复现 title-only many-to-many join 和行序。

**未来优化：**在连接中保留 typed summary identity。

**影响：**`extract_graph`、图摘要、`finalize_graph`。

兼容基线：已实现

未来优化：未实现

## 2. 更新实体身份与抽取身份不同

**GraphRAG 行为：**更新按 `title` 合并实体，抽取最初按 `(title,type)`。

**问题：**更新时不同 type 可能合并。

**GraphLoom 兼容行为：**按排序 title 分组并保留首行身份字段。

**未来优化：**增加显式、独立测试的身份策略。

**影响：**实体 merge、实体 ID 映射、text-unit remap。

兼容基线：已实现

未来优化：未实现

## 3. 不重新计算 entity degree

**GraphRAG 行为：**合并实体保留第一个 `degree`。

**问题：**degree 可能与合并后的关系图不一致。

**GraphLoom 兼容行为：**保留第一个 degree。

**未来优化：**从最终 relationships 重新计算。

**影响：**entity merge、Local Search 排序。

兼容基线：已实现

未来优化：未实现

## 4. Document delta 只使用 title

**GraphRAG 行为：**已有 title 即使 text/metadata 改变也被忽略。

**问题：**编辑后的文档无法增量刷新。

**GraphLoom 兼容行为：**只以 title membership 判断新输入。

**未来优化：**增加显式 content-aware update 算法。

**影响：**`load_update_documents`、documents、cache/model 工作。

兼容基线：已实现

未来优化：未实现

## 5. 不应用已删除输入

**GraphRAG 行为：**可以计算删除的 title，但更新 pipeline 不删除记录。

**问题：**已删除来源仍可被查询。

**GraphLoom 兼容行为：**不删除 document、graph 或 vector。

**未来优化：**增加引用感知删除和向量清理。

**影响：**所有最终表和受管向量。

兼容基线：已实现

未来优化：未实现

## 6. No-op update 仍复制 previous

**GraphRAG 行为：**检测新 title 前先创建时间戳 namespace 并完整复制
`previous`。

**问题：**无变化也消耗存储与 I/O。

**GraphLoom 兼容行为：**先复制，再在 `load_update_documents` 后停止。

**未来优化：**在显式非兼容模式下备份前检测 no-op。

**影响：**update runtime 准备和 table provider。

兼容基线：已实现

未来优化：未实现

## 7. 复制每张 provider 表

**GraphRAG 行为：**启动更新时列出并复制所有正式输出表 provider entry。

**问题：**备份成本随整个索引而非 delta 增长。

**GraphLoom 兼容行为：**每张已列出表都读写到 `previous`；不复制 cache、
logs 或 LanceDB。

**未来优化：**使用 snapshot、reflink 或表版本引用。

**影响：**更新存储与 runtime 准备。

兼容基线：已实现

未来优化：未实现

## 8. Embedding 生成两次

**GraphRAG 行为：**Delta 标准索引先 embed delta，随后
`update_text_embeddings` 再 embed 最终表。

**问题：**新记录产生重复模型和向量工作。

**GraphLoom 兼容行为：**两次都按配置的 cache、batch、snapshot 和
callback 执行。

**未来优化：**复用 delta vector 或只 embed 变化的最终行。

**影响：**embedding model、cache、snapshot、vector store。

兼容基线：已实现

未来优化：未实现

## 9. Embedding pass 覆盖受管 collection

**GraphRAG 行为：**`embed_text` 调用 LanceDB `create_index()`，在加载前以
`mode="overwrite"` 创建表。Delta 与 final pass 都替换受管 collection；
后续 flush 无条件 append，所以同 batch 或跨 batch 的重复 ID 都保留。

**问题：**Delta pass 修改最终 store 后又被丢弃；重复 content-addressed
report ID 使 collection 不唯一。

**GraphLoom 兼容行为：**每次 embedding pass 对存在来源表的已配置
collection reset 一次，之后每次 flush append。完整 update manifest 保留
重复行。未配置 collection、缺失来源表的字段和未知第三方 collection 不动。

**未来优化：**使用 keyed incremental upsert，显式清除 stale ID 并保证唯一。

**影响：**所有受管向量、vector manifest、存储写入。

兼容基线：已实现

未来优化：未实现

## 10. 更新提前修改最终向量

**GraphRAG 行为：**表 merge 完成前，delta embedding 已覆盖已配置的最终
受管 collection。

**问题：**后续失败可让 vector state 超前于最终 Parquet。

**GraphLoom 兼容行为：**Delta replacement 立即可见，直到 final embedding
再次替换。

**未来优化：**让 vector 与 table 原子 staged publication。

**影响：**update runtime、embedding workflow、失败恢复。

兼容基线：已实现

未来优化：未实现

## 11. Community children 不 remap

**GraphRAG 行为：**Delta `community` 和 `parent` rebased，`children` 不变。

**问题：**child ID 可能仍指向 delta-local 编号。

**GraphLoom 兼容行为：**只映射 `community` 与 `parent`。

**未来优化：**一致 remap 全部 hierarchy reference。

**影响：**communities 与 hierarchical query context。

兼容基线：已实现

未来优化：未实现

## 12. Community report title 不重写

**GraphRAG 行为：**Report `community`、`parent`、`human_readable_id`
改变，但 `title` 不变。

**问题：**包含旧 community 编号的 title 会过时。

**GraphLoom 兼容行为：**保留 delta report title。

**未来优化：**按显式语义重新生成或重写 title。

**影响：**community reports 与 Global Search context。

兼容基线：已实现

未来优化：未实现

## 13. Merge 失败留下部分最终输出

**GraphRAG 行为：**Merge workflow 顺序写最终表，不回滚。

**问题：**失败会留下混合版本索引。

**GraphLoom 兼容行为：**保留已完成最终表/向量写入，以及 `previous`、
`delta` 供诊断。

**未来优化：**增加原子 publication 与 recovery metadata。

**影响：**全部 update workflow、最终存储、向量存储。

兼容基线：已实现

未来优化：未实现

## 14. Auto 对一个集合排序却返回另一个集合

**GraphRAG 行为：**Auto 随机抽取最多 `n_subset_max` 个 chunk，对样本
embedding 并按质心距离排序，然后把这些位置应用到原始、未抽样的 chunk
列表。

**问题：**最终选中的 chunk 不一定是 embedding 被排序的 chunk，因此模型
成本不能可靠改善样本代表性。

**GraphLoom 兼容行为：**与 GraphRAG 3.1.0 一样，把排序后的位置索引应用到
原始 chunk 列表。

**未来优化：**在显式优化选择模式中返回排序后的样本行本身，并暴露确定性
sampling hook。

**影响：**Prompt-tune Auto 选择、embedding 请求、prompt example。

兼容基线：已实现

未来优化：未实现

## 15. 过大 Random limit fallback 会制造可避免的错误

**GraphRAG 行为：**公共 API 会拒绝非正 `limit`、
`min_examples_required`、`n_subset_max` 和 `k`。正 limit 大于 chunk 数量
时进入 loader 并回退到 15；Random 随后在 chunk 少于 15 时失败。

**问题：**希望使用小语料全部 chunk 的请求反而报错，而且 fallback 隐藏了
无效的有效值。

**GraphLoom 兼容行为：**GraphLoom 在公共 API 边界校验上述四个正数字段，
并复现 oversized-limit fallback 及其 Random 错误。Top 作为独立路径仍会
clamp。

**未来优化：**直接验证请求值，或在名称明确的安全选择模式中使用
`min(limit, chunk_count)`。

**影响：**Prompt-tune Top/Random 选择和 CLI 诊断。

兼容基线：已实现

未来优化：未实现

## 16. Relationship example 复用累积请求

**GraphRAG 行为：**并发 relationship-example coroutine 共享一个可变
message builder。真正执行时，每个请求都看到包含全部已选文档的最终累积
message list。

**问题：**请求重复携带无关文档 message；总输入随 example 数量近似二次
增长；response 也不再隔离对应一个目标文档。

**GraphLoom 兼容行为：**为每个已选文档 clone 同一个累积请求，并按生产者
顺序收集 response。

**未来优化：**每个文档构建独立请求，或显式发送一次 batch 请求并定义
response 到 example 的映射。

**影响：**Prompt-tune relationship example、completion 成本和生成的
extract prompt。

兼容基线：已实现

未来优化：未实现

## 17. Claim gleaning response 被丢弃

**GraphRAG 行为：**Claim extraction 可以发送 continuation 和 loop-check
请求，但 tuple parsing 只使用初始 completion。

**问题：**额外模型调用消耗延迟和 token，却没有贡献新 claim；continuation
中有效的 claim 会丢失。

**GraphLoom 兼容行为：**执行 continuation conversation 和停止判断，但只
解析初始 response。

**未来优化：**以稳定去重方式解析、合并每个已接受 continuation；不要求
兼容时也可跳过这些 continuation 调用。

**影响：**Covariate extraction、LLM cache/成本和下游 claim 覆盖率。

兼容基线：已实现

未来优化：未实现

## 18. Claim 未进入 community-report graph context

**GraphRAG 行为：**Community-report 准备阶段把 claim 作为 scalar merge
value，而 context sorter 只接受 claim list，导致 claim 从渲染的 graph
context 中消失。

**问题：**Claim extraction 可能产生模型和存储成本，却不能增强 community
report grounding；为 covariate 预留的 token budget 也被浪费。

**GraphLoom 兼容行为：**Claim 仍属于 workflow 输入契约，但有意不出现在
兼容渲染 context 中。

**未来优化：**把 claim 规范为显式的 per-community list，并与 entity、
relationship 一起进行 token budget。

**影响：**Community-report context、covariate、report 质量与 token
budgeting。

兼容基线：已实现

未来优化：未实现

## 19. 静态 Query community roll-up 使用 entity title

**GraphRAG 行为：**静态 community-report adaptation 展开 entity
membership，按 entity title 分组，取最大 community number，再用这些编号
选择 report。

**问题：**同 title 的不同 entity 会被合并；不透明编号的最大值也不是稳定
的语义父子规则。

**GraphLoom 兼容行为：**静态 Global Query 复现按 title 取最大 community
的 roll-up。

**未来优化：**通过 entity ID 和显式 hierarchy relationship 选择 report，
并为重复 title 增加迁移测试。

**影响：**Query index adaptation、静态 Global context 与 community
selection。

兼容基线：已实现

未来优化：未实现

## 20. 标准索引直接发布到 active output

**GraphRAG 行为：**Workflow 直接写 active Parquet 和 vector output，没有
generation pointer、ready marker、跨命令锁或原子 activation。

**问题：**并发 Query 可能看到混合 generation；失败索引也可能留下局部但
可解析的 active index。

**GraphLoom 兼容行为：**标准索引保留 direct active publication。这与更
安全的事务式 `init` 和 prompt-tune 文件发布是两个独立边界。

**未来优化：**Stage 完整 generation，验证表和向量，再通过 generation
pointer 原子切换，并记录 recovery metadata。

**影响：**标准索引、Query 启动、storage layout 和失败恢复。

兼容基线：已实现

未来优化：未实现

## 21. 子社区报告不会替换父社区明细

**GraphRAG 行为：**标准 workflow 会先构造并冻结所有 community context，
然后才生成 report。Context builder 虽然能用已有子报告替换子社区明细，
但构造 context 时 report collection 为空，因此不会进入该分支。

**问题：**父层 prompt 会重复低层 entity/relationship 明细，而不能复用
密度更高的子摘要。Token 上限极低时，兼容的首条关系兜底还可能让最终
context 超出配置预算。

**GraphLoom 兼容行为：**在 report 生成前构造 context，并复现 GraphRAG
的裁剪与首条关系兜底。

**未来优化：**增加显式 bottom-up 模式，从最深层开始生成 report，按完整
prompt 预算安全替换已有子报告，并隔离基线与优化模式的 cache。

**影响：**Community-report 调度、层级 context、token budgeting、cache key
和 report 质量。详见[优化设计](community-report-hierarchical-context-optimization-zh.md)。

兼容基线：已实现

未来优化：未实现
