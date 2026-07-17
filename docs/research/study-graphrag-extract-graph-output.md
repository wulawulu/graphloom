# GraphRAG `extract_graph` 输出语义研究

状态：已完成 · 维护者：graphloom · 日期：2026-07-14 · 参考源码：`../graphrag` @
`79ab7c9ad586856e82635264c200d8a1eb3c63d9`

## 结论摘要

GraphLoom 与上述版本的 GraphRAG 使用了相同的 LLM 缓存响应，但两者生成的
`entities.parquet` 并非逐行一致。这不代表缓存未命中，也不是图抽取结果不同。差异发生在图抽取和
描述摘要完成之后：GraphRAG 仅使用 `title` 将实体摘要合并回抽取结果。

当同一个标题被识别为多个实体类型时，这种合并会产生笛卡尔积。一个标题下的每条摘要都会被连接到
该标题下的每条带类型实体记录，包括来自其他类型的摘要。GraphLoom 不进行这种二次连接，而是让
摘要结果始终跟随产生它的原始带类型实体记录。

GraphLoom 的实现具有更好的语义正确性：

- 保持 `type`、`description`、`text_unit_ids` 和 `frequency` 之间的对应关系；
- 每个抽取后的 `(title, type)` 分组恰好输出一条记录；
- 不会生成描述与类型不匹配的交叉记录，也不会在摘要阶段扩大实体数量；
- 同一标题对应的类型数量增加时，输出数量保持线性增长，而不是平方增长。

如果要求严格复刻当前 GraphRAG 的最终行，就必须同时复刻其不完整的连接键以及由此产生的语义不一致
记录。GraphLoom 将“缓存兼容”和“输出语义兼容”视为两个不同问题：它兼容 GraphRAG 的缓存协议和
LLM 响应，同时在工作流输出中保留更强的实体记录不变量。

## 检查范围与比较方法

本次比较使用同一份调试语料生成的以下文件：

| 文件 | GraphLoom | GraphRAG |
| --- | ---: | ---: |
| `entities.parquet` 行数 | 370 | 386 |
| `entities.parquet` 唯一标题数 | 362 | 362 |
| `relationships.parquet` 行数 | 1,107 | 1,107 |

被检查文件的精确信息如下：

| 文件 | 大小 | SHA-256 |
| --- | ---: | --- |
| GraphLoom `entities.parquet` | 78,640 字节 | `ab27b1f0c3bcf2d8aad13da5a9db9cb8a970e3e384c83c9ef09d419a26f4719d` |
| GraphRAG `entities.parquet` | 59,371 字节 | `391c9ed462cbfa500a09ea87a18155246cb796deb0200695f4ff1a32aaefe38b` |
| GraphLoom `relationships.parquet` | 113,046 字节 | `0c862026e2daa2610d942b40763e8f1cb492f3fe3613b43b6d1196d772535d1e` |
| GraphRAG `relationships.parquet` | 79,564 字节 | `ed991be709e2ade4ec0c4778679271239fe9645859805c5b31a3902a1ee858fe` |

逻辑比较先解码 Parquet 表，再对完整记录的多重集合进行比较，并对 Arrow 表示差异进行规范化。比较时
曾有意忽略行顺序：当时 GraphLoom 的 `BTreeMap` 按键排序，而 Pandas 的
`groupby(..., sort=False)` 保留首次出现顺序。这个处理足以比较 `extract_graph` 的关系多重集合，
但后续验证发现它不足以保证整个 pipeline 兼容；`create_communities` 会在无向重复边中保留最后一条，
因此关系行顺序会决定最终边权和社区划分。GraphLoom 现已改为与 Pandas 相同的首次出现分组顺序。
语义比较仍可忽略列顺序以及相互兼容的 Arrow 物理类型。

比较确认了以下事实：

1. 1,107 条关系记录的 `source`、`target`、`description`、`text_unit_ids` 和
   `weight` 全部一致。
2. 两份实体表都包含相同的 362 个唯一标题。
3. GraphLoom 的 370 条完整实体记录全部存在于 GraphRAG 的输出中。
4. GraphRAG 恰好多出 16 条实体记录。这些记录全部是后文所述的跨类型摘要组合；GraphLoom 没有任何
   无法在 GraphRAG 中找到的实体记录。

关系记录完全一致，并且 GraphLoom 的实体记录是 GraphRAG 实体记录的完整子集。这两点有力地说明
两个工作流消费了相同的抽取和摘要结果。如果 LLM 响应不同，通常会表现为关系内容变化，或者基础实体
分组缺失、增加或字段改变，而不会只产生本次观察到的连接组合。

## 受影响的八个标题

语料中有八个标题在抽取聚合后同时存在一个 `GEO` 分组和一个 `ORGANIZATION` 分组：

| 标题 | GraphLoom 行数 | GraphRAG 行数 | GraphRAG 额外行数 |
| --- | ---: | ---: | ---: |
| 东平府 | 2 | 4 | 2 |
| 守备府 | 2 | 4 | 2 |
| 张大户家 | 2 | 4 | 2 |
| 报恩寺 | 2 | 4 | 2 |
| 李家 | 2 | 4 | 2 |
| 清河县 | 2 | 4 | 2 |
| 玉皇庙 | 2 | 4 | 2 |
| 王婆茶坊 | 2 | 4 | 2 |
| **合计** | **16** | **32** | **16** |

其余标题只有一个带类型实体分组，因此不受影响：当标题在连接两侧都只有一行时，一对一连接和只按
标题连接会产生相同的行数。

## 具体示例：`守备府`

LLM 抽取完成后，GraphRAG 与 GraphLoom 都会按照 `(title, type)` 聚合实体。两个“守备府”分组
具有不同的来源、频次和最终摘要：

| 标题 | 类型 | 正确关联的摘要 | 频次 |
| --- | --- | --- | ---: |
| 守备府 | `GEO` | 守备府是西门庆派人送人情的地点，属于官府邸宅 | 2 |
| 守备府 | `ORGANIZATION` | 守备府是地方军事机构，提供一二十名军牢帮助西门庆搬抬嫁妆 | 1 |

每个分组还有自己的 `text_unit_ids`。这些 ID 和频次表达的是特定带类型实体分组的证据来源与出现次数，
而不只是“守备府”这个标题字符串的证据。

摘要阶段为每个输入分组生成一条描述。GraphRAG 随后生成的实体摘要临时表却只有两列：

| 标题 | 摘要描述 |
| --- | --- |
| 守备府 | 官府邸宅、送人情地点的摘要 |
| 守备府 | 地方军事机构、提供军牢的摘要 |

区分两个输入分组的 `type` 已经丢失。使用 `title` 将这张表连接回两条抽取记录时，会形成多对多连接：

```text
2 条“守备府”抽取记录 × 2 条“守备府”摘要记录 = 4 条输出记录
```

最终组合如下：

| 输出类型 | 附加的描述 | 结果 |
| --- | --- | --- |
| `GEO` | 地点摘要 | 对应关系正确 |
| `GEO` | 军事机构摘要 | 错误的跨类型组合 |
| `ORGANIZATION` | 地点摘要 | 错误的跨类型组合 |
| `ORGANIZATION` | 军事机构摘要 | 对应关系正确 |

两条交叉记录并非无害的重复数据。它们的 `type`、`text_unit_ids` 和 `frequency` 来自一个抽取分组，
而 `description` 来自另一个分组。因此，同一行中的类型、证据来源和语义描述互相矛盾。

## GraphRAG 算法及其问题

当前参考版本的 GraphRAG 在初始实体聚合时正确使用了两个键：

```python
all_entities.groupby(["title", "type"], sort=False)
```

源码位置：
`../graphrag/packages/graphrag/graphrag/index/operations/extract_graph/extract_graph.py:104-115`。

摘要操作会遍历聚合后的每条实体记录，但输出结果只保留 `title` 和 `description`，没有保留 `type`：

```python
node_descriptions = [
    {
        "title": result.id,
        "description": result.description,
    }
    for result in node_results
]
```

源码位置：
`../graphrag/packages/graphrag/graphrag/index/operations/summarize_descriptions/summarize_descriptions.py:59-66`。

最后，工作流只使用 `title` 把摘要连接回原始实体表：

```python
extracted_entities.drop(columns=["description"], inplace=True)
entities = extracted_entities.merge(entity_summaries, on="title", how="left")
```

源码位置：
`../graphrag/packages/graphrag/graphrag/index/workflows/extract_graph.py:183-184`。

如果同一个标题有 `k` 个带类型分组，连接左右两侧就各有 `k` 条相同标题的记录，因此连接会输出
`k²` 条记录。其中只有 `k` 条保留了摘要与类型之间的正确关系，其余 `k² - k` 条都是交叉组合。
在本次语料中，八个标题的 `k = 2`，所以每个标题增加两条错误记录，合计增加 16 条。

问题不在最初按照 `(title, type)` 聚合。这个聚合是必要的，因为同一个名称可以拥有不同的类型解释和
不同的支持证据。真正的问题是摘要身份中丢失了 `type`，随后又使用比原分组键更弱的键进行连接。

## GraphLoom 算法及其不变量

GraphLoom 同样按照 `(title, entity_type)` 聚合原始实体记录：

```rust
let key = (row.title.clone(), row.entity_type.clone());
```

源码位置：`crates/graphloom/src/operations/graph/merge.rs:7-22`。

在摘要阶段，每个异步操作拥有一条完整的 `EntityRow`。摘要返回后，GraphLoom 直接使用同一条记录
构造结果：

```rust
SummarizedEntityRow {
    title: row.title,
    entity_type: row.entity_type,
    description,
    text_unit_ids: row.text_unit_ids,
    frequency: row.frequency,
}
```

源码位置：`crates/graphloom/src/operations/graph/summarize.rs:90-106`。

整个过程不需要通过 DataFrame 二次连接恢复记录身份。输入索引会跟随每条记录一起进入异步任务，任务
完成后再按输入索引恢复顺序，因此并发执行也不会混淆摘要结果（`summarize.rs:111-120`）。

该设计保持了核心实体记录不变量：

```text
(title, type) 唯一标识一个聚合分组；该行中的
description、text_unit_ids 和 frequency 必须全部属于这个分组。
```

对于 `n` 条聚合实体记录，GraphLoom 始终返回 `n` 条摘要实体记录。摘要可以改变描述文本，但不能
改变记录身份、证据集合、频次或记录数量。

## 为什么 GraphLoom 的实现更好

### 1. 保持证据来源正确

`text_unit_ids` 指向为特定带类型实体分组提供描述的文本单元。如果附加了另一个类型的摘要，该描述就
无法由同一行保存的文本单元 ID 支持。GraphLoom 始终把证据与由这些证据生成的摘要保存在一起。

### 2. 保持分类与含义一致

实体记录不仅由标题构成，类型也决定标题的含义。同一个名称可以同时指代地点和组织，但两者代表不同的
解释。GraphLoom 不会给地点记录附加组织含义，也不会给组织记录附加地点含义。

### 3. 保持记录基数不变

描述摘要是值转换操作，不是图扩张操作。它应该把一组描述转换为一条摘要，同时保持实体分组的数量和
身份不变。GraphLoom 从结构上保证了这一性质；GraphRAG 的多对多连接则破坏了它。

### 4. 避免平方级放大

当一个标题对应 `k` 个类型时，GraphLoom 输出 `k` 条记录，而 GraphRAG 输出 `k²` 条记录。本次
观察到的规模较小，但算法问题具有普遍性。增加同一标题的分类数量不应该制造越来越多的实体记录。

### 5. 避免不必要的有损往返

GraphRAG 先把带类型记录转换为不带类型的摘要表，再尝试通过连接恢复对应关系。GraphLoom 在整个操作
中保留带类型领域对象。让数据流和类型系统直接保持不变量，比先丢失身份再尝试恢复更安全。

### 6. 更符合原始数据模型的含义

GraphRAG 自己在摘要前使用 `(title, type)` 定义实体分组身份。GraphLoom 在摘要后继续使用同一个身份。
如果复刻只按标题连接的行为，虽然可以匹配当前产物的行数，却会生成与 GraphRAG 自身前序分组键相冲突
的记录。

## Parquet 物理差异

即使逻辑记录相同，文件在存储层也存在以下差异：

- GraphLoom 写入 Arrow `string_view` 和 `large_list<string_view>`，参考文件使用 `string` 和
  `list<string>`；
- 列顺序不同：GraphLoom 将 `description` 放在 `text_unit_ids` 之前，而本次 GraphRAG 产物将其放在
  最后；
- 本次被比较的历史产物行顺序因两边当时的分组顺序策略不同而不同；当前实现已按 GraphRAG 的首次出现
  顺序输出分组，以保持下游社区检测兼容；
- GraphRAG 文件包含 Pandas schema 元数据，GraphLoom 不写入该元数据；
- 写入器、编码和压缩方式会导致文件大小和哈希不同。

因此，字节完全相同既不是这些 Parquet 文件的预期结果，也不是判断语义兼容性的有效标准。应区分以下
三个层级：

1. **缓存兼容：**相同请求可以读取 GraphRAG 缓存，并恢复相同的 LLM 响应。
2. **逻辑字段兼容：**等价实体和关系具有兼容的 schema 与含义。
3. **产物复制：**完全复刻行顺序、Arrow 物理类型、元数据，甚至参考实现中的异常行为。

GraphLoom 的目标是前两个层级。`entities.parquet` 的差异来自对正确逻辑关系的保留，而不是对
GraphRAG 交叉记录的复刻。`relationships.parquet` 没有类似问题，因为关系摘要和连接在两侧都使用
完整的 `("source", "target")` 身份。

## 兼容性决策

GraphLoom 应保留当前“一条输入记录对应一条输出记录”的摘要设计。不应为了让行数与当前 GraphRAG
产物一致而加入只按标题连接的逻辑，也不应生成笛卡尔积记录。

后续兼容性检查应验证：

- 物理表示规范化后，关系记录的多重集合相同；
- `relationships.parquet` 的语义行顺序与 GraphRAG 相同，因为社区检测的“保留最后一条重复无向边”
  会把顺序差异转化为边权差异；
- 实体 `(title, type)` 分组及其正确关联的字段相同；
- GraphLoom 不会输出描述来自其他带类型分组的记录；
- 仅由 GraphRAG 标题连接笛卡尔积引起的差异应被明确报告，不应被判断为 LLM 或缓存不兼容。

如果未来确实需要逐字节或逐行复制参考产物，应将其设计为名称明确的独立兼容模式，并记录其语义代价，
而不能用它替换默认的、不变量保持行为。

## 与缓存互操作研究的关系

缓存格式和缓存键保证记录在
[GraphRAG v4 LLM 缓存互操作研究](study-graphrag-llm-cache.md)中。缓存互操作保证 GraphLoom 获得相同的
模型响应，但不要求复刻 GraphRAG 随后的所有 Pandas 转换，更不要求复刻参考工作流中的异常行为。
