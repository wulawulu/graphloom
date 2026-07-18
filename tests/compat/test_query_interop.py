"""Bidirectional Query interoperability through logical vector records."""

from __future__ import annotations

import hashlib
import json
import struct
import subprocess
import sys
import threading
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

import pandas as pd
import pytest

from compat_harness import (
    CompatibilityRun,
    RecordedRequest,
    clone_project,
    compatibility_environment,
    configure_consumer_vector_db,
    convert_prompts_for_graphrag,
    run_graphloom_query,
    run_graphrag_query,
)
from vector_manifest import (
    COLLECTION_NAMES,
    VectorManifest,
    assert_collections_equal,
    assert_manifests_equal,
    collection,
    export_graphloom_manifest,
    export_graphrag_manifest,
    import_graphloom_manifest,
    import_graphrag_manifest,
    validate_manifest,
    write_manifest,
)

QUERY = "西门庆和武松如何通过清河县的人物网络产生联系？"
EXPECTED_ANSWERS = {
    "basic": "Basic interoperable answer.",
    "local": "Local interoperable answer.",
    "global": "Global interoperable answer.",
    "drift": "DRIFT interoperable answer.",
}

REQUEST_CONTRACT_PATH = (
    Path(__file__).parent / "fixtures" / "query" / "query_interop_request_contract.json"
)


@dataclass(frozen=True)
class NormalizedField:
    """Presence-aware JSON request field."""

    present: bool
    kind: str
    value: Any


@dataclass(frozen=True)
class NormalizedRequest:
    """Secret-free stable request projection used by the checked-in contract."""

    endpoint: str
    model: NormalizedField
    message_roles: tuple[str, ...]
    response_format: NormalizedField
    temperature: NormalizedField
    top_p: NormalizedField
    n: NormalizedField
    max_tokens: NormalizedField
    max_completion_tokens: NormalizedField
    stream: NormalizedField
    embedding_input_count: int


def _minimal_vector_manifest() -> VectorManifest:
    return {
        "format_version": 1,
        "collections": [
            {
                "name": name,
                "dimension": 1,
                "records": [{"id": f"{name}-1", "vector": [0.5]}],
            }
            for name in COLLECTION_NAMES
        ],
    }


@pytest.mark.parametrize("invalid_field", ["version", "dimension", "vector"])
def test_should_reject_boolean_manifest_numbers(invalid_field: str) -> None:
    """Keep Python validation aligned with Rust/Serde numeric types."""
    manifest = _minimal_vector_manifest()
    if invalid_field == "version":
        manifest["format_version"] = True
    elif invalid_field == "dimension":
        manifest["collections"][0]["dimension"] = True
    else:
        manifest["collections"][0]["records"][0]["vector"] = [True]

    with pytest.raises(AssertionError):
        validate_manifest(manifest)


@dataclass(frozen=True)
class InteropRun:
    """Consumer views and immutable bridge state for the Query matrix."""

    compatibility: CompatibilityRun
    graphloom_consumer: Path
    graphrag_consumer: Path
    graphloom_manifest: VectorManifest
    graphrag_manifest: VectorManifest
    graphloom_bridge: Path
    graphrag_bridge: Path


@pytest.fixture(scope="session")
def interop_run(
    compatibility_run: CompatibilityRun,
    tmp_path_factory: pytest.TempPathFactory,
) -> InteropRun:
    """Export once, import once, then reuse both producer indexes."""
    root = tmp_path_factory.mktemp("query-interop")
    helper = compatibility_run.vector_manifest_bin
    graphloom_manifest_path = root / "graphloom-vectors.json"
    graphrag_manifest_path = root / "graphrag-vectors.json"
    request_offset = compatibility_run.server.offset()

    graphloom_manifest = export_graphloom_manifest(
        helper,
        compatibility_run.graphloom_project / "output" / "lancedb",
        graphloom_manifest_path,
        4,
    )
    graphrag_manifest = export_graphrag_manifest(
        compatibility_run.graphrag_project / "output" / "lancedb"
    )
    write_manifest(graphrag_manifest_path, graphrag_manifest)

    graphloom_bridge = root / "graphloom-native-lancedb"
    graphrag_bridge = root / "graphrag-native-lancedb"
    import_graphloom_manifest(helper, graphrag_manifest_path, graphloom_bridge)
    import_graphrag_manifest(graphloom_manifest, graphrag_bridge)
    assert compatibility_run.server.requests_since(request_offset) == ()

    graphloom_roundtrip = export_graphloom_manifest(
        helper,
        graphloom_bridge,
        root / "graphloom-roundtrip.json",
        4,
    )
    graphrag_roundtrip = export_graphrag_manifest(graphrag_bridge)
    assert_manifests_equal(graphloom_roundtrip, graphrag_manifest)
    assert_manifests_equal(graphrag_roundtrip, graphloom_manifest)

    graphloom_consumer = root / "graphloom-consumer"
    graphrag_consumer = root / "graphrag-consumer"
    clone_project(compatibility_run.base_project, graphloom_consumer)
    clone_project(compatibility_run.base_project, graphrag_consumer)
    convert_prompts_for_graphrag(graphrag_consumer)
    configure_consumer_vector_db(graphloom_consumer, graphloom_bridge)
    configure_consumer_vector_db(graphrag_consumer, graphrag_bridge)

    producer_files_before = _producer_file_snapshots(compatibility_run)
    consumer_assets_before = _consumer_asset_snapshots(
        graphloom_consumer,
        graphrag_consumer,
    )
    run = InteropRun(
        compatibility=compatibility_run,
        graphloom_consumer=graphloom_consumer,
        graphrag_consumer=graphrag_consumer,
        graphloom_manifest=graphloom_manifest,
        graphrag_manifest=graphrag_manifest,
        graphloom_bridge=graphloom_bridge,
        graphrag_bridge=graphrag_bridge,
    )
    yield run

    assert _producer_file_snapshots(compatibility_run) == producer_files_before
    assert (
        _consumer_asset_snapshots(
            graphloom_consumer,
            graphrag_consumer,
        )
        == consumer_assets_before
    )
    assert not (graphloom_consumer / "cache").exists()
    assert not (graphrag_consumer / "cache").exists()
    producer_graphloom_after = export_graphloom_manifest(
        helper,
        compatibility_run.graphloom_project / "output" / "lancedb",
        root / "graphloom-producer-after.json",
        4,
    )
    producer_graphrag_after = export_graphrag_manifest(
        compatibility_run.graphrag_project / "output" / "lancedb"
    )
    graphloom_bridge_after = export_graphloom_manifest(
        helper,
        graphloom_bridge,
        root / "graphloom-bridge-after.json",
        4,
    )
    graphrag_bridge_after = export_graphrag_manifest(graphrag_bridge)
    assert_manifests_equal(producer_graphloom_after, graphloom_manifest)
    assert_manifests_equal(producer_graphrag_after, graphrag_manifest)
    assert_manifests_equal(graphloom_bridge_after, graphrag_manifest)
    assert_manifests_equal(graphrag_bridge_after, graphloom_manifest)


def test_should_preserve_vector_collection_schema_and_producer_ids(
    interop_run: InteropRun,
) -> None:
    """Lock collection names, counts, dimensions, and table foreign keys."""
    graphloom = interop_run.graphloom_manifest
    graphrag = interop_run.graphrag_manifest
    assert [item["name"] for item in graphloom["collections"]] == list(COLLECTION_NAMES)
    assert [item["name"] for item in graphrag["collections"]] == list(COLLECTION_NAMES)
    for manifest, project in (
        (graphloom, interop_run.compatibility.graphloom_project),
        (graphrag, interop_run.compatibility.graphrag_project),
    ):
        expected_ids = {
            "text_unit_text": set(
                pd.read_parquet(project / "output" / "text_units.parquet")["id"].astype(
                    str
                )
            ),
            "entity_description": set(
                pd.read_parquet(project / "output" / "entities.parquet")["id"].astype(
                    str
                )
            ),
            "community_full_content": set(
                pd.read_parquet(project / "output" / "community_reports.parquet")[
                    "id"
                ].astype(str)
            ),
        }
        for name in COLLECTION_NAMES:
            item = collection(manifest, name)
            assert item["dimension"] == 4
            assert {record["id"] for record in item["records"]} == expected_ids[name]


def test_should_generate_equivalent_vectors_for_semantically_equal_records(
    interop_run: InteropRun,
) -> None:
    """Compare deterministic vectors while accounting for generated entity UUIDs."""
    graphloom = interop_run.graphloom_manifest
    graphrag = interop_run.graphrag_manifest
    for name in ("text_unit_text", "community_full_content"):
        assert_collections_equal(
            collection(graphloom, name),
            collection(graphrag, name),
        )

    graphloom_entities = _entity_vectors_by_title(
        interop_run.compatibility.graphloom_project,
        collection(graphloom, "entity_description"),
    )
    graphrag_entities = _entity_vectors_by_title(
        interop_run.compatibility.graphrag_project,
        collection(graphrag, "entity_description"),
    )
    assert graphloom_entities == graphrag_entities


def _request_operations(method: str, dynamic: bool) -> str:
    if method == "basic":
        return "embedding+basic_search"
    if method == "local":
        return "embedding+local_search"
    if method == "global" and dynamic:
        return "dynamic_rating+global_search_map+global_search_reduce"
    if method == "global":
        return "global_search_map+global_search_reduce"
    return "drift_hyde+embedding+drift_primer+drift_action+drift_reduce"


def _interop_cases() -> list[Any]:
    cases = []
    for producer in ("graphrag", "graphloom"):
        consumer = "graphloom" if producer == "graphrag" else "graphrag"
        for method in ("basic", "local", "global", "drift"):
            for streaming in (False, True):
                cases.append(
                    pytest.param(
                        producer,
                        method,
                        False,
                        streaming,
                        id=(
                            f"producer={producer}__consumer={consumer}__method={method}"
                            f"__dynamic=false__streaming={str(streaming).lower()}"
                            f"__operations={_request_operations(method, False)}"
                        ),
                    )
                )
        for streaming in (False, True):
            cases.append(
                pytest.param(
                    producer,
                    "global",
                    True,
                    streaming,
                    id=(
                        f"producer={producer}__consumer={consumer}__method=global"
                        f"__dynamic=true__streaming={str(streaming).lower()}"
                        f"__operations={_request_operations('global', True)}"
                    ),
                )
            )
    return cases


@pytest.mark.parametrize(
    ("producer", "method", "dynamic", "streaming"),
    _interop_cases(),
)
def test_should_match_graphrag_request_contract(
    interop_run: InteropRun,
    producer: str,
    method: str,
    dynamic: bool,
    streaming: bool,
) -> None:
    """Run all 20 producer/method/streaming scenarios through real CLIs."""
    compatibility = interop_run.compatibility
    offset = compatibility.server.offset()
    if producer == "graphrag":
        result = run_graphloom_query(
            compatibility.graphloom_bin,
            interop_run.graphloom_consumer,
            compatibility.graphrag_project / "output",
            method,
            QUERY,
            streaming=streaming,
            dynamic=dynamic,
        )
    else:
        result = run_graphrag_query(
            interop_run.graphrag_consumer,
            compatibility.graphloom_project / "output",
            method,
            QUERY,
            streaming=streaming,
            dynamic=dynamic,
        )

    assert result.returncode == 0
    assert EXPECTED_ANSWERS[method] in result.stdout
    assert result.stdout.count(EXPECTED_ANSWERS[method]) == 1
    requests = compatibility.server.requests_since(offset)
    _assert_request_contract(
        requests,
        method,
        dynamic,
        streaming,
        graphloom_consumer=producer == "graphrag",
    )
    _assert_producer_context(requests, method, dynamic)
    assert "compat-test-key" not in repr(requests)
    assert "compat-test-key" not in result.stdout
    assert "compat-test-key" not in result.stderr


@pytest.mark.parametrize(
    ("consumer", "dynamic"),
    [
        ("graphloom", False),
        ("graphloom", True),
        ("graphrag", False),
        ("graphrag", True),
    ],
)
def test_should_run_global_directly_without_a_vector_bridge(
    interop_run: InteropRun,
    tmp_path: Path,
    consumer: str,
    dynamic: bool,
) -> None:
    """Prove Global and Dynamic Global never require a LanceDB connection."""
    compatibility = interop_run.compatibility
    project = tmp_path / consumer
    missing_vector_db = tmp_path / "must-not-exist"
    clone_project(compatibility.base_project, project)
    if consumer == "graphrag":
        convert_prompts_for_graphrag(project)
    configure_consumer_vector_db(project, missing_vector_db)
    offset = compatibility.server.offset()

    if consumer == "graphloom":
        result = run_graphloom_query(
            compatibility.graphloom_bin,
            project,
            compatibility.graphrag_project / "output",
            "global",
            QUERY,
            streaming=False,
            dynamic=dynamic,
        )
    else:
        result = run_graphrag_query(
            project,
            compatibility.graphloom_project / "output",
            "global",
            QUERY,
            streaming=False,
            dynamic=dynamic,
        )

    assert result.returncode == 0
    assert EXPECTED_ANSWERS["global"] in result.stdout
    assert not missing_vector_db.exists()
    requests = compatibility.server.requests_since(offset)
    _assert_request_contract(
        requests,
        "global",
        dynamic,
        False,
        graphloom_consumer=consumer == "graphloom",
    )
    _assert_producer_context(requests, "global", dynamic)


@pytest.mark.parametrize("consumer", ["graphloom", "graphrag"])
def test_should_flush_first_cross_implementation_stream_delta(
    interop_run: InteropRun,
    consumer: str,
) -> None:
    """Observe a real CLI delta before the provider releases stream completion."""
    compatibility = interop_run.compatibility
    answer = EXPECTED_ANSWERS["basic"]
    first_delta = answer[: max(1, len(answer) // 2)]
    delayed = compatibility.server.delay_next_stream()
    if consumer == "graphloom":
        command = [
            str(compatibility.graphloom_bin),
            "query",
            "--root",
            str(interop_run.graphloom_consumer),
            "--data",
            str(compatibility.graphrag_project / "output"),
            "--method",
            "basic",
            "--streaming",
            QUERY,
        ]
        cwd = interop_run.graphloom_consumer
    else:
        command = [
            sys.executable,
            "-m",
            "graphrag",
            "query",
            "--root",
            str(interop_run.graphrag_consumer),
            "--data",
            str(compatibility.graphloom_project / "output"),
            "--method",
            "basic",
            "--streaming",
            QUERY,
        ]
        cwd = interop_run.graphrag_consumer

    process = subprocess.Popen(
        command,
        cwd=cwd,
        env=compatibility_environment(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    assert process.stdout is not None
    observed = threading.Event()
    output: list[str] = []

    def read_stdout() -> None:
        while character := process.stdout.read(1):
            output.append(character)
            if first_delta in "".join(output):
                observed.set()

    reader = threading.Thread(target=read_stdout, daemon=True)
    reader.start()
    try:
        assert delayed.first_delta_sent.wait(timeout=10)
        assert observed.wait(timeout=10)
        assert process.poll() is None
        delayed.release_remaining.set()
        returncode = process.wait(timeout=30)
        reader.join(timeout=5)
        assert not reader.is_alive()
        stdout = "".join(output)
        stderr = process.stderr.read() if process.stderr is not None else ""
    finally:
        delayed.release_remaining.set()
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        reader.join(timeout=5)
    assert returncode == 0
    assert answer in stdout
    assert stdout.count(answer) == 1
    assert "compat-test-key" not in stdout
    assert "compat-test-key" not in stderr


def _normalize_json_value(value: Any) -> Any:
    if isinstance(value, dict):
        return {
            str(key): _normalize_json_value(item)
            for key, item in sorted(value.items(), key=lambda pair: str(pair[0]))
        }
    if isinstance(value, list):
        return [_normalize_json_value(item) for item in value]
    return value


def _normalized_field(request: RecordedRequest, name: str) -> NormalizedField:
    present = name in request.present_fields
    value = _normalize_json_value(getattr(request, name))
    if not present:
        kind = "absent"
    elif value is None:
        kind = "null"
    elif isinstance(value, bool):
        kind = "boolean"
    elif isinstance(value, int):
        kind = "integer"
    elif isinstance(value, float):
        kind = "number"
    elif isinstance(value, str):
        kind = "string"
    elif isinstance(value, dict):
        kind = "object"
    elif isinstance(value, list):
        kind = "array"
    else:
        raise AssertionError(f"unsupported normalized request value: {value!r}")
    return NormalizedField(
        present=present,
        kind=kind,
        value=value,
    )


def _normalize_request(request: RecordedRequest) -> dict[str, Any]:
    normalized = NormalizedRequest(
        endpoint=request.endpoint,
        model=_normalized_field(request, "model"),
        message_roles=request.message_roles,
        response_format=_normalized_field(request, "response_format"),
        temperature=_normalized_field(request, "temperature"),
        top_p=_normalized_field(request, "top_p"),
        n=_normalized_field(request, "n"),
        max_tokens=_normalized_field(request, "max_tokens"),
        max_completion_tokens=_normalized_field(request, "max_completion_tokens"),
        stream=_normalized_field(request, "stream"),
        embedding_input_count=len(request.embedding_input),
    )
    return json.loads(
        json.dumps(asdict(normalized), ensure_ascii=False, sort_keys=True)
    )


def _normalize_request_scenario(
    requests: tuple[RecordedRequest, ...],
) -> dict[str, Any]:
    operations = [request.operation for request in requests]
    normalized_operations = {}
    for operation in sorted(set(operations)):
        projections = [
            _normalize_request(request)
            for request in requests
            if request.operation == operation
        ]
        projections.sort(
            key=lambda projection: json.dumps(
                projection,
                ensure_ascii=False,
                sort_keys=True,
            )
        )
        normalized_operations[operation] = {
            "count": len(projections),
            "requests": projections,
        }
    return {
        "sequence": operations,
        "operations": normalized_operations,
        "total_count": len(requests),
    }


def _request_scenario_name(method: str, dynamic: bool, streaming: bool) -> str:
    algorithm = "dynamic_global" if method == "global" and dynamic else method
    output_mode = "stream" if streaming else "non_stream"
    return f"{algorithm}__{output_mode}"


def _load_request_contract() -> dict[str, Any]:
    contract = json.loads(REQUEST_CONTRACT_PATH.read_text(encoding="utf-8"))
    assert contract["format_version"] == 1
    assert contract["baseline"] == {
        "distribution": "graphrag",
        "version": "3.1.0",
    }
    expected_request_fields = {
        "embedding_input_count",
        "endpoint",
        "max_completion_tokens",
        "max_tokens",
        "message_roles",
        "model",
        "n",
        "response_format",
        "stream",
        "temperature",
        "top_p",
    }
    assert all(
        set(template) == expected_request_fields
        for template in contract["request_templates"].values()
    )
    return contract


def _assert_request_contract(
    requests: tuple[RecordedRequest, ...],
    method: str,
    dynamic: bool,
    streaming: bool,
    *,
    graphloom_consumer: bool,
) -> None:
    consumer = "graphloom" if graphloom_consumer else "graphrag"
    scenario = _request_scenario_name(method, dynamic, streaming)
    actual = _normalize_request_scenario(requests)
    contract = _load_request_contract()
    expected = contract["consumers"][consumer][scenario]
    assert actual["sequence"] == expected["sequence"]
    assert actual["total_count"] == expected["total_count"]
    assert set(actual["operations"]) == set(expected["operations"])
    for operation, operation_contract in expected["operations"].items():
        actual_operation = actual["operations"][operation]
        assert actual_operation["count"] == operation_contract["count"], operation
        template = contract["request_templates"][operation_contract["request_template"]]
        assert (
            actual_operation["requests"] == [template] * operation_contract["count"]
        ), operation

    embedding_requests = [
        request for request in requests if request.operation == "embedding"
    ]
    if method in {"basic", "local"}:
        assert [request.embedding_input for request in embedding_requests] == [(QUERY,)]
    elif method == "drift":
        assert [request.embedding_input for request in embedding_requests] == [
            ("A hypothetical answer connecting the fixture characters.",),
            ("How are 西门庆 and 武松 connected through 清河县?",),
        ]
    else:
        assert not embedding_requests


def _assert_producer_context(
    requests: tuple[RecordedRequest, ...],
    method: str,
    dynamic: bool,
) -> None:
    if method == "basic":
        prompts = _operation_prompts(requests, "basic_search")
        assert "Sources" in prompts
        assert "玉皇庙" in prompts
    elif method == "local":
        prompts = _operation_prompts(requests, "local_search")
        for heading in ("Reports", "Entities", "Relationships", "Sources"):
            assert heading in prompts
        assert "玉皇庙" in prompts
    elif method == "global":
        map_prompts = _operation_prompts(requests, "global_search_map")
        assert "Reports" in map_prompts
        assert "玉皇庙" in map_prompts
        if dynamic:
            rating_prompts = _operation_prompts(requests, "dynamic_rating")
            assert "relevant or helpful" in rating_prompts
            assert "玉皇庙" in rating_prompts
    else:
        primer_prompts = _operation_prompts(requests, "drift_primer")
        assert "top-ranked community summaries" in primer_prompts
        assert "知县" in primer_prompts
        action_prompts = _operation_prompts(requests, "drift_action")
        assert "Entities" in action_prompts
        assert "Sources" in action_prompts
        assert "巡捕都头" in action_prompts
        reduce_prompts = _operation_prompts(requests, "drift_reduce")
        assert (
            "Producer community reports connect the fixture characters."
            in reduce_prompts
        )
        assert "Producer entity and source evidence was retrieved." in reduce_prompts


def _operation_prompts(
    requests: tuple[RecordedRequest, ...],
    operation: str,
) -> str:
    return "\n".join(
        f"{request.system_prompt}\n{request.user_message}"
        for request in requests
        if request.operation == operation
    )


def _entity_vectors_by_title(
    project: Path,
    manifest_collection: dict[str, Any],
) -> dict[str, tuple[bytes, ...]]:
    entities = pd.read_parquet(project / "output" / "entities.parquet")
    title_by_id = dict(zip(entities["id"].astype(str), entities["title"], strict=True))
    return {
        title_by_id[record["id"]]: tuple(
            struct.pack("<f", value) for value in record["vector"]
        )
        for record in manifest_collection["records"]
    }


def _producer_file_snapshots(run: CompatibilityRun) -> dict[str, tuple[int, int, str]]:
    result = {}
    for producer, project in (
        ("graphloom", run.graphloom_project),
        ("graphrag", run.graphrag_project),
    ):
        for path in sorted((project / "output").glob("*.parquet")):
            stat = path.stat()
            result[f"{producer}/{path.name}"] = (
                stat.st_size,
                stat.st_mtime_ns,
                hashlib.sha256(path.read_bytes()).hexdigest(),
            )
    return result


def _consumer_asset_snapshots(*projects: Path) -> dict[str, str]:
    result = {}
    for project in projects:
        for path in [
            project / "settings.yaml",
            *sorted((project / "prompts").glob("*")),
        ]:
            if path.is_file():
                result[f"{project.name}/{path.relative_to(project)}"] = hashlib.sha256(
                    path.read_bytes()
                ).hexdigest()
    return result
