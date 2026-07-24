"""Tests for the real Query record/replay runner helpers."""

from __future__ import annotations

import json
from pathlib import Path

from llm_cache_proxy import ObservedRequest, exact_request_key, request_key
from query_record_replay import (
    _drift_request_contract,
    _timeout_text,
    compare_artifact_presence,
    compare_drift_behavior,
    compare_requests,
    load_api_key,
    prepare_consumer_view,
)


def _observed(
    body: dict[str, object],
    ordinal: int = 0,
    *,
    endpoint: str = "/v1/chat/completions",
    response_content: str | None = None,
) -> ObservedRequest:
    return ObservedRequest(
        ordinal=ordinal,
        key=request_key(endpoint, body),
        exact_key=exact_request_key(endpoint, body),
        endpoint=endpoint,
        body=body,
        cache_status="miss",
        response_content=response_content,
    )


def _drift_requests(
    candidates: list[str],
    selected: list[str],
    *,
    action_top_p: float = 1.0,
    decorate_action_response: bool = False,
    fallback_first_action: bool = False,
    localized_reduce: bool = False,
    action_followups: dict[str, list[str]] | None = None,
    action_top_p_overrides: dict[int, float] | None = None,
) -> tuple[ObservedRequest, ...]:
    action_followups = action_followups or {}
    action_top_p_overrides = action_top_p_overrides or {}
    requests = [
        _observed(
            {
                "model": "test",
                "messages": [
                    {
                        "role": "user",
                        "content": (
                            "Create a hypothetical answer to the following query: root"
                        ),
                    }
                ],
                "stream": False,
            },
            0,
            response_content="expanded",
        ),
        _observed(
            {"model": "embed", "input": ["expanded"]},
            1,
            endpoint="/v1/embeddings",
        ),
        _observed(
            {
                "model": "test",
                "messages": [
                    {
                        "role": "user",
                        "content": "Use top-ranked community summaries",
                    }
                ],
                "response_format": {"type": "json_schema"},
                "stream": False,
            },
            2,
            response_content=json.dumps(
                {
                    "intermediate_answer": "primer",
                    "score": 80,
                    "follow_up_queries": candidates,
                }
            ),
        ),
    ]
    ordinal = 3
    for action_index, query in enumerate(selected):
        action_response = json.dumps(
            {
                "response": f"answer-{query}",
                "score": 90,
                "follow_up_queries": action_followups.get(query, []),
            }
        )
        if decorate_action_response:
            action_response = f"answer before JSON\n```json\n{action_response}\n```"
        if fallback_first_action and action_index == 0:
            action_response = "unstructured fallback answer"
        requests.append(
            _observed(
                {"model": "embed", "input": [query]},
                ordinal,
                endpoint="/v1/embeddings",
            )
        )
        ordinal += 1
        requests.append(
            _observed(
                {
                    "model": "test",
                    "messages": [
                        {
                            "role": "system",
                            "content": "'follow_up_queries': List[str]",
                        },
                        {"role": "user", "content": query},
                    ],
                    "temperature": 0.0,
                    "top_p": action_top_p_overrides.get(
                        action_index,
                        action_top_p,
                    ),
                    "n": 1,
                    "max_completion_tokens": 100,
                    "stream": True,
                },
                ordinal,
                response_content=action_response,
            )
        )
        ordinal += 1
    requests.append(
        _observed(
            {
                "model": "test",
                "messages": [
                    {
                        "role": "system",
                        "content": (
                            "---数据报告---"
                            if localized_reduce
                            else "---Data Reports---"
                        ),
                    },
                    {"role": "user", "content": "root"},
                ],
                "temperature": 0.0,
                "stream": False,
            },
            ordinal,
            response_content="final",
        )
    )
    return tuple(requests)


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


def test_should_treat_omitted_drift_defaults_as_their_effective_values() -> None:
    body = {
        "model": "test",
        "messages": [{"role": "user", "content": "same prompt"}],
    }

    omitted = _drift_request_contract(_observed(body))
    explicit = _drift_request_contract(
        _observed(
            {
                **body,
                "temperature": 1.0,
                "top_p": 1.0,
                "n": 1,
                "stream": False,
            }
        )
    )

    assert omitted == explicit


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


def test_should_classify_valid_drift_branch_difference_as_expected_nondeterminism() -> (
    None
):
    candidates = ["Q1", "Q2", "Q3", "Q4"]
    report = compare_drift_behavior(
        _drift_requests(candidates, ["Q1", "Q4"]),
        _drift_requests(candidates, ["Q2", "Q3"]),
        query="root",
        drift_k_followups=2,
        n_depth=1,
    )

    assert report["compatible"] is True
    assert report["classification"] == "expected nondeterminism"
    assert report["expectedNondeterminism"] is True
    assert report["graphRag"]["layers"][0]["candidates"] == candidates
    assert report["graphRag"]["layers"][0]["selected"] == ["Q1", "Q4"]
    assert report["graphLoom"]["layers"][0]["selected"] == ["Q2", "Q3"]
    assert report["firstDivergenceDepth"] == 0
    assert "candidate sets were compatible" in report["message"].lower()


def test_should_allow_valid_drift_branch_to_change_action_and_layer_counts() -> None:
    candidates = ["Q1", "Q2", "Q3", "Q4"]
    graph_rag = _drift_requests(
        candidates,
        ["Q1", "Q2", "Q3", "Q4"],
    )
    graph_loom = _drift_requests(
        candidates,
        ["Q3", "Q4", "Q1", "Q2", "C1", "C2"],
        action_followups={
            "Q3": ["C1", "C2"],
            "Q4": ["C3"],
        },
    )

    report = compare_drift_behavior(
        graph_rag,
        graph_loom,
        query="root",
        drift_k_followups=2,
        n_depth=3,
    )

    assert report["compatible"] is True
    assert report["classification"] == "expected nondeterminism"
    assert report["firstDivergenceDepth"] == 0
    assert report["graphRag"]["layerCount"] == 2
    assert report["graphLoom"]["layerCount"] == 3
    assert report["graphRag"]["actionRequestCount"] == 4
    assert report["graphLoom"]["actionRequestCount"] == 6


def test_should_reject_excess_depth_after_valid_drift_branch() -> None:
    candidates = ["Q1", "Q2", "Q3", "Q4"]
    graph_rag = _drift_requests(candidates, ["Q1", "Q2", "Q3", "Q4"])
    graph_loom = _drift_requests(
        candidates,
        ["Q3", "Q4", "Q1", "Q2", "C1", "C2", "C3"],
        action_followups={
            "Q3": ["C1", "C2"],
            "Q4": ["C3"],
        },
    )

    report = compare_drift_behavior(
        graph_rag,
        graph_loom,
        query="root",
        drift_k_followups=2,
        n_depth=3,
    )

    assert report["compatible"] is False
    assert report["classification"] == "depth mismatch"
    assert any(error["code"] == "depth mismatch" for error in report["errors"])


def test_should_reject_illegal_action_after_valid_drift_branch() -> None:
    candidates = ["Q1", "Q2", "Q3", "Q4"]
    graph_rag = _drift_requests(candidates, ["Q1", "Q2", "Q3", "Q4"])
    graph_loom = _drift_requests(
        candidates,
        ["Q3", "Q4", "Q1", "Q9"],
        action_followups={"Q3": ["C1"]},
    )

    report = compare_drift_behavior(
        graph_rag,
        graph_loom,
        query="root",
        drift_k_followups=2,
        n_depth=3,
    )

    assert report["compatible"] is False
    assert report["classification"] == "illegal selected action"
    assert any(error["code"] == "illegal selected action" for error in report["errors"])


def test_should_reject_action_contract_mismatch_after_valid_drift_branch() -> None:
    candidates = ["Q1", "Q2", "Q3", "Q4"]
    graph_rag = _drift_requests(candidates, ["Q1", "Q2", "Q3", "Q4"])
    graph_loom = _drift_requests(
        candidates,
        ["Q3", "Q4", "Q1", "Q2", "C1", "C2"],
        action_followups={"Q3": ["C1", "C2"]},
        action_top_p_overrides={4: 0.5},
    )

    report = compare_drift_behavior(
        graph_rag,
        graph_loom,
        query="root",
        drift_k_followups=2,
        n_depth=3,
    )

    assert report["compatible"] is False
    assert report["classification"] == "request contract mismatch"
    assert any(
        error["code"] == "request contract mismatch" for error in report["errors"]
    )


def test_should_reject_layer_count_difference_before_random_branch() -> None:
    candidates = ["Q1", "Q2", "Q3", "Q4"]
    graph_rag = _drift_requests(candidates, ["Q1", "Q2", "Q3", "Q4"])
    graph_loom = _drift_requests(candidates, ["Q1", "Q2"])

    report = compare_drift_behavior(
        graph_rag,
        graph_loom,
        query="root",
        drift_k_followups=2,
        n_depth=3,
    )

    assert report["compatible"] is False
    assert report["classification"] == "depth mismatch"
    assert report["firstDivergenceDepth"] is None


def test_should_strictly_match_drift_when_scripted_trajectory_is_equal() -> None:
    candidates = ["Q1", "Q2", "Q3"]
    graph_rag = _drift_requests(candidates, ["Q3", "Q1"])
    graph_loom = _drift_requests(candidates, ["Q3", "Q1"])

    behavior = compare_drift_behavior(
        graph_rag,
        graph_loom,
        query="root",
        drift_k_followups=2,
        n_depth=1,
    )
    requests = compare_requests(graph_rag, graph_loom)

    assert behavior["compatible"] is True
    assert behavior["classification"] == "strict match"
    assert behavior["expectedNondeterminism"] is False
    assert requests["matchEqual"] is True
    assert requests["exactEqual"] is True


def test_should_parse_drift_action_response_like_graphrag() -> None:
    candidates = ["Q1", "Q2"]
    requests = _drift_requests(
        candidates,
        ["Q2"],
        decorate_action_response=True,
    )

    report = compare_drift_behavior(
        requests,
        requests,
        query="root",
        drift_k_followups=1,
        n_depth=1,
    )

    assert report["compatible"] is True
    assert report["classification"] == "strict match"


def test_should_keep_fallback_action_incomplete_for_the_next_depth() -> None:
    requests = _drift_requests(
        ["Q1"],
        ["Q1", "Q1"],
        fallback_first_action=True,
        localized_reduce=True,
    )

    report = compare_drift_behavior(
        requests,
        requests,
        query="root",
        drift_k_followups=1,
        n_depth=2,
    )

    assert report["compatible"] is True
    assert report["classification"] == "strict match"
    assert [layer["selected"] for layer in report["graphRag"]["layers"]] == [
        ["Q1"],
        ["Q1"],
    ]


def test_should_reject_drift_action_outside_incomplete_candidates() -> None:
    candidates = ["Q1", "Q2", "Q3"]
    report = compare_drift_behavior(
        _drift_requests(candidates, ["Q1", "Q2"]),
        _drift_requests(candidates, ["Q2", "Q9"]),
        query="root",
        drift_k_followups=2,
        n_depth=1,
    )

    assert report["compatible"] is False
    assert report["classification"] == "illegal selected action"
    assert any(
        error["message"]
        == "DRIFT selected action is not present in the incomplete candidate set: Q9"
        for error in report["errors"]
    )


def test_should_reject_drift_primer_candidate_multiset_mismatch() -> None:
    report = compare_drift_behavior(
        _drift_requests(["Q1", "Q2", "Q2"], ["Q1", "Q2"]),
        _drift_requests(["Q1", "Q2", "Q3"], ["Q1", "Q2"]),
        query="root",
        drift_k_followups=2,
        n_depth=1,
    )

    assert report["compatible"] is False
    assert report["classification"] == "candidate set mismatch"
    assert any(
        error["message"] == "DRIFT Primer follow-up candidate multisets differ"
        for error in report["errors"]
    )


def test_should_reject_drift_selection_count_and_request_contract_mismatch() -> None:
    candidates = ["Q1", "Q2", "Q3"]
    count_report = compare_drift_behavior(
        _drift_requests(candidates, ["Q1", "Q2"]),
        _drift_requests(candidates, ["Q1"]),
        query="root",
        drift_k_followups=2,
        n_depth=1,
    )
    contract_report = compare_drift_behavior(
        _drift_requests(candidates, ["Q1", "Q2"]),
        _drift_requests(candidates, ["Q1", "Q2"], action_top_p=0.5),
        query="root",
        drift_k_followups=2,
        n_depth=1,
    )

    assert any(
        error["code"] == "selection count mismatch" for error in count_report["errors"]
    )
    assert any(
        error["code"] == "request contract mismatch"
        for error in contract_report["errors"]
    )


def test_should_reject_duplicate_drift_selection_and_excess_depth() -> None:
    candidates = ["Q1", "Q2", "Q3"]
    duplicate_report = compare_drift_behavior(
        _drift_requests(candidates, ["Q1", "Q1"]),
        _drift_requests(candidates, ["Q1", "Q2"]),
        query="root",
        drift_k_followups=2,
        n_depth=1,
    )
    depth_report = compare_drift_behavior(
        _drift_requests(candidates, ["Q1", "Q2", "Q3"]),
        _drift_requests(candidates, ["Q1", "Q2"]),
        query="root",
        drift_k_followups=2,
        n_depth=1,
    )

    assert any(
        error["code"] == "duplicate selected action"
        for error in duplicate_report["errors"]
    )
    assert any(error["code"] == "depth mismatch" for error in depth_report["errors"])


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
