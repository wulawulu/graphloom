"""GraphLoom and GraphRAG cross-language compatibility gates."""

from __future__ import annotations

import asyncio
from pathlib import Path

import pandas as pd
import pyarrow as pa
import pyarrow.parquet as pq

from compat_harness import (
    STANDARD_TABLES,
    CompatibilityRun,
    assert_reference_integrity,
    canonical_index,
    clone_project,
    convert_prompts_for_graphrag,
    load_tables,
    replace_cache,
    run_graphloom_index,
    run_graphrag_global_query,
)
from graphrag.data_model import schemas
from graphrag.data_model.data_reader import DataReader
from graphrag_storage import create_storage
from graphrag_storage.tables.table_provider_factory import create_table_provider

EXPECTED_COLUMNS = {
    "documents": schemas.DOCUMENTS_FINAL_COLUMNS,
    "text_units": schemas.TEXT_UNITS_FINAL_COLUMNS,
    "entities": schemas.ENTITIES_FINAL_COLUMNS,
    "relationships": schemas.RELATIONSHIPS_FINAL_COLUMNS,
    "covariates": schemas.COVARIATES_FINAL_COLUMNS,
    "communities": schemas.COMMUNITIES_FINAL_COLUMNS,
    "community_reports": schemas.COMMUNITY_REPORTS_FINAL_COLUMNS,
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
            assert _is_list_like(
                data_type
            ), f"{name}.{column} must remain an Arrow list, found {data_type}"
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
    settings = compatibility_run.graphloom_project / "settings.yaml"
    from graphrag.config.load_config import load_config

    config = load_config(root_dir=compatibility_run.graphloom_project)
    storage = create_storage(config.output_storage)
    reader = DataReader(create_table_provider(config.table_provider, storage=storage))

    async def read_all() -> None:
        for name in STANDARD_TABLES:
            frame = await getattr(reader, name)()
            assert list(frame.columns) == EXPECTED_COLUMNS[name]
            assert not frame.empty

    assert settings.is_file()
    asyncio.run(read_all())


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
    assert "西门庆与武松通过清河县人物网络间接相连" in result.stdout
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
        assert (
            graphloom_column.null_count == graphrag_column.null_count
        ), f"{table_name}.{column_name} null count differs"
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
