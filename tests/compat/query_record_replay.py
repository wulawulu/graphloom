"""Run GraphRAG and GraphLoom Query through one semantic caching proxy."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from collections import Counter, defaultdict
from dataclasses import asdict
from pathlib import Path
from typing import Any

import yaml
from graphrag.query.llm.text_utils import try_parse_json_object

from llm_cache_proxy import (
    CachingProxyEngine,
    CachingProxyServer,
    JsonlCache,
    LiteLlmBackend,
    ObservedRequest,
    UpstreamRoute,
)

METHODS = ("basic", "local", "global", "drift")
CASE_PATTERN = re.compile(r"^[A-Za-z0-9_-]{1,64}$")
QUERY_TIMEOUT_SECONDS = 20 * 60
MISSING = object()
DRIFT_HYDE_PREFIX = "Create a hypothetical answer to the following query: "
DRIFT_PRIMER_MARKER = "top-ranked community summaries"
DRIFT_ACTION_MARKER = "'follow_up_queries': List[str]"
DRIFT_REDUCE_MARKERS = ("---Data Reports---", "---数据报告---")
DRIFT_CONTRACT_FIELDS = (
    "model",
    "response_format",
    "temperature",
    "top_p",
    "n",
    "max_tokens",
    "max_completion_tokens",
    "stream",
)
DRIFT_CONTRACT_DEFAULTS = {
    "response_format": None,
    "temperature": 1,
    "top_p": 1,
    "n": 1,
    "max_tokens": None,
    "max_completion_tokens": None,
    "stream": False,
}
GRAPHRAG_3_1_DRIFT_K_FOLLOWUPS = 20
GRAPHRAG_3_1_DRIFT_N_DEPTH = 3


def load_api_key(path: Path) -> str:
    """Load only GRAPHRAG_API_KEY from one bounded dotenv file."""
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 64 * 1024:
        raise ValueError(f"invalid dotenv file: {path}")
    values: list[str] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        name, separator, value = stripped.partition("=")
        if separator and name.strip() == "GRAPHRAG_API_KEY":
            values.append(value.strip().strip('"').strip("'"))
    if len(values) != 1 or not values[0] or len(values[0].encode()) > 8192:
        raise ValueError("debug/.env must define one bounded GRAPHRAG_API_KEY")
    return values[0]


def prepare_consumer_view(
    source: Path,
    destination: Path,
    api_base: str,
    api_key: str,
) -> None:
    """Create an ephemeral view over one immutable debug index."""
    destination.mkdir(parents=True)
    prompts = source / "prompts"
    if prompts.is_dir():
        shutil.copytree(prompts, destination / "prompts")
    settings_path = source / "settings.yaml"
    settings = yaml.safe_load(settings_path.read_text(encoding="utf-8"))
    if not isinstance(settings, dict):
        raise ValueError(f"settings root must be a mapping: {settings_path}")
    for section in ("completion_models", "embedding_models"):
        models = settings.get(section)
        if not isinstance(models, dict) or not models:
            raise ValueError(f"settings has no {section}: {settings_path}")
        for model in models.values():
            if not isinstance(model, dict):
                raise ValueError(f"{section} values must be mappings")
            model["model_provider"] = "openai"
            model["api_base"] = api_base
            model["api_key"] = (
                api_key if section == "completion_models" else "local-proxy"
            )
            model["retry"] = {"type": "exponential_backoff", "max_retries": 2}
    vector_store = settings.get("vector_store")
    if not isinstance(vector_store, dict):
        raise ValueError(f"settings has no vector_store: {settings_path}")
    vector_store["db_uri"] = str((source / "output" / "lancedb").resolve())
    (destination / "settings.yaml").write_text(
        yaml.safe_dump(settings, sort_keys=False, allow_unicode=True),
        encoding="utf-8",
    )


def compare_requests(
    graphrag: tuple[ObservedRequest, ...],
    graphloom: tuple[ObservedRequest, ...],
) -> dict[str, Any]:
    """Compare normalized request multisets while preserving raw differences."""
    differences: list[dict[str, Any]] = []
    graph_rag_groups: defaultdict[str, list[ObservedRequest]] = defaultdict(list)
    graph_loom_groups: defaultdict[str, list[ObservedRequest]] = defaultdict(list)
    for request in graphrag:
        graph_rag_groups[request.key].append(request)
    for request in graphloom:
        graph_loom_groups[request.key].append(request)

    graph_rag_match_counts = Counter(request.key for request in graphrag)
    graph_loom_match_counts = Counter(request.key for request in graphloom)
    match_equal = graph_rag_match_counts == graph_loom_match_counts
    exact_order_equal = [request.exact_key for request in graphrag] == [
        request.exact_key for request in graphloom
    ]
    unmatched_graph_rag: list[ObservedRequest] = []
    unmatched_graph_loom: list[ObservedRequest] = []

    for key in sorted(graph_rag_match_counts.keys() | graph_loom_match_counts.keys()):
        pairs, left_only, right_only = _pair_requests(
            graph_rag_groups[key], graph_loom_groups[key]
        )
        for left, right in pairs:
            _append_raw_differences(differences, left, right, matched=True)

        unmatched_graph_rag.extend(left_only)
        unmatched_graph_loom.extend(right_only)

    diagnostic_pairs, unmatched_graph_rag, unmatched_graph_loom = (
        _pair_unmatched_by_endpoint_kind(unmatched_graph_rag, unmatched_graph_loom)
    )
    for left, right in diagnostic_pairs:
        _append_raw_differences(differences, left, right, matched=False)

    for side, requests in (
        ("graphRag", unmatched_graph_rag),
        ("graphLoom", unmatched_graph_loom),
    ):
        for request in requests:
            other_side = "graphLoom" if side == "graphRag" else "graphRag"
            differences.append(
                {
                    "matchKey": request.key,
                    "path": "$",
                    "affectsMatch": True,
                    f"{side}Present": True,
                    f"{other_side}Present": False,
                    f"{side}Ordinal": request.ordinal,
                    "endpoint": request.endpoint,
                }
            )

    exact_multiset_equal = Counter(
        request.exact_key for request in graphrag
    ) == Counter(request.exact_key for request in graphloom)
    if not exact_order_equal and exact_multiset_equal:
        differences.append(
            {
                "path": "$.requestOrder",
                "affectsMatch": False,
                "graphRagValue": [request.exact_key for request in graphrag],
                "graphLoomValue": [request.exact_key for request in graphloom],
            }
        )
    return {
        "equal": match_equal,
        "matchEqual": match_equal,
        "exactEqual": exact_order_equal,
        "graphRagCount": len(graphrag),
        "graphLoomCount": len(graphloom),
        "differences": differences,
    }


def compare_drift_behavior(
    graphrag: tuple[ObservedRequest, ...],
    graphloom: tuple[ObservedRequest, ...],
    *,
    query: str,
    drift_k_followups: int,
    n_depth: int,
) -> dict[str, Any]:
    """Compare DRIFT constraints while preserving legitimate random branching."""
    graph_rag_trace = _build_drift_trace(
        graphrag,
        side="GraphRAG",
        query=query,
        drift_k_followups=drift_k_followups,
        n_depth=n_depth,
    )
    graph_loom_trace = _build_drift_trace(
        graphloom,
        side="GraphLoom",
        query=query,
        drift_k_followups=drift_k_followups,
        n_depth=n_depth,
    )
    errors = [*graph_rag_trace["errors"], *graph_loom_trace["errors"]]

    _compare_drift_stage_requests(
        errors,
        "HyDE",
        graph_rag_trace["operations"]["hyde"],
        graph_loom_trace["operations"]["hyde"],
        compare_messages=True,
    )
    _compare_drift_stage_requests(
        errors,
        "HyDE embedding",
        graph_rag_trace["operations"]["hydeEmbedding"],
        graph_loom_trace["operations"]["hydeEmbedding"],
        compare_messages=True,
    )
    _compare_drift_stage_requests(
        errors,
        "Primer",
        graph_rag_trace["operations"]["primer"],
        graph_loom_trace["operations"]["primer"],
        compare_messages=True,
    )
    _compare_drift_stage_requests(
        errors,
        "action",
        graph_rag_trace["operations"]["action"],
        graph_loom_trace["operations"]["action"],
        compare_messages=False,
        compare_counts=False,
    )
    _compare_drift_stage_requests(
        errors,
        "Reduce",
        graph_rag_trace["operations"]["reduce"],
        graph_loom_trace["operations"]["reduce"],
        compare_messages=False,
    )

    graph_rag_primer = graph_rag_trace["primer"]
    graph_loom_primer = graph_loom_trace["primer"]
    if graph_rag_primer["answers"] != graph_loom_primer["answers"]:
        errors.append(
            _drift_error(
                "candidate set mismatch",
                "DRIFT Primer initial answers differ",
            )
        )
    if graph_rag_primer["scores"] != graph_loom_primer["scores"]:
        errors.append(
            _drift_error(
                "candidate set mismatch",
                "DRIFT Primer scores differ",
            )
        )
    if Counter(graph_rag_primer["followups"]) != Counter(
        graph_loom_primer["followups"]
    ):
        errors.append(
            _drift_error(
                "candidate set mismatch",
                "DRIFT Primer follow-up candidate multisets differ",
            )
        )

    first_divergence_depth: int | None = None
    paths_aligned = True
    graph_rag_layers = graph_rag_trace["layers"]
    graph_loom_layers = graph_loom_trace["layers"]
    for graph_rag_layer, graph_loom_layer in zip(
        graph_rag_layers,
        graph_loom_layers,
        strict=False,
    ):
        depth = graph_rag_layer["depth"]
        if not paths_aligned:
            continue
        candidates_equal = Counter(graph_rag_layer["candidates"]) == Counter(
            graph_loom_layer["candidates"]
        )
        if not candidates_equal:
            errors.append(
                _drift_error(
                    "candidate set mismatch",
                    f"DRIFT incomplete candidate sets differ at depth {depth}",
                    depth=depth,
                )
            )
            paths_aligned = False
            continue
        if Counter(graph_rag_layer["selected"]) != Counter(
            graph_loom_layer["selected"]
        ):
            if _drift_layer_selection_is_valid(
                graph_rag_layer
            ) and _drift_layer_selection_is_valid(graph_loom_layer):
                first_divergence_depth = depth
            paths_aligned = False

    if first_divergence_depth is None and len(graph_rag_layers) != len(
        graph_loom_layers
    ):
        errors.append(
            _drift_error(
                "depth mismatch",
                (
                    "DRIFT executed layer counts differ before any valid random "
                    f"branch divergence: GraphRAG={len(graph_rag_layers)}, "
                    f"GraphLoom={len(graph_loom_layers)}"
                ),
            )
        )

    compatible = not errors
    expected_nondeterminism = compatible and first_divergence_depth is not None
    if expected_nondeterminism:
        classification = "expected nondeterminism"
        message = (
            "DRIFT follow-up selection differs due to expected nondeterminism. "
            "Candidate sets were compatible before the first valid random branch "
            "divergence. Both sides independently satisfy candidate, selection-count, "
            "request-contract, embedding-input, and depth constraints."
        )
    elif compatible:
        classification = "strict match"
        message = (
            "DRIFT random choices converged and the complete behavior is compatible."
        )
    else:
        classification = errors[0]["code"]
        message = errors[0]["message"]
    return {
        "mode": "default-random",
        "compatible": compatible,
        "classification": classification,
        "message": message,
        "expectedNondeterminism": expected_nondeterminism,
        "firstDivergenceDepth": first_divergence_depth,
        "driftKFollowups": drift_k_followups,
        "nDepth": n_depth,
        "graphRag": _public_drift_trace(graph_rag_trace),
        "graphLoom": _public_drift_trace(graph_loom_trace),
        "errors": errors,
    }


def _build_drift_trace(
    requests: tuple[ObservedRequest, ...],
    *,
    side: str,
    query: str,
    drift_k_followups: int,
    n_depth: int,
) -> dict[str, Any]:
    operations: defaultdict[str, list[ObservedRequest]] = defaultdict(list)
    for request in sorted(requests, key=lambda item: item.ordinal):
        operations[_drift_operation(request)].append(request)
    errors: list[dict[str, Any]] = []
    if operations["unknown"]:
        errors.append(
            _drift_error(
                "request contract mismatch",
                f"{side} emitted unclassified DRIFT completion requests",
                side=side,
            )
        )

    hyde = operations["hyde"]
    embeddings = operations["embedding"]
    primer = operations["primer"]
    actions = operations["action"]
    reduce = operations["reduce"]
    if len(hyde) != 1:
        errors.append(
            _drift_error(
                "request contract mismatch",
                f"{side} emitted {len(hyde)} HyDE requests; expected 1",
                side=side,
            )
        )
    if len(reduce) != 1:
        errors.append(
            _drift_error(
                "request contract mismatch",
                f"{side} emitted {len(reduce)} Reduce requests; expected 1",
                side=side,
            )
        )

    hyde_embedding = embeddings[:1]
    action_embeddings = embeddings[1:]
    if len(hyde_embedding) != 1:
        errors.append(
            _drift_error(
                "request contract mismatch",
                f"{side} emitted no HyDE embedding request",
                side=side,
            )
        )
    elif hyde:
        expanded = hyde[0].response_content or query
        if _embedding_inputs(hyde_embedding[0]) != [expanded]:
            errors.append(
                _drift_error(
                    "request contract mismatch",
                    f"{side} did not embed the HyDE response or fallback query",
                    side=side,
                )
            )

    primer_summary = _primer_summary(primer, errors, side)
    incomplete = _unique(primer_summary["followups"])
    known = set(incomplete)
    action_cursor = 0
    layers: list[dict[str, Any]] = []
    selected_queries: list[str] = []
    for depth in range(n_depth):
        if not incomplete:
            break
        expected_count = min(len(incomplete), drift_k_followups)
        layer_requests = actions[action_cursor : action_cursor + expected_count]
        selected = [_request_user_message(request) for request in layer_requests]
        layer = {
            "depth": depth,
            "candidates": list(incomplete),
            "selected": selected,
            "incompleteActionCount": len(incomplete),
            "expectedSelectionCount": expected_count,
        }
        layers.append(layer)
        if len(layer_requests) != expected_count:
            code = (
                "depth mismatch"
                if not layer_requests and action_cursor == len(actions)
                else "selection count mismatch"
            )
            detail = (
                f"{side} ended before depth {depth} while "
                f"{len(incomplete)} incomplete actions remained"
                if code == "depth mismatch"
                else (
                    f"{side} selected {len(layer_requests)} actions at depth {depth}; "
                    f"expected {expected_count}"
                )
            )
            errors.append(
                _drift_error(
                    code,
                    detail,
                    side=side,
                    depth=depth,
                )
            )
            action_cursor += len(layer_requests)
            break
        if len(selected) != len(set(selected)):
            errors.append(
                _drift_error(
                    "duplicate selected action",
                    f"{side} selected the same action more than once at depth {depth}",
                    side=side,
                    depth=depth,
                )
            )
        for selected_query in selected:
            if selected_query not in incomplete:
                errors.append(
                    _drift_error(
                        "illegal selected action",
                        (
                            "DRIFT selected action is not present in the incomplete "
                            f"candidate set: {selected_query}"
                        ),
                        side=side,
                        depth=depth,
                    )
                )
        selected_queries.extend(selected)
        completed: set[str] = set()
        new_followups: list[str] = []
        for selected_query, request in zip(selected, layer_requests, strict=True):
            response = _parse_response_object(request, errors, side, "action")
            if isinstance(response.get("response"), str):
                completed.add(selected_query)
            followups = response.get("follow_up_queries", [])
            if followups is None:
                followups = []
            elif not isinstance(followups, list) or not all(
                isinstance(item, str) for item in followups
            ):
                errors.append(
                    _drift_error(
                        "request contract mismatch",
                        f"{side} action response has invalid follow_up_queries",
                        side=side,
                        depth=depth,
                    )
                )
                continue
            for followup in followups:
                if followup not in known:
                    known.add(followup)
                    new_followups.append(followup)
        incomplete = [
            candidate for candidate in incomplete if candidate not in completed
        ]
        incomplete.extend(new_followups)
        action_cursor += expected_count

    if action_cursor < len(actions):
        errors.append(
            _drift_error(
                "depth mismatch",
                (
                    f"{side} emitted {len(actions) - action_cursor} action requests "
                    f"after n_depth={n_depth}"
                ),
                side=side,
            )
        )
    action_embedding_inputs = [
        value for request in action_embeddings for value in _embedding_inputs(request)
    ]
    if Counter(action_embedding_inputs) != Counter(selected_queries):
        errors.append(
            _drift_error(
                "request contract mismatch",
                (
                    f"{side} action embedding inputs do not match the selected "
                    "follow-up actions"
                ),
                side=side,
            )
        )
    return {
        "operations": {
            "hyde": hyde,
            "hydeEmbedding": hyde_embedding,
            "primer": primer,
            "action": actions,
            "reduce": reduce,
        },
        "primer": primer_summary,
        "layers": layers,
        "errors": errors,
    }


def _compare_drift_stage_requests(
    errors: list[dict[str, Any]],
    stage: str,
    graphrag: list[ObservedRequest],
    graphloom: list[ObservedRequest],
    *,
    compare_messages: bool,
    compare_counts: bool = True,
) -> None:
    if compare_messages:
        graph_rag_contracts = Counter(
            json.dumps(
                {
                    "matchKey": request.key,
                    "contract": _drift_request_contract(request),
                },
                sort_keys=True,
            )
            for request in graphrag
        )
        graph_loom_contracts = Counter(
            json.dumps(
                {
                    "matchKey": request.key,
                    "contract": _drift_request_contract(request),
                },
                sort_keys=True,
            )
            for request in graphloom
        )
    else:
        graph_rag_contracts = Counter(
            json.dumps(_drift_request_contract(request), sort_keys=True)
            for request in graphrag
        )
        graph_loom_contracts = Counter(
            json.dumps(_drift_request_contract(request), sort_keys=True)
            for request in graphloom
        )
    if compare_counts:
        contracts_equal = graph_rag_contracts == graph_loom_contracts
    else:
        contracts_equal = set(graph_rag_contracts) == set(graph_loom_contracts)
    if not contracts_equal:
        errors.append(
            _drift_error(
                "request contract mismatch",
                f"DRIFT {stage} request contracts differ",
            )
        )


def _drift_request_contract(request: ObservedRequest) -> dict[str, Any]:
    messages = request.body.get("messages")
    roles = (
        [message.get("role") for message in messages if isinstance(message, dict)]
        if isinstance(messages, list)
        else []
    )
    fields = {
        name: _normalize_contract_value(
            request.body.get(name, DRIFT_CONTRACT_DEFAULTS.get(name))
        )
        for name in DRIFT_CONTRACT_FIELDS
    }
    return {
        "endpoint": request.endpoint,
        "messageRoles": roles,
        "fields": fields,
    }


def _normalize_contract_value(value: Any) -> Any:
    if isinstance(value, float) and value.is_integer():
        return int(value)
    if isinstance(value, list):
        return [_normalize_contract_value(item) for item in value]
    if isinstance(value, dict):
        return {key: _normalize_contract_value(item) for key, item in value.items()}
    return value


def _primer_summary(
    requests: list[ObservedRequest],
    errors: list[dict[str, Any]],
    side: str,
) -> dict[str, Any]:
    answers: list[str] = []
    scores: list[int | float] = []
    followups: list[str] = []
    for request in sorted(requests, key=lambda item: item.key):
        response = _parse_response_object(request, errors, side, "Primer")
        answer = response.get("intermediate_answer")
        score = response.get("score")
        candidates = response.get("follow_up_queries")
        if not isinstance(answer, str):
            errors.append(
                _drift_error(
                    "request contract mismatch",
                    f"{side} Primer response has no string intermediate_answer",
                    side=side,
                )
            )
        else:
            answers.append(answer)
        if isinstance(score, bool) or not isinstance(score, int | float):
            errors.append(
                _drift_error(
                    "request contract mismatch",
                    f"{side} Primer response has no numeric score",
                    side=side,
                )
            )
        else:
            scores.append(score)
        if not isinstance(candidates, list) or not all(
            isinstance(item, str) for item in candidates
        ):
            errors.append(
                _drift_error(
                    "request contract mismatch",
                    f"{side} Primer response has invalid follow_up_queries",
                    side=side,
                )
            )
        else:
            followups.extend(candidates)
    return {
        "answers": answers,
        "scores": scores,
        "followups": followups,
    }


def _parse_response_object(
    request: ObservedRequest,
    errors: list[dict[str, Any]],
    side: str,
    stage: str,
) -> dict[str, Any]:
    if request.response_content is None:
        errors.append(
            _drift_error(
                "request contract mismatch",
                f"{side} {stage} response content was not observed",
                side=side,
            )
        )
        return {}
    _, value = try_parse_json_object(request.response_content, verbose=False)
    if not isinstance(value, dict):
        errors.append(
            _drift_error(
                "request contract mismatch",
                f"{side} {stage} response is not a JSON object",
                side=side,
            )
        )
        return {}
    return value


def _drift_operation(request: ObservedRequest) -> str:
    if request.endpoint.endswith("/embeddings"):
        return "embedding"
    content = "\n".join(_request_message_contents(request))
    if DRIFT_HYDE_PREFIX in content:
        return "hyde"
    if DRIFT_PRIMER_MARKER in content:
        return "primer"
    if DRIFT_ACTION_MARKER in content:
        return "action"
    if any(marker in content for marker in DRIFT_REDUCE_MARKERS):
        return "reduce"
    return "unknown"


def _request_message_contents(request: ObservedRequest) -> list[str]:
    messages = request.body.get("messages")
    if not isinstance(messages, list):
        return []
    return [
        message["content"]
        for message in messages
        if isinstance(message, dict) and isinstance(message.get("content"), str)
    ]


def _request_user_message(request: ObservedRequest) -> str:
    messages = request.body.get("messages")
    if not isinstance(messages, list):
        return ""
    return "\n".join(
        message["content"]
        for message in messages
        if isinstance(message, dict)
        and message.get("role") == "user"
        and isinstance(message.get("content"), str)
    )


def _embedding_inputs(request: ObservedRequest) -> list[str]:
    value = request.body.get("input")
    values = value if isinstance(value, list) else [value]
    return [item for item in values if isinstance(item, str)]


def _unique(values: list[str]) -> list[str]:
    return list(dict.fromkeys(values))


def _drift_layer_selection_is_valid(layer: dict[str, Any]) -> bool:
    selected = layer["selected"]
    return (
        len(selected) == layer["expectedSelectionCount"]
        and len(selected) == len(set(selected))
        and all(query in layer["candidates"] for query in selected)
    )


def _drift_error(
    code: str,
    message: str,
    *,
    side: str | None = None,
    depth: int | None = None,
) -> dict[str, Any]:
    error: dict[str, Any] = {"code": code, "message": message}
    if side is not None:
        error["side"] = side
    if depth is not None:
        error["depth"] = depth
    return error


def _public_drift_trace(trace: dict[str, Any]) -> dict[str, Any]:
    return {
        "primer": trace["primer"],
        "layers": trace["layers"],
        "layerCount": len(trace["layers"]),
        "actionRequestCount": len(trace["operations"]["action"]),
    }


def _pair_requests(
    graphrag: list[ObservedRequest],
    graphloom: list[ObservedRequest],
) -> tuple[
    list[tuple[ObservedRequest, ObservedRequest]],
    list[ObservedRequest],
    list[ObservedRequest],
]:
    """Pair exact requests first, then normalized equivalents for raw diffing."""
    remaining_right = list(graphloom)
    pairs: list[tuple[ObservedRequest, ObservedRequest]] = []
    remaining_left: list[ObservedRequest] = []
    for left in graphrag:
        exact_index = next(
            (
                index
                for index, right in enumerate(remaining_right)
                if right.exact_key == left.exact_key
            ),
            None,
        )
        if exact_index is None:
            remaining_left.append(left)
        else:
            pairs.append((left, remaining_right.pop(exact_index)))
    normalized_pair_count = min(len(remaining_left), len(remaining_right))
    pairs.extend(
        zip(
            remaining_left[:normalized_pair_count],
            remaining_right[:normalized_pair_count],
            strict=True,
        )
    )
    return (
        pairs,
        remaining_left[normalized_pair_count:],
        remaining_right[normalized_pair_count:],
    )


def _pair_unmatched_by_endpoint_kind(
    graphrag: list[ObservedRequest],
    graphloom: list[ObservedRequest],
) -> tuple[
    list[tuple[ObservedRequest, ObservedRequest]],
    list[ObservedRequest],
    list[ObservedRequest],
]:
    """Create diagnostic pairs without pairing embeddings with chat requests."""
    pairs: list[tuple[ObservedRequest, ObservedRequest]] = []
    left_only: list[ObservedRequest] = []
    right_only: list[ObservedRequest] = []
    for kind in ("embedding", "chat"):
        left = sorted(
            (request for request in graphrag if _endpoint_kind(request) == kind),
            key=lambda request: request.ordinal,
        )
        right = sorted(
            (request for request in graphloom if _endpoint_kind(request) == kind),
            key=lambda request: request.ordinal,
        )
        pair_count = min(len(left), len(right))
        pairs.extend(zip(left[:pair_count], right[:pair_count], strict=True))
        left_only.extend(left[pair_count:])
        right_only.extend(right[pair_count:])
    return pairs, left_only, right_only


def _endpoint_kind(request: ObservedRequest) -> str:
    return "embedding" if request.endpoint.endswith("/embeddings") else "chat"


def _append_raw_differences(
    differences: list[dict[str, Any]],
    left: ObservedRequest,
    right: ObservedRequest,
    *,
    matched: bool,
) -> None:
    for path, left_value, right_value in _value_differences(
        {"endpoint": left.endpoint, "body": left.body},
        {"endpoint": right.endpoint, "body": right.body},
        "$",
    ):
        difference = {
            "graphRagMatchKey": left.key,
            "graphLoomMatchKey": right.key,
            "graphRagOrdinal": left.ordinal,
            "graphLoomOrdinal": right.ordinal,
            "path": path,
            "graphRagPresent": left_value is not MISSING,
            "graphLoomPresent": right_value is not MISSING,
            "graphRagType": _json_type(left_value),
            "graphLoomType": _json_type(right_value),
            "affectsMatch": (
                False
                if matched
                else _unmatched_difference_affects_match(
                    path, left.endpoint, right.endpoint, left_value, right_value
                )
            ),
        }
        if isinstance(left_value, str) and isinstance(right_value, str):
            difference["firstDifference"] = _first_string_difference(
                left_value, right_value
            )
        else:
            if left_value is not MISSING:
                difference["graphRagValue"] = left_value
            if right_value is not MISSING:
                difference["graphLoomValue"] = right_value
        differences.append(difference)


def _unmatched_difference_affects_match(
    path: str,
    graph_rag_endpoint: str,
    graph_loom_endpoint: str,
    graph_rag_value: Any,
    graph_loom_value: Any,
) -> bool:
    """Identify raw fields that explain why two diagnostic pairs did not match."""
    if path == "$.endpoint" or graph_rag_endpoint != graph_loom_endpoint:
        return True
    if path == "$.body.model":
        return True
    if path == "$.body.stream":
        return (graph_rag_value is True) != (graph_loom_value is True)
    semantic_field = (
        "input"
        if graph_rag_endpoint.endswith("/embeddings")
        and graph_loom_endpoint.endswith("/embeddings")
        else "messages"
    )
    return path == f"$.body.{semantic_field}" or path.startswith(
        f"$.body.{semantic_field}["
    )


def write_observations(path: Path, observations: tuple[ObservedRequest, ...]) -> None:
    """Write a secret-free request transcript."""
    with path.open("x", encoding="utf-8", newline="\n") as output:
        for observation in observations:
            output.write(
                json.dumps(
                    asdict(observation),
                    ensure_ascii=False,
                    sort_keys=True,
                    allow_nan=False,
                )
            )
            output.write("\n")


def compare_artifact_presence(
    graph_rag_data: Path, graph_loom_data: Path
) -> dict[str, Any]:
    """Report which Parquet inputs exist on only one side."""
    graph_rag_names = sorted(path.name for path in graph_rag_data.glob("*.parquet"))
    graph_loom_names = sorted(path.name for path in graph_loom_data.glob("*.parquet"))
    graph_rag_set = set(graph_rag_names)
    graph_loom_set = set(graph_loom_names)
    return {
        "equal": graph_rag_set == graph_loom_set,
        "graphRag": graph_rag_names,
        "graphLoom": graph_loom_names,
        "onlyGraphRag": sorted(graph_rag_set - graph_loom_set),
        "onlyGraphLoom": sorted(graph_loom_set - graph_rag_set),
    }


def run_method(
    repository_root: Path,
    output_root: Path,
    graphloom_bin: Path,
    method: str,
    query: str,
    api_key: str,
) -> dict[str, Any]:
    """Run one real GraphRAG-first, GraphLoom-second comparison."""
    method_root = output_root / method
    method_root.mkdir(exist_ok=False)
    graphloom_source = repository_root / "debug"
    graphrag_source = repository_root.parent / "graphrag" / "debug"
    graphloom_settings = _load_settings(graphloom_source / "settings.yaml")
    graphrag_settings = _load_settings(graphrag_source / "settings.yaml")
    completion = _route(graphloom_settings, "completion_models")
    embedding = _route(graphloom_settings, "embedding_models")
    graphloom_drift_limits = _drift_limits(graphloom_settings)
    graphrag_drift_limits = _drift_limits(graphrag_settings)
    if method == "drift" and graphloom_drift_limits != graphrag_drift_limits:
        raise ValueError(
            "GraphRAG and GraphLoom DRIFT limits differ: "
            f"{graphrag_drift_limits} != {graphloom_drift_limits}"
        )
    engine = CachingProxyEngine(
        JsonlCache(method_root / "cache.jsonl"),
        LiteLlmBackend(completion, embedding),
    )
    server = CachingProxyServer(engine)
    server.start()
    try:
        with tempfile.TemporaryDirectory(prefix=f"graphloom-{method}-") as temp:
            temp_root = Path(temp)
            graphrag_view = temp_root / "graphrag"
            graphloom_view = temp_root / "graphloom"
            prepare_consumer_view(
                graphrag_source, graphrag_view, server.api_base, api_key
            )
            prepare_consumer_view(
                graphloom_source, graphloom_view, server.api_base, api_key
            )
            environment = _query_environment(api_key)

            left_offset = engine.observation_count()
            graphrag_result = _run_query(
                [sys.executable, "-m", "graphrag", "query"],
                graphrag_view,
                graphrag_source / "output",
                method,
                query,
                environment,
            )
            graphrag_requests = engine.observations_since(left_offset)

            right_offset = engine.observation_count()
            graphloom_result = _run_query(
                [str(graphloom_bin), "query"],
                graphloom_view,
                graphloom_source / "output",
                method,
                query,
                environment,
            )
            graphloom_requests = engine.observations_since(right_offset)
    finally:
        server.close()

    write_observations(method_root / "graphrag-requests.jsonl", graphrag_requests)
    write_observations(method_root / "graphloom-requests.jsonl", graphloom_requests)
    (method_root / "graphrag.stdout").write_text(
        graphrag_result.stdout, encoding="utf-8"
    )
    (method_root / "graphrag.stderr").write_text(
        graphrag_result.stderr, encoding="utf-8"
    )
    (method_root / "graphloom.stdout").write_text(
        graphloom_result.stdout, encoding="utf-8"
    )
    (method_root / "graphloom.stderr").write_text(
        graphloom_result.stderr, encoding="utf-8"
    )
    request_comparison = compare_requests(graphrag_requests, graphloom_requests)
    answer_equal = graphrag_result.stdout == graphloom_result.stdout
    drift_behavior = (
        compare_drift_behavior(
            graphrag_requests,
            graphloom_requests,
            query=query,
            drift_k_followups=graphloom_drift_limits[0],
            n_depth=graphloom_drift_limits[1],
        )
        if method == "drift"
        else None
    )
    expected_nondeterminism = (
        drift_behavior is not None and drift_behavior["expectedNondeterminism"]
    )
    request_compatible = (
        drift_behavior["compatible"]
        if drift_behavior is not None
        else request_comparison["matchEqual"]
    )
    answer_compatible = answer_equal or expected_nondeterminism
    report = {
        "formatVersion": 3,
        "method": method,
        "query": query,
        "graphRagExitCode": graphrag_result.returncode,
        "graphLoomExitCode": graphloom_result.returncode,
        "indexArtifactPresence": compare_artifact_presence(
            graphrag_source / "output", graphloom_source / "output"
        ),
        "requests": request_comparison,
        "answersEqual": answer_equal,
        "graphRagCacheStatuses": [
            request.cache_status for request in graphrag_requests
        ],
        "graphLoomCacheStatuses": [
            request.cache_status for request in graphloom_requests
        ],
        **({"driftBehavior": drift_behavior} if drift_behavior is not None else {}),
        "passed": (
            graphrag_result.returncode == 0
            and graphloom_result.returncode == 0
            and request_compatible
            and answer_compatible
        ),
    }
    (method_root / "report.json").write_text(
        f"{json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True)}\n",
        encoding="utf-8",
    )
    return report


def _run_query(
    executable: list[str],
    project: Path,
    data: Path,
    method: str,
    query: str,
    environment: dict[str, str],
) -> subprocess.CompletedProcess[str]:
    command = [
        *executable,
        "--root",
        str(project),
        "--data",
        str(data),
        "--method",
        method,
        query,
    ]
    try:
        return subprocess.run(
            command,
            cwd=project,
            env=environment,
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=QUERY_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as error:
        return subprocess.CompletedProcess(
            command,
            124,
            _timeout_text(error.stdout, ""),
            _timeout_text(error.stderr, "query timed out"),
        )


def _timeout_text(value: str | bytes | None, fallback: str) -> str:
    if value is None:
        return fallback
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    return value


def _query_environment(api_key: str) -> dict[str, str]:
    environment = os.environ.copy()
    environment.pop("PYTHONPATH", None)
    environment.pop("RUST_LOG", None)
    environment["PYTHONNOUSERSITE"] = "1"
    environment["GRAPHRAG_API_KEY"] = api_key
    return environment


def _load_settings(path: Path) -> dict[str, Any]:
    settings = yaml.safe_load(path.read_text(encoding="utf-8"))
    if not isinstance(settings, dict):
        raise ValueError(f"settings root must be a mapping: {path}")
    return settings


def _route(settings: dict[str, Any], section: str) -> UpstreamRoute:
    models = settings.get(section)
    if not isinstance(models, dict) or not models:
        raise ValueError(f"settings has no {section}")
    model = next(iter(models.values()))
    if not isinstance(model, dict):
        raise ValueError(f"{section} default must be a mapping")
    provider = model.get("model_provider")
    api_base = model.get("api_base")
    if not isinstance(provider, str) or not provider:
        raise ValueError(f"{section} provider must be a string")
    if api_base is not None and not isinstance(api_base, str):
        raise ValueError(f"{section} api_base must be a string")
    return UpstreamRoute(provider, api_base)


def _drift_limits(settings: dict[str, Any]) -> tuple[int, int]:
    section = settings.get("drift_search", {})
    if not isinstance(section, dict):
        raise ValueError("drift_search settings must be a mapping")
    drift_k_followups = section.get(
        "drift_k_followups",
        GRAPHRAG_3_1_DRIFT_K_FOLLOWUPS,
    )
    n_depth = section.get("n_depth", GRAPHRAG_3_1_DRIFT_N_DEPTH)
    for name, value in (
        ("drift_k_followups", drift_k_followups),
        ("n_depth", n_depth),
    ):
        if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
            raise ValueError(f"drift_search.{name} must be a positive integer")
    return drift_k_followups, n_depth


def _value_differences(
    left: Any,
    right: Any,
    path: str,
) -> list[tuple[str, Any, Any]]:
    if left is MISSING or right is MISSING:
        return [] if left is right else [(path, left, right)]
    if type(left) is not type(right):
        return [(path, left, right)]
    if isinstance(left, dict):
        differences = []
        for key in sorted(set(left) | set(right)):
            differences.extend(
                _value_differences(
                    left.get(key, MISSING),
                    right.get(key, MISSING),
                    f"{path}.{key}",
                )
            )
        return differences
    if isinstance(left, list):
        differences = []
        for index in range(max(len(left), len(right))):
            differences.extend(
                _value_differences(
                    left[index] if index < len(left) else MISSING,
                    right[index] if index < len(right) else MISSING,
                    f"{path}[{index}]",
                )
            )
        return differences
    return [] if left == right else [(path, left, right)]


def _json_type(value: Any) -> str:
    if value is MISSING:
        return "absent"
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, int):
        return "integer"
    if isinstance(value, float):
        return "number"
    if isinstance(value, str):
        return "string"
    if isinstance(value, list):
        return "array"
    if isinstance(value, dict):
        return "object"
    return type(value).__name__


def _first_string_difference(left: str, right: str) -> dict[str, int]:
    shared = 0
    for left_character, right_character in zip(left, right, strict=False):
        if left_character != right_character:
            break
        shared += 1
    prefix = left[:shared]
    last_newline = prefix.rfind("\n")
    return {
        "byte": len(prefix.encode("utf-8")),
        "line": prefix.count("\n") + 1,
        "column": shared + 1 if last_newline < 0 else shared - last_newline,
    }


def argument_parser() -> argparse.ArgumentParser:
    """Build the real comparison CLI parser."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", required=True)
    parser.add_argument("--query", required=True)
    parser.add_argument("--method", choices=(*METHODS, "all"), default="all")
    parser.add_argument("--graphloom-bin", type=Path, required=True)
    return parser


def main() -> int:
    """Run selected real Query comparisons and print bounded summaries."""
    arguments = argument_parser().parse_args()
    if CASE_PATTERN.fullmatch(arguments.case) is None:
        print("error: CASE must match [A-Za-z0-9_-]{1,64}", file=sys.stderr)
        return 2
    repository_root = Path(__file__).resolve().parents[2]
    debug_root = repository_root / "debug"
    results_root = debug_root / "query-record-replay"
    if debug_root.is_symlink() or results_root.is_symlink():
        print("error: debug result directories must not be symlinks", file=sys.stderr)
        return 2
    output_root = results_root / arguments.case
    if output_root.exists():
        print(f"error: output already exists: {output_root}", file=sys.stderr)
        return 2
    try:
        output_root.mkdir(parents=True, exist_ok=False)
        api_key = load_api_key(repository_root / "debug" / ".env")
        methods = METHODS if arguments.method == "all" else (arguments.method,)
        reports = [
            run_method(
                repository_root,
                output_root,
                arguments.graphloom_bin.resolve(strict=True),
                method,
                arguments.query,
                api_key,
            )
            for method in methods
        ]
    except (OSError, UnicodeError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    for report in reports:
        drift_summary = (
            f" drift={report['driftBehavior']['classification']}"
            if "driftBehavior" in report
            else ""
        )
        print(
            f"{report['method']}: requestsMatch={report['requests']['matchEqual']} "
            f"requestsExact={report['requests']['exactEqual']} "
            f"answersEqual={report['answersEqual']} "
            f"exitCodes={report['graphRagExitCode']}/{report['graphLoomExitCode']}"
            f"{drift_summary}"
        )
    return 0 if all(report["passed"] for report in reports) else 1


if __name__ == "__main__":
    raise SystemExit(main())
