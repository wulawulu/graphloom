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
    completion = _route(graphloom_settings, "completion_models")
    embedding = _route(graphloom_settings, "embedding_models")
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
    report = {
        "formatVersion": 2,
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
        "passed": (
            graphrag_result.returncode == 0
            and graphloom_result.returncode == 0
            and request_comparison["matchEqual"]
            and answer_equal
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
        print(
            f"{report['method']}: requestsMatch={report['requests']['matchEqual']} "
            f"requestsExact={report['requests']['exactEqual']} "
            f"answersEqual={report['answersEqual']} "
            f"exitCodes={report['graphRagExitCode']}/{report['graphLoomExitCode']}"
        )
    return 0 if all(report["passed"] for report in reports) else 1


if __name__ == "__main__":
    raise SystemExit(main())
