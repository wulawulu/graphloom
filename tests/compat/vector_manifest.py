"""Version-neutral logical vector records used only by compatibility tests."""

from __future__ import annotations

import json
import math
import struct
import subprocess
from pathlib import Path
from typing import Any

import lancedb
from graphrag_vectors import (
    IndexSchema,
    VectorStoreConfig,
    VectorStoreDocument,
    create_vector_store,
)
from compat_harness import compatibility_environment

FORMAT_VERSION = 1
COLLECTION_NAMES = (
    "community_full_content",
    "entity_description",
    "text_unit_text",
)

VectorManifest = dict[str, Any]


def export_graphrag_manifest(db_uri: Path) -> VectorManifest:
    """Export GraphRAG's producer records through its installed LanceDB client."""
    connection = lancedb.connect(db_uri)
    table_names = set(connection.table_names())
    assert table_names == set(COLLECTION_NAMES)
    collections = []
    for name in COLLECTION_NAMES:
        rows = connection.open_table(name).to_arrow().select(["id", "vector"])
        records = sorted(
            (
                {
                    "id": str(row["id"]),
                    "vector": [float(value) for value in row["vector"]],
                }
                for row in rows.to_pylist()
            ),
            key=lambda record: record["id"],
        )
        dimension = len(records[0]["vector"]) if records else 0
        if records:
            results = (
                connection.open_table(name)
                .search(records[0]["vector"], vector_column_name="vector")
                .limit(min(3, len(records)))
                .to_list()
            )
            assert records[0]["id"] in {str(result["id"]) for result in results}
        collections.append({"name": name, "dimension": dimension, "records": records})
    manifest = {"format_version": FORMAT_VERSION, "collections": collections}
    validate_manifest(manifest)
    return manifest


def import_graphrag_manifest(manifest: VectorManifest, db_uri: Path) -> None:
    """Materialize producer records with GraphRAG's official vector store."""
    validate_manifest(manifest)
    _require_new_or_empty_directory(db_uri)
    for collection in manifest["collections"]:
        schema = IndexSchema(
            index_name=collection["name"],
            vector_size=collection["dimension"],
        )
        config = VectorStoreConfig(
            type="lancedb",
            db_uri=str(db_uri),
            vector_size=collection["dimension"],
        )
        store = create_vector_store(config, schema)
        store.connect()
        store.create_index()
        documents = [
            VectorStoreDocument(id=record["id"], vector=record["vector"])
            for record in collection["records"]
        ]
        store.load_documents(documents)
        assert store.count() == len(documents)
        if documents:
            probe = documents[0]
            by_id = store.search_by_id(str(probe.id))
            assert by_id.vector is not None
            assert [_float32_bytes(value) for value in by_id.vector] == [
                _float32_bytes(value) for value in probe.vector or []
            ]
            results = store.similarity_search_by_vector(
                probe.vector or [],
                k=min(3, len(documents)),
            )
            match = next(
                (result for result in results if str(result.document.id) == probe.id),
                None,
            )
            assert match is not None
            assert abs(match.score - 1.0) <= 1e-6


def export_graphloom_manifest(
    helper: Path,
    db_uri: Path,
    destination: Path,
    dimension: int,
) -> VectorManifest:
    """Export with GraphLoom's public VectorStore-backed test helper."""
    _run_helper(
        helper,
        ["export", str(db_uri), str(destination), str(dimension)],
    )
    return read_manifest(destination)


def import_graphloom_manifest(
    helper: Path,
    manifest_path: Path,
    db_uri: Path,
) -> None:
    """Import with GraphLoom's public VectorStore-backed test helper."""
    _run_helper(helper, ["import", str(manifest_path), str(db_uri)])


def write_manifest(path: Path, manifest: VectorManifest) -> None:
    """Write one stable canonical manifest."""
    validate_manifest(manifest)
    path.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


def read_manifest(path: Path) -> VectorManifest:
    """Read and validate one canonical manifest."""
    manifest = json.loads(path.read_text(encoding="utf-8"))
    validate_manifest(manifest)
    return manifest


def validate_manifest(manifest: VectorManifest) -> None:
    """Reject unsupported, incomplete, ambiguous, or unsafe logical records."""
    assert set(manifest) == {"format_version", "collections"}
    version = manifest.get("format_version")
    assert type(version) is int and version == FORMAT_VERSION
    collections = manifest.get("collections")
    assert isinstance(collections, list)
    assert [collection.get("name") for collection in collections] == list(
        COLLECTION_NAMES
    )
    for collection in collections:
        assert set(collection) == {"name", "dimension", "records"}
        dimension = collection.get("dimension")
        records = collection.get("records")
        assert type(dimension) is int and dimension > 0
        assert isinstance(records, list)
        ids = [record.get("id") for record in records]
        assert ids == sorted(ids)
        assert len(ids) == len(set(ids))
        for record in records:
            assert set(record) == {"id", "vector"}
            assert isinstance(record.get("id"), str) and record["id"]
            vector = record.get("vector")
            assert isinstance(vector, list) and len(vector) == dimension
            assert all(
                type(value) in {int, float} and math.isfinite(value) for value in vector
            )


def collection(manifest: VectorManifest, name: str) -> dict[str, Any]:
    """Return a named validated collection."""
    validate_manifest(manifest)
    return next(item for item in manifest["collections"] if item["name"] == name)


def assert_manifests_equal(
    actual: VectorManifest,
    expected: VectorManifest,
) -> None:
    """Compare IDs and vectors with exact Float32 logical semantics."""
    validate_manifest(actual)
    validate_manifest(expected)
    assert actual["format_version"] == expected["format_version"]
    for actual_collection, expected_collection in zip(
        actual["collections"],
        expected["collections"],
        strict=True,
    ):
        assert_collections_equal(actual_collection, expected_collection)


def assert_collections_equal(
    actual: dict[str, Any],
    expected: dict[str, Any],
) -> None:
    """Compare one collection with exact Float32 logical semantics."""
    assert actual["name"] == expected["name"]
    assert actual["dimension"] == expected["dimension"]
    assert len(actual["records"]) == len(expected["records"])
    for actual_record, expected_record in zip(
        actual["records"],
        expected["records"],
        strict=True,
    ):
        assert actual_record["id"] == expected_record["id"]
        assert [_float32_bytes(value) for value in actual_record["vector"]] == [
            _float32_bytes(value) for value in expected_record["vector"]
        ]


def _require_new_or_empty_directory(path: Path) -> None:
    if path.exists():
        assert path.is_dir()
        assert not any(path.iterdir())


def _run_helper(helper: Path, arguments: list[str]) -> None:
    result = subprocess.run(
        [str(helper), *arguments],
        check=False,
        capture_output=True,
        text=True,
        timeout=60,
        env=compatibility_environment(),
    )
    assert result.returncode == 0, (
        f"vector helper failed ({result.returncode})\n"
        f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
    )


def _float32_bytes(value: float) -> bytes:
    return struct.pack("<f", value)
