# 兼容 Fixture

`query/query_interop_request_contract.json` 是从隔离的 PyPI
`graphrag==3.1.0` 环境捕获并审查的 Query provider-stage 请求契约。
普通测试只读取该文件。

固定契约可防止普通 gate 为每个消费者、方法、动态选择和流式模式重新学习
预期值；否则一个确定性兼容门禁会变成数百次模型调用。
