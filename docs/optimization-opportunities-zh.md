# GraphRAG 3.1.0 兼容优化机会

以下每一项都是 GraphLoom 为 GraphRAG 3.1.0 兼容基线而有意保留的行为。

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
