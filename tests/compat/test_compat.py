"""GraphLoom and GraphRAG cross-language compatibility gates."""

from __future__ import annotations

import asyncio
import json
import shutil
import struct
import sys
from dataclasses import replace
from pathlib import Path

import pandas as pd
import pytest
import pyarrow as pa
import pyarrow.parquet as pq

from compat_harness import (
    STANDARD_TABLES,
    CompatibilityRun,
    RecordedRequest,
    assert_reference_integrity,
    canonical_index,
    clone_project,
    convert_prompts_for_graphrag,
    load_tables,
    replace_cache,
    request_contract,
    run_command,
    run_graphloom_index,
    run_graphloom_query,
    run_graphrag_global_query,
    run_graphrag_index,
    run_graphrag_query,
    run_graphrag_update,
    run_graphloom_update,
)
from graphrag.data_model import schemas
from graphrag.data_model.data_reader import DataReader
from graphrag.query.indexer_adapters import read_indexer_reports
from graphrag_storage import create_storage
from graphrag_storage.tables.table_provider_factory import create_table_provider
from probe_environment import DistributionEvidence
from vector_manifest import (
    assert_collections_equal,
    assert_manifests_equal,
    export_graphloom_manifest,
    export_graphloom_update_manifest,
    export_graphrag_manifest,
    export_graphrag_update_manifest,
    update_collection,
)

EXPECTED_COLUMNS = {
    "documents": schemas.DOCUMENTS_FINAL_COLUMNS,
    "text_units": schemas.TEXT_UNITS_FINAL_COLUMNS,
    "entities": schemas.ENTITIES_FINAL_COLUMNS,
    "relationships": schemas.RELATIONSHIPS_FINAL_COLUMNS,
    "covariates": schemas.COVARIATES_FINAL_COLUMNS,
    "communities": schemas.COMMUNITIES_FINAL_COLUMNS,
    "community_reports": schemas.COMMUNITY_REPORTS_FINAL_COLUMNS,
}
GRAPHRAG_UPDATE_COLUMNS = {
    name: (
        [column for column in columns if column != "description"] + ["description"]
        if name in {"entities", "relationships"}
        else columns
    )
    for name, columns in EXPECTED_COLUMNS.items()
}

LIST_COLUMNS = {
    "documents": ("text_unit_ids",),
    "text_units": ("entity_ids", "relationship_ids", "covariate_ids"),
    "entities": ("text_unit_ids",),
    "relationships": ("text_unit_ids",),
    "communities": ("children", "entity_ids", "relationship_ids", "text_unit_ids"),
    "community_reports": ("children", "findings"),
}

EXPECTED_LOGICAL_LIST_TYPES = {
    ("documents", "text_unit_ids"): "list<string>",
    ("text_units", "entity_ids"): "list<string>",
    ("text_units", "relationship_ids"): "list<string>",
    ("text_units", "covariate_ids"): "list<string>",
    ("entities", "text_unit_ids"): "list<string>",
    ("relationships", "text_unit_ids"): "list<string>",
    ("communities", "children"): "list<int64>",
    ("communities", "entity_ids"): "list<string>",
    ("communities", "relationship_ids"): "list<string>",
    ("communities", "text_unit_ids"): "list<string>",
    ("community_reports", "children"): "list<int64>",
    (
        "community_reports",
        "findings",
    ): "list<struct<explanation:string,summary:string>>",
}


def _recorded_request() -> RecordedRequest:
    return RecordedRequest(
        operation="completion",
        endpoint="/v1/chat/completions",
        model="gpt-test",
        message_roles=("system", "user"),
        system_prompt="system",
        user_message="user",
        response_format={"type": "json_object"},
        temperature=0.0,
        top_p=1.0,
        n=1,
        max_tokens=100,
        max_completion_tokens=None,
        stream=False,
        embedding_input=(),
        present_fields=frozenset(
            {
                "model",
                "messages",
                "response_format",
                "temperature",
                "max_tokens",
            }
        ),
    )


@pytest.mark.parametrize(
    ("field", "different"),
    [
        ("model", "other-model"),
        ("message_roles", ("user",)),
        ("system_prompt", "other system"),
        ("response_format", {"type": "text"}),
        ("temperature", 0.5),
        ("max_tokens", 101),
        ("present_fields", frozenset({"model", "messages"})),
    ],
)
def test_request_contract_should_detect_provider_field_difference(
    field: str,
    different: object,
) -> None:
    """Every observable provider option must participate in parity."""
    request = _recorded_request()
    assert request_contract(request) != request_contract(
        replace(request, **{field: different})
    )


def test_request_contract_should_preserve_embedding_batch_boundaries() -> None:
    """Input normalization must not merge or reorder embedding requests."""
    request = replace(
        _recorded_request(),
        operation="embedding",
        endpoint="/v1/embeddings",
        embedding_input=("first",),
    )
    split = [
        request_contract(request),
        request_contract(replace(request, embedding_input=("second",))),
    ]
    combined = [request_contract(replace(request, embedding_input=("first", "second")))]
    assert split != combined


def test_request_contract_should_only_normalize_known_community_content() -> None:
    """Markdown text alone must not trigger community-order normalization."""
    request = replace(
        _recorded_request(),
        operation="embedding",
        endpoint="/v1/embeddings",
        embedding_input=("# second", "# first"),
    )
    reversed_request = replace(
        request,
        embedding_input=tuple(reversed(request.embedding_input)),
    )
    assert request_contract(request) != request_contract(reversed_request)

    community_content = frozenset(request.embedding_input)
    assert request_contract(
        request,
        unordered_embedding_inputs=community_content,
    ) == request_contract(
        reversed_request,
        unordered_embedding_inputs=community_content,
    )


def test_should_use_locked_graphrag_distribution(
    require_isolated_python_distributions: dict[str, DistributionEvidence],
) -> None:
    """Lock the GraphRAG package and distribution provenance."""
    evidence = require_isolated_python_distributions["graphrag"]
    assert evidence.distribution == "graphrag"
    assert evidence.version == "3.1.0"


def test_should_use_locked_graphrag_vectors_distribution(
    require_isolated_python_distributions: dict[str, DistributionEvidence],
) -> None:
    """Lock the real graphrag_vectors distribution mapping and provenance."""
    evidence = require_isolated_python_distributions["graphrag_vectors"]
    assert evidence.distribution == "graphrag-vectors"
    assert evidence.version == "3.1.0"


def test_should_use_locked_lancedb_distribution(
    require_isolated_python_distributions: dict[str, DistributionEvidence],
) -> None:
    """Lock the Python LanceDB package and distribution provenance."""
    evidence = require_isolated_python_distributions["lancedb"]
    assert evidence.distribution == "lancedb"
    assert evidence.version == "0.24.3"


def test_should_ignore_host_pythonpath_in_compat_subprocess(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Prove a fresh compatibility process cannot import poisoned host packages."""
    poison = tmp_path / "poison"
    for package in ("graphrag", "graphrag_vectors"):
        package_root = poison / package
        package_root.mkdir(parents=True)
        (package_root / "__init__.py").write_text(
            f'raise RuntimeError("PYTHONPATH {package} leak")\n',
            encoding="utf-8",
        )
    (poison / "lancedb.py").write_text(
        'raise RuntimeError("PYTHONPATH lancedb leak")\n',
        encoding="utf-8",
    )
    monkeypatch.setenv("PYTHONPATH", str(poison))

    result = run_command(
        [sys.executable, str(Path(__file__).parent / "probe_environment.py")]
    )

    assert "graphrag: distribution=graphrag version=3.1.0" in result.stdout
    assert "graphrag_vectors: distribution=graphrag-vectors version=3.1.0" in (
        result.stdout
    )
    assert "lancedb: distribution=lancedb version=0.24.3" in result.stdout
    assert str(poison) not in result.stdout


def test_graphrag_3_1_report_rollup_golden() -> None:
    """Lock title grouping, level filtering, and merge order to GraphRAG 3.1.0."""
    communities = pd.DataFrame(
        {
            "id": [
                "co-1",
                "co-4",
                "co-2",
                "co-3",
                "co-9",
                "co-null",
                "co-no-title",
            ],
            "community": [1, 4, 2, 3, 9, None, 8],
            "level": [0, 1, 1, 1, 3, 1, 1],
            "title": ["Alpha", "Alpha", "Beta", "Gamma", "Alpha", "Delta", None],
            "entity_ids": [
                ["entity-x"],
                ["entity-y"],
                ["entity-x"],
                ["entity-y"],
                ["entity-z"],
                ["entity-null"],
                ["entity-no-title"],
            ],
        }
    )
    reports = pd.DataFrame(
        {
            "id": ["rp-3", "rp-1", "rp-4", "rp-2", "rp-9", "rp-neg", "rp-8"],
            "community": [3, 1, 4, 2, 9, -1, 8],
            "level": [1, 0, 1, 2, 3, 1, 1],
            "title": [
                "Report 3",
                "Report 1",
                "Report 4",
                "Report 2",
                None,
                "Report -1",
                "Report 8",
            ],
            "summary": ["S3", "S1", "S4", "S2", "S9", "SN", "S8"],
            "full_content": ["F3", "F1", "F4", "F2", "F9", "FN", "F8"],
        }
    )

    rolled_up = read_indexer_reports(reports, communities, 1)
    dynamic = read_indexer_reports(
        reports,
        communities,
        1,
        dynamic_community_selection=True,
    )

    assert [(report.id, report.community_id) for report in rolled_up] == [
        ("rp-3", "3"),
        ("rp-4", "4"),
        ("rp-neg", "-1"),
    ]
    assert [(report.id, report.community_id) for report in dynamic] == [
        ("rp-3", "3"),
        ("rp-1", "1"),
        ("rp-4", "4"),
        ("rp-neg", "-1"),
        ("rp-8", "8"),
    ]


def test_python_should_read_every_graphloom_parquet(
    compatibility_run: CompatibilityRun,
) -> None:
    """PyArrow and pandas must decode every standard GraphLoom table."""
    output = compatibility_run.graphloom_project / "output"
    reference_output = compatibility_run.graphrag_project / "output"
    tables = load_tables(output)
    reference_tables = load_tables(reference_output)
    assert set(tables) == set(STANDARD_TABLES)
    for name in STANDARD_TABLES:
        path = output / f"{name}.parquet"
        arrow_table = pq.read_table(path)
        reference_table = pq.read_table(reference_output / f"{name}.parquet")
        assert arrow_table.column_names == EXPECTED_COLUMNS[name]
        _assert_logical_arrow_schema(name, arrow_table, reference_table)
        assert len(arrow_table) > 0
        for column in LIST_COLUMNS.get(name, ()):
            data_type = arrow_table.schema.field(column).type
            if not _is_list_like(data_type):
                raise AssertionError(
                    f"{name}.{column} must remain an Arrow list, found {data_type}"
                )
            assert (
                _logical_arrow_type(data_type)
                == EXPECTED_LOGICAL_LIST_TYPES[(name, column)]
            )
    assert_reference_integrity(tables)
    assert_reference_integrity(reference_tables)
    _assert_fixture_exercises_nontrivial_paths(tables)


def test_graphrag_data_reader_should_consume_graphloom_tables(
    compatibility_run: CompatibilityRun,
) -> None:
    """Exercise GraphRAG's own typed table reader against GraphLoom output."""
    assert_graphrag_data_reader_consumes_output(compatibility_run.graphloom_project)


def test_graphloom_and_graphrag_indexes_should_be_semantically_equivalent(
    compatibility_run: CompatibilityRun,
) -> None:
    """Compare complete UUID-independent semantic records from both indexers."""
    graphloom = canonical_index(compatibility_run.graphloom_project / "output")
    graphrag = canonical_index(compatibility_run.graphrag_project / "output")
    assert graphloom == graphrag


def test_graphrag_global_search_should_query_graphloom_index(
    compatibility_run: CompatibilityRun,
) -> None:
    """Run the upstream Global Search CLI directly over GraphLoom tables."""
    # GraphLoom's managed assets are Tera templates; GraphRAG uses
    # ``str.format``. Convert only the consumer-side syntax while leaving the
    # GraphLoom index under test untouched.
    convert_prompts_for_graphrag(compatibility_run.graphloom_project)
    result = run_graphrag_global_query(
        compatibility_run.graphloom_project,
        "西门庆和武松如何通过清河县的人物网络产生联系？",
    )
    assert "Global interoperable answer." in result.stdout
    counts = compatibility_run.server.snapshot()
    assert counts["global_search_map"] >= 1
    assert counts["global_search_reduce"] >= 1


def test_graphrag_extract_graph_cache_should_feed_graphloom(
    compatibility_run: CompatibilityRun,
    tmp_path: Path,
) -> None:
    """Consume an unmodified v3.1.0 GraphRAG extraction cache in GraphLoom."""
    graphloom_consumer = tmp_path / "graphloom-consumer"
    clone_project(compatibility_run.base_project, graphloom_consumer)
    replace_cache(
        graphloom_consumer,
        compatibility_run.graphrag_project / "cache",
    )
    before = compatibility_run.server.snapshot()
    assert before["extract_graph"] >= 1
    run_graphloom_index(compatibility_run.graphloom_bin, graphloom_consumer)
    after = compatibility_run.server.snapshot()
    assert after["extract_graph"] == before["extract_graph"]
    assert canonical_index(graphloom_consumer / "output") == canonical_index(
        compatibility_run.graphloom_project / "output"
    )


def test_standard_update_should_match_graphrag_3_1(
    compatibility_run: CompatibilityRun,
    tmp_path: Path,
) -> None:
    """Compare previous, delta, final, requests, and complete vector state."""
    graphloom = tmp_path / "update-graphloom"
    graphrag = tmp_path / "update-graphrag"
    clone_project(compatibility_run.base_project, graphloom)
    clone_project(compatibility_run.base_project, graphrag)
    convert_prompts_for_graphrag(graphrag)
    input_names = sorted(path.name for path in (graphloom / "input").glob("*.txt"))
    assert len(input_names) >= 2
    delta_name = input_names[-1]
    (graphloom / "input" / delta_name).unlink()
    (graphrag / "input" / delta_name).unlink()
    run_graphloom_index(compatibility_run.graphloom_bin, graphloom)
    run_graphrag_index(graphrag)
    initial_graphloom = canonical_index(graphloom / "output")
    initial_graphrag = canonical_index(graphrag / "output")
    assert initial_graphloom == initial_graphrag
    shutil.copyfile(
        compatibility_run.base_project / "input" / delta_name,
        graphloom / "input" / delta_name,
    )
    shutil.copyfile(
        compatibility_run.base_project / "input" / delta_name,
        graphrag / "input" / delta_name,
    )

    graphloom_offset = compatibility_run.server.offset()
    graphloom_result = run_graphloom_update(compatibility_run.graphloom_bin, graphloom)
    graphloom_requests = compatibility_run.server.requests_since(graphloom_offset)
    graphrag_offset = compatibility_run.server.offset()
    run_graphrag_update(graphrag)
    graphrag_requests = compatibility_run.server.requests_since(graphrag_offset)

    assert "Update completed successfully" in graphloom_result.stdout
    assert "New documents: 1" in graphloom_result.stdout
    graphloom_timestamp = _single_update_timestamp(graphloom)
    graphrag_timestamp = _single_update_timestamp(graphrag)
    assert canonical_index(graphloom_timestamp / "previous") == initial_graphloom
    assert canonical_index(graphrag_timestamp / "previous") == initial_graphrag
    assert canonical_index(graphloom_timestamp / "previous") == canonical_index(
        graphrag_timestamp / "previous"
    )
    assert canonical_index(graphloom_timestamp / "delta") == canonical_index(
        graphrag_timestamp / "delta"
    )
    assert canonical_index(
        graphloom / "output",
        identity_sources=(graphloom_timestamp / "delta",),
    ) == canonical_index(
        graphrag / "output",
        identity_sources=(graphrag_timestamp / "delta",),
    )
    _assert_request_contracts_equal(
        graphloom_requests,
        graphrag_requests,
        graphloom,
        graphrag,
    )

    graphloom_manifest = export_graphloom_update_manifest(
        compatibility_run.vector_manifest_bin,
        graphloom / "output" / "lancedb",
        tmp_path / "update-graphloom-vectors.json",
        4,
    )
    graphrag_manifest = export_graphrag_update_manifest(graphrag / "output" / "lancedb")
    for name in ("text_unit_text", "community_full_content"):
        assert_collections_equal(
            update_collection(graphloom_manifest, name),
            update_collection(graphrag_manifest, name),
        )
    assert _update_entity_vectors_by_title(
        graphloom,
        graphloom_timestamp / "delta",
        update_collection(graphloom_manifest, "entity_description"),
    ) == _update_entity_vectors_by_title(
        graphrag,
        graphrag_timestamp / "delta",
        update_collection(graphrag_manifest, "entity_description"),
    )
    assert_updated_output_schema(graphloom, graphrag)
    assert_updated_output_schema(
        graphrag,
        graphloom,
        require_canonical_order=False,
    )
    assert_graphrag_data_reader_consumes_output(graphloom)
    _assert_graphloom_reader_consumes_output(
        compatibility_run,
        graphrag / "output",
    )
    _run_post_update_query_matrix(compatibility_run, graphloom, graphrag)


def test_noop_update_should_copy_previous_without_model_or_vector_changes(
    compatibility_run: CompatibilityRun,
    tmp_path: Path,
) -> None:
    """Lock GraphRAG's startup copy and early-stop side effects for no-op update."""
    graphloom = tmp_path / "noop-graphloom"
    graphrag = tmp_path / "noop-graphrag"
    shutil.copytree(compatibility_run.graphloom_project, graphloom)
    shutil.copytree(compatibility_run.graphrag_project, graphrag)
    graphloom_index = canonical_index(graphloom / "output")
    graphrag_index = canonical_index(graphrag / "output")
    graphloom_vectors = export_graphloom_manifest(
        compatibility_run.vector_manifest_bin,
        graphloom / "output" / "lancedb",
        tmp_path / "noop-graphloom-before.json",
        4,
    )
    graphrag_vectors = export_graphrag_manifest(graphrag / "output" / "lancedb")

    graphloom_offset = compatibility_run.server.offset()
    graphloom_result = run_graphloom_update(compatibility_run.graphloom_bin, graphloom)
    assert not compatibility_run.server.requests_since(graphloom_offset)
    graphrag_offset = compatibility_run.server.offset()
    run_graphrag_update(graphrag)
    assert not compatibility_run.server.requests_since(graphrag_offset)

    assert "New documents: 0" in graphloom_result.stdout
    graphloom_timestamp = _single_update_timestamp(graphloom)
    graphrag_timestamp = _single_update_timestamp(graphrag)
    assert canonical_index(graphloom_timestamp / "previous") == graphloom_index
    assert canonical_index(graphrag_timestamp / "previous") == graphrag_index
    assert not list((graphloom_timestamp / "delta").glob("*.parquet"))
    assert not list((graphrag_timestamp / "delta").glob("*.parquet"))
    assert canonical_index(graphloom / "output") == graphloom_index
    assert canonical_index(graphrag / "output") == graphrag_index
    assert_manifests_equal(
        export_graphloom_manifest(
            compatibility_run.vector_manifest_bin,
            graphloom / "output" / "lancedb",
            tmp_path / "noop-graphloom-after.json",
            4,
        ),
        graphloom_vectors,
    )
    assert_manifests_equal(
        export_graphrag_manifest(graphrag / "output" / "lancedb"),
        graphrag_vectors,
    )


def test_cross_producer_parquet_should_support_bidirectional_native_updates(
    compatibility_run: CompatibilityRun,
    tmp_path: Path,
) -> None:
    """Update either producer's seven Parquet tables with the other implementation."""
    graphloom_reference = tmp_path / "cross-graphloom-reference"
    graphrag_reference = tmp_path / "cross-graphrag-reference"
    graphloom_native_consumer = tmp_path / "cross-graphloom-native-consumer"
    graphrag_native_consumer = tmp_path / "cross-graphrag-native-consumer"
    graphloom_consumer = tmp_path / "cross-graphloom-consumer"
    graphrag_consumer = tmp_path / "cross-graphrag-consumer"
    for project in (
        graphloom_reference,
        graphrag_reference,
        graphloom_native_consumer,
        graphrag_native_consumer,
        graphloom_consumer,
        graphrag_consumer,
    ):
        clone_project(compatibility_run.base_project, project)
    convert_prompts_for_graphrag(graphrag_reference)
    convert_prompts_for_graphrag(graphrag_native_consumer)
    convert_prompts_for_graphrag(graphrag_consumer)

    input_names = sorted(
        path.name for path in (graphloom_reference / "input").glob("*.txt")
    )
    assert len(input_names) >= 2
    delta_name = input_names[-1]
    for project in (graphloom_reference, graphrag_reference):
        (project / "input" / delta_name).unlink()
    run_graphloom_index(compatibility_run.graphloom_bin, graphloom_reference)
    run_graphrag_index(graphrag_reference)
    initial_graphloom = canonical_index(graphloom_reference / "output")
    initial_graphrag = canonical_index(graphrag_reference / "output")
    assert initial_graphloom == initial_graphrag

    _copy_managed_parquet(graphrag_reference, graphloom_consumer)
    _copy_managed_parquet(graphloom_reference, graphrag_consumer)
    _copy_managed_parquet(graphloom_reference, graphloom_native_consumer)
    _copy_managed_parquet(graphrag_reference, graphrag_native_consumer)
    assert not (graphloom_consumer / "output" / "lancedb").exists()
    assert not (graphrag_consumer / "output" / "lancedb").exists()
    for project in (graphloom_reference, graphrag_reference):
        shutil.copyfile(
            compatibility_run.base_project / "input" / delta_name,
            project / "input" / delta_name,
        )

    run_graphloom_update(compatibility_run.graphloom_bin, graphloom_reference)
    run_graphrag_update(graphrag_reference)
    graphloom_native_consumer_offset = compatibility_run.server.offset()
    run_graphloom_update(
        compatibility_run.graphloom_bin,
        graphloom_native_consumer,
    )
    graphloom_native_consumer_requests = compatibility_run.server.requests_since(
        graphloom_native_consumer_offset
    )
    graphrag_native_consumer_offset = compatibility_run.server.offset()
    run_graphrag_update(graphrag_native_consumer)
    graphrag_native_consumer_requests = compatibility_run.server.requests_since(
        graphrag_native_consumer_offset
    )
    graphloom_consumer_offset = compatibility_run.server.offset()
    run_graphloom_update(compatibility_run.graphloom_bin, graphloom_consumer)
    graphloom_consumer_requests = compatibility_run.server.requests_since(
        graphloom_consumer_offset
    )
    graphrag_consumer_offset = compatibility_run.server.offset()
    run_graphrag_update(graphrag_consumer)
    graphrag_consumer_requests = compatibility_run.server.requests_since(
        graphrag_consumer_offset
    )
    graphloom_reference_timestamp = _single_update_timestamp(graphloom_reference)
    graphrag_reference_timestamp = _single_update_timestamp(graphrag_reference)
    graphloom_native_consumer_timestamp = _single_update_timestamp(
        graphloom_native_consumer
    )
    graphrag_native_consumer_timestamp = _single_update_timestamp(
        graphrag_native_consumer
    )
    graphloom_consumer_timestamp = _single_update_timestamp(graphloom_consumer)
    graphrag_consumer_timestamp = _single_update_timestamp(graphrag_consumer)

    assert (
        canonical_index(graphloom_consumer_timestamp / "previous") == initial_graphrag
    )
    assert (
        canonical_index(graphrag_consumer_timestamp / "previous") == initial_graphloom
    )
    assert canonical_index(graphloom_consumer_timestamp / "delta") == canonical_index(
        graphloom_reference_timestamp / "delta"
    )
    assert canonical_index(graphrag_consumer_timestamp / "delta") == canonical_index(
        graphrag_reference_timestamp / "delta"
    )
    assert canonical_index(
        graphloom_consumer / "output",
        identity_sources=(graphloom_consumer_timestamp / "delta",),
    ) == canonical_index(
        graphrag_reference / "output",
        identity_sources=(graphrag_reference_timestamp / "delta",),
    )
    assert canonical_index(
        graphrag_consumer / "output",
        identity_sources=(graphrag_consumer_timestamp / "delta",),
    ) == canonical_index(
        graphloom_reference / "output",
        identity_sources=(graphloom_reference_timestamp / "delta",),
    )
    for project, timestamp in (
        (graphloom_consumer, graphloom_consumer_timestamp),
        (graphrag_consumer, graphrag_consumer_timestamp),
    ):
        assert_reference_integrity(load_tables(timestamp / "previous"))
        _assert_old_uuid_references_preserved(
            timestamp / "previous",
            project / "output",
        )

    _assert_request_contracts_equal(
        graphloom_consumer_requests,
        graphloom_native_consumer_requests,
        graphloom_consumer,
        graphloom_native_consumer,
    )
    _assert_request_contracts_equal(
        graphrag_consumer_requests,
        graphrag_native_consumer_requests,
        graphrag_consumer,
        graphrag_native_consumer,
    )

    graphloom_consumer_manifest = export_graphloom_update_manifest(
        compatibility_run.vector_manifest_bin,
        graphloom_consumer / "output" / "lancedb",
        tmp_path / "cross-graphloom-consumer-vectors.json",
        4,
    )
    graphloom_reference_manifest = export_graphloom_update_manifest(
        compatibility_run.vector_manifest_bin,
        graphloom_native_consumer / "output" / "lancedb",
        tmp_path / "cross-graphloom-reference-vectors.json",
        4,
    )
    graphrag_consumer_manifest = export_graphrag_update_manifest(
        graphrag_consumer / "output" / "lancedb"
    )
    graphrag_reference_manifest = export_graphrag_update_manifest(
        graphrag_native_consumer / "output" / "lancedb"
    )
    for name, table, identity_column in (
        ("text_unit_text", "text_units", "human_readable_id"),
        ("community_full_content", "community_reports", "full_content"),
    ):
        assert _update_vectors_by_identity(
            graphloom_consumer,
            graphloom_consumer_timestamp / "delta",
            update_collection(graphloom_consumer_manifest, name),
            table,
            identity_column,
        ) == _update_vectors_by_identity(
            graphloom_native_consumer,
            graphloom_native_consumer_timestamp / "delta",
            update_collection(graphloom_reference_manifest, name),
            table,
            identity_column,
        )
        assert _update_vectors_by_identity(
            graphrag_consumer,
            graphrag_consumer_timestamp / "delta",
            update_collection(graphrag_consumer_manifest, name),
            table,
            identity_column,
        ) == _update_vectors_by_identity(
            graphrag_native_consumer,
            graphrag_native_consumer_timestamp / "delta",
            update_collection(graphrag_reference_manifest, name),
            table,
            identity_column,
        )
    assert _update_entity_vectors_by_title(
        graphloom_consumer,
        graphloom_consumer_timestamp / "delta",
        update_collection(graphloom_consumer_manifest, "entity_description"),
    ) == _update_entity_vectors_by_title(
        graphloom_native_consumer,
        graphloom_native_consumer_timestamp / "delta",
        update_collection(graphloom_reference_manifest, "entity_description"),
    )
    assert _update_entity_vectors_by_title(
        graphrag_consumer,
        graphrag_consumer_timestamp / "delta",
        update_collection(graphrag_consumer_manifest, "entity_description"),
    ) == _update_entity_vectors_by_title(
        graphrag_native_consumer,
        graphrag_native_consumer_timestamp / "delta",
        update_collection(graphrag_reference_manifest, "entity_description"),
    )
    assert_updated_output_schema(graphloom_consumer, graphrag_reference)
    assert_updated_output_schema(
        graphrag_consumer,
        graphloom_reference,
        require_canonical_order=False,
    )
    _assert_physical_column_order_matches(
        graphrag_consumer,
        graphrag_reference,
    )
    assert_graphrag_data_reader_consumes_output(graphloom_consumer)
    _assert_graphloom_reader_consumes_output(
        compatibility_run,
        graphrag_consumer / "output",
    )
    _run_post_update_query_matrix(
        compatibility_run,
        graphloom_consumer,
        graphrag_consumer,
    )


def _copy_managed_parquet(producer: Path, consumer: Path) -> None:
    """Copy only the seven producer tables, leaving a clean native vector store."""
    output = consumer / "output"
    output.mkdir(parents=True, exist_ok=True)
    vector_db = output / "lancedb"
    if vector_db.exists():
        shutil.rmtree(vector_db)
    for table in STANDARD_TABLES:
        shutil.copyfile(
            producer / "output" / f"{table}.parquet",
            output / f"{table}.parquet",
        )


def assert_updated_output_schema(
    project: Path,
    reference_project: Path,
    *,
    require_canonical_order: bool = True,
) -> None:
    """Validate one final update schema against GraphRAG constants and a reference."""
    output = project / "output"
    reference_output = reference_project / "output"
    for name in STANDARD_TABLES:
        table = pq.read_table(output / f"{name}.parquet")
        reference = pq.read_table(reference_output / f"{name}.parquet")
        expected_columns = EXPECTED_COLUMNS[name]
        if require_canonical_order:
            assert table.column_names == expected_columns
        else:
            # GraphRAG 3.1.0's update merge appends entity and relationship
            # descriptions at the end, even though its schema constants put
            # them earlier. Lock that pinned physical order and compare logical
            # schemas in canonical constant order without rewriting output.
            assert table.column_names == GRAPHRAG_UPDATE_COLUMNS[name]
        assert len(reference.column_names) == len(expected_columns)
        assert set(reference.column_names) == set(expected_columns)
        ordered = table.select(expected_columns)
        ordered_reference = reference.select(expected_columns)
        if require_canonical_order:
            _assert_logical_arrow_schema(name, ordered, ordered_reference)
        else:
            _assert_logical_arrow_schema(name, ordered_reference, ordered)
        for column in LIST_COLUMNS.get(name, ()):
            data_type = ordered.schema.field(column).type
            reference_type = ordered_reference.schema.field(column).type
            if not _is_list_like(data_type):
                raise AssertionError(
                    f"{name}.{column} must remain an Arrow list, found {data_type}"
                )
            if not _is_list_like(reference_type):
                raise AssertionError(
                    f"{name}.{column} reference must remain an Arrow list, "
                    f"found {reference_type}"
                )
            expected_type = EXPECTED_LOGICAL_LIST_TYPES[(name, column)]
            assert _logical_arrow_type(data_type) == expected_type
            assert _logical_arrow_type(reference_type) == expected_type


def _assert_physical_column_order_matches(
    project: Path,
    reference_project: Path,
) -> None:
    """Keep cross-producer GraphRAG update layout identical to its native update."""
    for name in STANDARD_TABLES:
        table = pq.read_table(project / "output" / f"{name}.parquet")
        reference = pq.read_table(reference_project / "output" / f"{name}.parquet")
        assert table.column_names == reference.column_names


def assert_graphrag_data_reader_consumes_output(project: Path) -> None:
    """Read every final table through GraphRAG 3.1.0's typed DataReader."""
    from graphrag.config.load_config import load_config

    config = load_config(root_dir=project)
    storage = create_storage(config.output_storage)
    reader = DataReader(create_table_provider(config.table_provider, storage=storage))

    async def read_all() -> None:
        for name in STANDARD_TABLES:
            frame = await getattr(reader, name)()
            assert list(frame.columns) == EXPECTED_COLUMNS[name]
            assert not frame.empty

    asyncio.run(read_all())


def _assert_graphloom_reader_consumes_output(
    compatibility_run: CompatibilityRun,
    output: Path,
) -> None:
    """Read every updated GraphRAG table through GraphLoom's Parquet provider."""
    result = run_command([str(compatibility_run.table_reader_bin), str(output)])
    projection = json.loads(result.stdout)
    assert set(projection) == set(STANDARD_TABLES)
    for name in STANDARD_TABLES:
        assert projection[name]["columns"] == GRAPHRAG_UPDATE_COLUMNS[name]
        assert projection[name]["rows"] > 0


def _assert_old_uuid_references_preserved(previous: Path, final: Path) -> None:
    """Lock retained producer identities without rejecting native stale delta IDs."""
    previous_tables = load_tables(previous)
    final_tables = load_tables(final)
    assert set(previous_tables["entities"]["id"].astype(str)).issubset(
        set(final_tables["entities"]["id"].astype(str))
    )
    for table, list_columns in (
        (
            "text_units",
            ("entity_ids", "relationship_ids", "covariate_ids"),
        ),
        (
            "communities",
            ("entity_ids", "relationship_ids", "text_unit_ids"),
        ),
    ):
        final_by_id = {str(row["id"]): row for _, row in final_tables[table].iterrows()}
        for _, old_row in previous_tables[table].iterrows():
            retained = final_by_id[str(old_row["id"])]
            for column in list_columns:
                assert list(retained[column]) == list(old_row[column])
    for table in STANDARD_TABLES:
        human_ids = final_tables[table]["human_readable_id"]
        assert human_ids.notna().all()
        assert not human_ids.duplicated().any()


def _single_update_timestamp(project: Path) -> Path:
    timestamps = sorted(
        path for path in (project / "update_output").iterdir() if path.is_dir()
    )
    assert len(timestamps) == 1
    assert (timestamps[0] / "previous").is_dir()
    assert (timestamps[0] / "delta").is_dir()
    return timestamps[0]


def _assert_request_contracts_equal(
    actual: tuple[RecordedRequest, ...],
    expected: tuple[RecordedRequest, ...],
    actual_project: Path,
    expected_project: Path,
) -> None:
    """Compare complete contracts with field-level diagnostics."""
    actual_community_content = _community_full_contents(actual_project)
    expected_community_content = _community_full_contents(expected_project)
    actual_contracts = [
        request_contract(
            request,
            unordered_embedding_inputs=actual_community_content,
        )
        for request in actual
    ]
    expected_contracts = [
        request_contract(
            request,
            unordered_embedding_inputs=expected_community_content,
        )
        for request in expected
    ]
    labels = (
        "operation",
        "endpoint",
        "model",
        "message_roles",
        "system_prompt",
        "user_message",
        "response_format",
        "temperature",
        "top_p",
        "n",
        "max_tokens",
        "max_completion_tokens",
        "stream",
        "present_fields",
        "embedding_input",
    )
    assert len(actual_contracts) == len(expected_contracts)
    differences = [
        (
            request_index,
            labels[field_index],
            actual_value,
            expected_value,
        )
        for request_index, (actual_contract, expected_contract) in enumerate(
            zip(actual_contracts, expected_contracts, strict=True)
        )
        for field_index, (actual_value, expected_value) in enumerate(
            zip(actual_contract, expected_contract, strict=True)
        )
        if actual_value != expected_value
    ]
    assert not differences, differences


def _community_full_contents(project: Path) -> frozenset[str]:
    """Return exact final community-report texts eligible for local reordering."""
    reports = pd.read_parquet(project / "output" / "community_reports.parquet")
    return frozenset(str(value) for value in reports["full_content"])


def _run_post_update_query_matrix(
    compatibility_run: CompatibilityRun,
    graphloom: Path,
    graphrag: Path,
) -> None:
    """Run every supported query method against both native updated indexes."""
    scenarios = [
        ("basic", False),
        ("local", False),
        ("global", False),
        ("global", True),
        ("drift", False),
    ]
    for project in (graphloom, graphrag):
        for method, dynamic in scenarios:
            offset = compatibility_run.server.offset()
            if project == graphloom:
                result = run_graphloom_query(
                    compatibility_run.graphloom_bin,
                    project,
                    project / "output",
                    method,
                    "两组人物有什么经历？",
                    streaming=False,
                    dynamic=dynamic,
                )
            else:
                result = run_graphrag_query(
                    project,
                    project / "output",
                    method,
                    "两组人物有什么经历？",
                    streaming=False,
                    dynamic=dynamic,
                )
            assert result.stdout.strip(), f"{project.name} {method} returned no answer"
            context = "\n".join(
                (
                    request.system_prompt
                    + "\n"
                    + request.user_message
                    + "\n"
                    + "\n".join(request.embedding_input)
                )
                for request in compatibility_run.server.requests_since(offset)
            )
            scenario = f"{project.name} {method} dynamic={dynamic}"
            assert "武松" in context, f"{scenario} did not use input A"
            assert "玉皇庙" in context, f"{scenario} did not use input B"


def _update_entity_vectors_by_title(
    project: Path,
    delta: Path,
    entity_collection: dict[str, object],
) -> dict[str, list[tuple[bytes, ...]]]:
    return _update_vectors_by_identity(
        project,
        delta,
        entity_collection,
        "entities",
        "title",
    )


def _update_vectors_by_identity(
    project: Path,
    delta: Path,
    vector_collection: dict[str, object],
    table_name: str,
    identity_column: str,
) -> dict[str, list[tuple[bytes, ...]]]:
    """Compare duplicate-preserving vectors through producer-neutral row identity."""
    identities: dict[str, str] = {}
    for table in (
        pd.read_parquet(project / "output" / f"{table_name}.parquet"),
        pd.read_parquet(delta / f"{table_name}.parquet"),
    ):
        identities.update(
            {str(row["id"]): str(row[identity_column]) for _, row in table.iterrows()}
        )
    vectors: dict[str, list[tuple[bytes, ...]]] = {}
    for record in vector_collection["records"]:  # type: ignore[index]
        identity = identities[str(record["id"])]
        vectors.setdefault(identity, []).append(
            tuple(struct.pack("!f", float(value)) for value in record["vector"])
        )
    for values in vectors.values():
        values.sort()
    return vectors


def _is_list_like(data_type: pa.DataType) -> bool:
    return pa.types.is_list(data_type) or pa.types.is_large_list(data_type)


def _assert_fixture_exercises_nontrivial_paths(
    tables: dict[str, pd.DataFrame],
) -> None:
    """Keep the fixture large enough to exercise chunk and community structure."""
    documents = tables["documents"]
    text_units = tables["text_units"]
    communities = tables["communities"]
    assert len(documents) == 2
    chunks_per_document = text_units.groupby("document_id").size()
    assert len(chunks_per_document) == 2
    assert chunks_per_document.min() >= 2
    assert communities["level"].max() >= 1
    assert any(len(value) > 0 for value in communities["children"])


def _assert_logical_arrow_schema(
    table_name: str,
    graphloom: pa.Table,
    graphrag: pa.Table,
) -> None:
    """Compare schema semantics while allowing Arrow offset-width differences."""
    assert graphloom.column_names == graphrag.column_names
    for graphloom_field, graphrag_field in zip(
        graphloom.schema,
        graphrag.schema,
        strict=True,
    ):
        column_name = graphloom_field.name
        graphloom_column = graphloom[column_name]
        graphrag_column = graphrag[column_name]
        assert graphloom_field.nullable == graphrag_field.nullable
        if graphloom_column.null_count != graphrag_column.null_count:
            raise AssertionError(f"{table_name}.{column_name} null count differs")
        if pa.types.is_null(graphrag_field.type):
            assert graphloom_column.null_count == len(graphloom_column)
            continue
        if _is_untyped_empty_list(graphrag_field.type):
            assert _is_list_like(graphloom_field.type)
            assert all(
                value is None or len(value) == 0
                for value in graphloom_column.to_pylist()
            )
            continue
        assert _logical_arrow_type(graphloom_field.type) == _logical_arrow_type(
            graphrag_field.type
        ), (
            f"{table_name}.{column_name} logical type differs: "
            f"{graphloom_field.type} != {graphrag_field.type}"
        )


def _logical_arrow_type(data_type: pa.DataType) -> str:
    """Collapse Arrow's 32-bit and 64-bit offset variants to one logical type."""
    if pa.types.is_string(data_type) or pa.types.is_large_string(data_type):
        return "string"
    if pa.types.is_list(data_type) or pa.types.is_large_list(data_type):
        return f"list<{_logical_arrow_type(data_type.value_type)}>"
    if pa.types.is_struct(data_type):
        fields = sorted(
            f"{field.name}:{_logical_arrow_type(field.type)}" for field in data_type
        )
        return f"struct<{','.join(fields)}>"
    return str(data_type)


def _is_untyped_empty_list(data_type: pa.DataType) -> bool:
    """Return whether pandas inferred list<null> from an all-empty list column."""
    return _is_list_like(data_type) and pa.types.is_null(data_type.value_type)
