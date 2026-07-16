# Compatibility input fixtures

The two UTF-8 documents under `input/` are short, lightly condensed excerpts
selected from the public-domain Chinese novel 《金瓶梅》. The local source used
to select them was `../graphrag/debug/input/金瓶梅.txt`.

The complete local file is intentionally not copied: it is about 196 KB and
would turn the deterministic compatibility gate into hundreds of model calls.
These excerpts preserve its Chinese text, multiple-document structure, named
entities, and enough length to exercise chunking and overlap in CI.
