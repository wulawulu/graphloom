"""Tests for the real Query record/replay runner helpers."""

from __future__ import annotations

import json
from pathlib import Path

from llm_cache_proxy import ObservedRequest, exact_request_key, request_key
from query_record_replay import (
    _timeout_text,
    compare_artifact_presence,
    compare_requests,
    load_api_key,
    prepare_consumer_view,
)


def _observed(body: dict[str, object], ordinal: int = 0) -> ObservedRequest:
    endpoint = "/v1/chat/completions"
    return ObservedRequest(
        ordinal=ordinal,
        key=request_key(endpoint, body),
        exact_key=exact_request_key(endpoint, body),
        endpoint=endpoint,
        body=body,
        cache_status="miss",
    )


def test_should_treat_message_whitespace_as_a_semantic_difference() -> None:
    left = _observed(
        {
            "model": "test",
            "messages": [{"role": "user", "content": "line one\nline two"}],
        }
    )
    right = _observed(
        {
            "model": "test",
            "messages": [{"role": "user", "content": "line one\r\nline two"}],
            "stream": False,
        }
    )

    report = compare_requests((left,), (right,))

    assert report["equal"] is False
    assert report["matchEqual"] is False
    assert report["exactEqual"] is False
    assert {item["path"] for item in report["differences"]} == {
        "$.body.messages[0].content",
        "$.body.stream",
    }
    affects_match = {
        item["path"]: item["affectsMatch"] for item in report["differences"]
    }
    assert affects_match == {
        "$.body.messages[0].content": True,
        "$.body.stream": False,
    }
    assert report["differences"][0]["firstDifference"] == {
        "byte": 8,
        "line": 1,
        "column": 9,
    }


def test_should_report_missing_request_without_serializing_sentinel() -> None:
    report = compare_requests((_observed({"model": "test"}),), ())

    assert report["equal"] is False
    assert len(report["differences"]) == 1
    difference = report["differences"][0]
    assert difference["path"] == "$"
    assert difference["affectsMatch"] is True
    assert difference["graphRagOrdinal"] == 0
    assert difference["graphRagPresent"] is True
    assert difference["graphLoomPresent"] is False
    json.dumps(report)


def test_should_ignore_transport_fields_for_match_but_report_them() -> None:
    messages = [{"role": "user", "content": "same prompt"}]
    left = _observed({"model": "test", "messages": messages})
    right = _observed(
        {
            "model": "test",
            "messages": messages,
            "stream": False,
            "response_format": {"type": "json_object"},
            "temperature": 0.0,
        }
    )

    report = compare_requests((left,), (right,))

    assert report["equal"] is True
    assert report["matchEqual"] is True
    assert report["exactEqual"] is False
    assert all(not item["affectsMatch"] for item in report["differences"])


def test_should_match_concurrent_requests_independent_of_observation_order() -> None:
    first = _observed(
        {"model": "test", "messages": [{"role": "user", "content": "first"}]},
        ordinal=0,
    )
    second = _observed(
        {"model": "test", "messages": [{"role": "user", "content": "second"}]},
        ordinal=1,
    )

    report = compare_requests((first, second), (second, first))

    assert report["matchEqual"] is True
    assert report["exactEqual"] is False
    assert report["differences"][-1]["path"] == "$.requestOrder"
    assert report["differences"][-1]["affectsMatch"] is False


def test_should_report_index_artifacts_present_on_only_one_side(tmp_path: Path) -> None:
    graph_rag = tmp_path / "graphrag"
    graph_loom = tmp_path / "graphloom"
    graph_rag.mkdir()
    graph_loom.mkdir()
    (graph_rag / "entities.parquet").touch()
    (graph_loom / "entities.parquet").touch()
    (graph_rag / "covariates.parquet").touch()

    report = compare_artifact_presence(graph_rag, graph_loom)

    assert report["equal"] is False
    assert report["onlyGraphRag"] == ["covariates.parquet"]
    assert report["onlyGraphLoom"] == []


def test_should_prepare_ephemeral_openai_view_without_mutating_source(
    tmp_path: Path,
) -> None:
    source = tmp_path / "source"
    destination = tmp_path / "view"
    (source / "prompts").mkdir(parents=True)
    (source / "output" / "lancedb").mkdir(parents=True)
    (source / "prompts" / "query.txt").write_text("prompt")
    original = (
        "completion_models:\n"
        "  default:\n"
        "    model_provider: deepseek\n"
        "    model: chat\n"
        "    api_key: old-secret\n"
        "embedding_models:\n"
        "  default:\n"
        "    model_provider: ollama\n"
        "    model: embed\n"
        "    api_key: ollama\n"
        "vector_store:\n"
        "  type: lancedb\n"
        "  db_uri: output/lancedb\n"
    )
    (source / "settings.yaml").write_text(original)

    prepare_consumer_view(source, destination, "http://127.0.0.1:1234/v1", "key")

    rendered = (destination / "settings.yaml").read_text()
    assert rendered.count("model_provider: openai") == 2
    assert rendered.count("api_base: http://127.0.0.1:1234/v1") == 2
    assert "concurrent_requests" not in rendered
    assert rendered.count("api_key: key") == 1
    assert rendered.count("api_key: local-proxy") == 1
    assert str((source / "output" / "lancedb").resolve()) in rendered
    assert (source / "settings.yaml").read_text() == original


def test_should_load_only_bounded_graphrag_key(tmp_path: Path) -> None:
    dotenv = tmp_path / ".env"
    dotenv.write_text("IGNORED=value\nGRAPHRAG_API_KEY=secret\n")

    assert load_api_key(dotenv) == "secret"


def test_should_decode_timeout_output_without_raising() -> None:
    assert _timeout_text(b"partial \xff", "fallback") == "partial �"
    assert _timeout_text(None, "fallback") == "fallback"
