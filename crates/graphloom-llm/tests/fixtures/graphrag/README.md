# GraphRAG cache fixtures

Source: local `../graphrag` checkout at commit
`79ab7c9ad586856e82635264c200d8a1eb3c63d9`.

- Completion: `ragdebug/cache/extract_graph/04ad9d...e3e_v4`
- Embedding: `ragdebug/cache/text_embedding/90c045...81bc7_v4`

The fixtures contain model outputs and metrics only. They contain no API keys or credentials. Tests
use them to verify typed JSON semantic round trips and cache middleware interoperability.
