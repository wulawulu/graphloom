#!/usr/bin/env python3
"""Verify GraphLoom fixtures with the pinned local GraphRAG Python types."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--graphrag-root", type=Path, required=True)
    parser.add_argument("--generated-cache-root", type=Path)
    args = parser.parse_args()
    root = args.graphrag_root.resolve()
    for package in ("graphrag", "graphrag-llm", "graphrag-cache", "graphrag-common"):
        sys.path.insert(0, str(root / "packages" / package))

    from graphrag.cache.cache_key_creator import cache_key_creator
    from graphrag_llm.types import LLMCompletionResponse, LLMEmbeddingResponse

    fixtures = (
        Path(__file__).resolve().parents[1]
        / "crates/graphloom-llm/tests/fixtures/graphrag"
    )
    completion_path = next((fixtures / "completion").glob("*_v4"))
    embedding_path = next((fixtures / "embedding").glob("*_v4"))
    completion = json.loads(completion_path.read_text(encoding="utf-8"))["result"]
    embedding = json.loads(embedding_path.read_text(encoding="utf-8"))["result"]
    LLMCompletionResponse(**completion["response"])
    LLMEmbeddingResponse(**embedding["response"])
    generated = fixtures.parent / "graphloom"
    generated_completion = json.loads(
        (generated / "completion_cache.json").read_text(encoding="utf-8")
    )["result"]
    generated_embedding = json.loads(
        (generated / "embedding_cache.json").read_text(encoding="utf-8")
    )["result"]
    LLMCompletionResponse(**generated_completion["response"])
    LLMEmbeddingResponse(**generated_embedding["response"])

    if args.generated_cache_root is not None:
        generated_root = args.generated_cache_root.resolve()
        generated_files = (
            ("completion", "extract_graph", LLMCompletionResponse),
            ("embedding", "embed_text", LLMEmbeddingResponse),
        )
        for kind, namespace, response_type in generated_files:
            request = json.loads((fixtures / f"{kind}_request.json").read_text())
            expected = cache_key_creator(request)
            path = generated_root / kind / namespace / expected
            payload = json.loads(path.read_text(encoding="utf-8"))["result"]
            response_type(**payload["response"])
            if not isinstance(payload.get("metrics"), dict):
                raise AssertionError(f"{kind} generated cache metrics is not an object")
            if path.name != expected:
                raise AssertionError(f"{kind} generated cache filename mismatch")

    for kind in ("completion", "embedding"):
        request = json.loads((fixtures / f"{kind}_request.json").read_text())
        expected = (fixtures / f"{kind}_expected_key.txt").read_text().strip()
        actual = cache_key_creator(request)
        if actual != expected:
            raise AssertionError(f"{kind} key mismatch: {actual} != {expected}")

    print("PASS: GraphRAG decoded GraphRAG and GraphLoom completion/embedding payloads")
    print("PASS: GraphRAG completion and embedding v4 keys match Rust goldens")
    if args.generated_cache_root is not None:
        print("PASS: GraphRAG decoded runtime-generated FileStorage + JsonCache files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
