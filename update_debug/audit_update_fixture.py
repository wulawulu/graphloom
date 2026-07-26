#!/usr/bin/env python3
"""Gate GraphLoom/GraphRAG update fixtures and emit compact machine-readable diffs."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
from datetime import date, datetime
from pathlib import Path
from typing import Any

import pyarrow.parquet as pq
import yaml

STANDARD_TABLES = {
    "communities",
    "community_reports",
    "covariates",
    "documents",
    "entities",
    "relationships",
    "text_units",
}
REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
GRAPHRAG_ROOT = REPOSITORY_ROOT.parent / "graphrag"


def parse_args() -> argparse.Namespace:
    """Parse command-line arguments."""
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--graphloom-root",
        type=Path,
        default=REPOSITORY_ROOT / "update_debug",
    )
    parser.add_argument(
        "--graphrag-root",
        type=Path,
        default=GRAPHRAG_ROOT / "update_debug",
    )
    parser.add_argument(
        "--graphloom-source",
        type=Path,
        default=REPOSITORY_ROOT / "debug" / "input" / "金瓶梅.txt",
    )
    parser.add_argument(
        "--graphrag-source",
        type=Path,
        default=GRAPHRAG_ROOT / "debug" / "input" / "金瓶梅.txt",
    )
    parser.add_argument("--stage", default="preflight")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--gate-comparison", action="store_true")
    parser.add_argument("--effective-config-report", type=Path)
    parser.add_argument("--expected-cache-manifest")
    parser.add_argument("--graphloom-vector-helper", type=Path)
    parser.add_argument("--vector-dimension", type=int, default=1024)
    parser.add_argument("--allow-input-layout-difference", action="store_true")
    return parser.parse_args()


def hash_file(path: Path) -> str:
    """Return a streaming SHA-256 digest."""
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def cache_snapshot(root: Path) -> dict[str, Any]:
    """Create a complete, stable cache snapshot."""
    cache_root = root / "cache"
    files = [
        {
            "path": str(path.relative_to(cache_root)),
            "size": path.stat().st_size,
            "sha256": hash_file(path),
        }
        for path in sorted(cache_root.rglob("*"))
        if path.is_file()
    ]
    serialized = json.dumps(
        files,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode()
    by_namespace: dict[str, int] = {}
    for item in files:
        namespace = item["path"].split("/", maxsplit=1)[0]
        by_namespace[namespace] = by_namespace.get(namespace, 0) + 1
    return {
        "entry_count": len(files),
        "total_bytes": sum(item["size"] for item in files),
        "manifest_sha256": hashlib.sha256(serialized).hexdigest(),
        "entries_by_namespace": dict(sorted(by_namespace.items())),
        "files": files,
    }


def input_snapshot(root: Path) -> dict[str, Any]:
    """Snapshot active and staged input files."""
    result: dict[str, Any] = {}
    for folder in ("input", "update_batch"):
        base = root / folder
        result[folder] = [
            {
                "path": path.name,
                "size": path.stat().st_size,
                "sha256": hash_file(path),
            }
            for path in sorted(base.glob("*.txt"))
        ]
    return result


def verify_split(root: Path, source: Path) -> dict[str, Any]:
    """Verify that active then staged input reconstructs the source exactly."""
    active = sorted((root / "input").glob("*.txt"))
    active_names = {path.name for path in active}
    parts = [
        *active,
        *(
            path
            for path in sorted((root / "update_batch").glob("*.txt"))
            if path.name not in active_names
        ),
    ]
    digest = hashlib.sha256()
    size = 0
    for path in parts:
        content = path.read_bytes()
        digest.update(content)
        size += len(content)
    return {
        "parts": [str(path.relative_to(root)) for path in parts],
        "reassembled_size": size,
        "source_size": source.stat().st_size,
        "reassembled_sha256": digest.hexdigest(),
        "source_sha256": hash_file(source),
        "exact": size == source.stat().st_size
        and digest.hexdigest() == hash_file(source),
    }


def settings_snapshot(root: Path) -> dict[str, Any]:
    """Capture the complete declared config with credentials redacted."""
    config = yaml.safe_load((root / "settings.yaml").read_text(encoding="utf-8"))
    return redact(config)


def redact(value: Any, key: str = "") -> Any:
    """Redact secrets without discarding compatibility-relevant structure."""
    if isinstance(value, dict):
        return {item: redact(child, item) for item, child in sorted(value.items())}
    if isinstance(value, list):
        return [redact(item, key) for item in value]
    if any(marker in key.lower() for marker in ("api_key", "token", "secret")):
        return "<redacted>"
    return value


def parquet_inventory(output: Path) -> dict[str, Any]:
    """Inventory Parquet schemas and row counts without normalizing differences."""
    tables: dict[str, Any] = {}
    for path in sorted(output.glob("*.parquet")):
        parquet = pq.ParquetFile(path)
        schema = parquet.schema_arrow
        tables[path.stem] = {
            "rows": parquet.metadata.num_rows,
            "arrow_schema": str(schema),
            "schema_metadata": {
                key.hex(): value.hex()
                for key, value in sorted((schema.metadata or {}).items())
            },
            "columns": [
                {
                    "name": field.name,
                    "type": str(field.type),
                    "nullable": field.nullable,
                }
                for field in schema
            ],
            "file_size": path.stat().st_size,
            "file_sha256": hash_file(path),
        }
    return tables


def vector_inventory(
    output: Path,
    *,
    graphloom_helper: Path | None = None,
    dimension: int = 1024,
) -> dict[str, Any]:
    """Read logical LanceDB IDs and Float32 vectors, not physical file hashes."""
    root = output / "lancedb"
    if not root.exists():
        return {"tables": {}, "error": None}
    try:
        import lancedb

        compat_path = Path(__file__).resolve().parents[1] / "tests" / "compat"
        sys.path.insert(0, str(compat_path))
        if graphloom_helper:
            from vector_manifest import export_graphloom_update_manifest

            destination = output / ".audit-vector-manifest.json"
            manifest = export_graphloom_update_manifest(
                graphloom_helper, root, destination, dimension
            )
            destination.unlink(missing_ok=True)
        else:
            from vector_manifest import export_graphrag_update_manifest

            manifest = export_graphrag_update_manifest(root)
        tables = {}
        connection = lancedb.connect(root)
        for collection in manifest["collections"]:
            records = collection["records"]
            payload = json.dumps(
                records, separators=(",", ":"), sort_keys=True
            ).encode()
            tables[collection["name"]] = {
                "arrow_schema": str(connection.open_table(collection["name"]).schema),
                "rows": len(records),
                "dimension": collection["dimension"],
                "ids": [record["id"] for record in records],
                "records_sha256": hashlib.sha256(payload).hexdigest(),
                "records": records,
            }
        return {"tables": tables, "error": None}
    except Exception as error:  # noqa: BLE001 - audit must report reader failures
        return {"tables": {}, "error": f"{type(error).__name__}: {error}"}


def output_snapshot(
    root: Path,
    *,
    graphloom_helper: Path | None = None,
    dimension: int = 1024,
) -> dict[str, Any]:
    """Snapshot all supported output artifacts."""
    output = root / "output"
    return {
        "exists": output.exists(),
        "parquet": parquet_inventory(output),
        "integrity": integrity_inventory(
            output,
            identity_sources=update_identity_sources(root),
        ),
        "vectors": vector_inventory(
            output, graphloom_helper=graphloom_helper, dimension=dimension
        ),
    }


def update_identity_sources(root: Path) -> tuple[Path, ...]:
    """Return retained update delta providers in timestamp order."""
    update_output = root / "update_output"
    if not update_output.exists():
        return ()
    return tuple(
        path for path in sorted(update_output.glob("*/delta")) if path.is_dir()
    )


def integrity_inventory(
    output: Path,
    *,
    identity_sources: tuple[Path, ...] = (),
) -> dict[str, Any]:
    """Check duplicate business IDs and the standard cross-table references."""
    paths = {path.stem: path for path in output.glob("*.parquet")}
    tables = {
        name: pq.read_table(path).to_pylist()
        for name, path in paths.items()
        if name in STANDARD_TABLES
    }
    violations: list[dict[str, Any]] = []
    id_sets = {
        name: {str(row["id"]) for row in rows if row.get("id") is not None}
        for name, rows in tables.items()
    }
    reference_identities = {
        name: {
            str(row["id"]): row.get("human_readable_id", str(row["id"]))
            for row in rows
            if row.get("id") is not None
        }
        for name, rows in tables.items()
    }
    entity_by_title = {
        str(row["title"]): row.get("human_readable_id")
        for row in tables.get("entities", [])
    }
    relationship_by_endpoints = {
        (str(row["source"]), str(row["target"])): row.get("human_readable_id")
        for row in tables.get("relationships", [])
    }
    for source in identity_sources:
        entity_path = source / "entities.parquet"
        if entity_path.exists():
            reference_identities.setdefault("entities", {}).update(
                {
                    str(row["id"]): entity_by_title.get(
                        str(row["title"]), f"<missing:{row['id']}>"
                    )
                    for row in pq.read_table(entity_path).to_pylist()
                }
            )
        relationship_path = source / "relationships.parquet"
        if relationship_path.exists():
            reference_identities.setdefault("relationships", {}).update(
                {
                    str(row["id"]): relationship_by_endpoints.get(
                        (str(row["source"]), str(row["target"])),
                        f"<missing:{row['id']}>",
                    )
                    for row in pq.read_table(relationship_path).to_pylist()
                }
            )
    for name, rows in tables.items():
        ids = [str(row["id"]) for row in rows if row.get("id") is not None]
        if len(ids) != len(set(ids)):
            violations.append(
                {
                    "kind": "duplicate_id",
                    "table": name,
                    "count": len(ids) - len(set(ids)),
                }
            )

    def check(
        table: str,
        field: str,
        target: str,
        *,
        many: bool = False,
        target_values: set[str] | None = None,
    ) -> None:
        allowed = (
            target_values if target_values is not None else id_sets.get(target, set())
        )
        missing: list[str] = []
        for row in tables.get(table, []):
            value = row.get(field)
            values = value or [] if many else [value]
            for item in values:
                if item is not None and str(item) not in allowed:
                    missing.append(str(item))
        if missing:
            canonical_missing = sorted(
                (
                    reference_identities.get(target, {}).get(item, f"<missing:{item}>")
                    for item in missing
                ),
                key=lambda item: json.dumps(
                    item,
                    ensure_ascii=False,
                    sort_keys=True,
                    separators=(",", ":"),
                ),
            )
            canonical_payload = json.dumps(
                canonical_missing,
                ensure_ascii=False,
                separators=(",", ":"),
            ).encode()
            violations.append(
                {
                    "kind": "orphan_reference",
                    "table": table,
                    "field": field,
                    "target": target,
                    "count": len(missing),
                    "canonical_unique_count": len(
                        {
                            json.dumps(
                                item,
                                ensure_ascii=False,
                                sort_keys=True,
                                separators=(",", ":"),
                            )
                            for item in canonical_missing
                        }
                    ),
                    "canonical_values_sha256": hashlib.sha256(
                        canonical_payload
                    ).hexdigest(),
                    "examples": canonical_missing[:3],
                }
            )

    check("documents", "text_unit_ids", "text_units", many=True)
    check("text_units", "document_id", "documents")
    check("text_units", "entity_ids", "entities", many=True)
    check("text_units", "relationship_ids", "relationships", many=True)
    check("text_units", "covariate_ids", "covariates", many=True)
    check("entities", "text_unit_ids", "text_units", many=True)
    check("relationships", "text_unit_ids", "text_units", many=True)
    titles = {str(row.get("title")) for row in tables.get("entities", [])}
    check("relationships", "source", "entities", target_values=titles)
    check("relationships", "target", "entities", target_values=titles)
    check("communities", "entity_ids", "entities", many=True)
    check("communities", "relationship_ids", "relationships", many=True)
    check("communities", "text_unit_ids", "text_units", many=True)
    community_numbers = {
        str(row.get("community")) for row in tables.get("communities", [])
    }
    check(
        "community_reports",
        "community",
        "communities",
        target_values=community_numbers,
    )
    return {"passed": not violations, "violations": violations}


def compare_outputs(
    graphloom_root: Path,
    graphrag_root: Path,
    graphloom: dict[str, Any],
    graphrag: dict[str, Any],
) -> dict[str, Any]:
    """Compare storage strictly and semantics canonically with compact diffs."""
    graphloom_tables = set(graphloom["parquet"])
    graphrag_tables = set(graphrag["parquet"])
    common = sorted(graphloom_tables & graphrag_tables)
    table_diffs = {}
    for name in common:
        left_path = graphloom_root / "output" / f"{name}.parquet"
        right_path = graphrag_root / "output" / f"{name}.parquet"
        left_records = stable_records(pq.read_table(left_path).to_pylist())
        right_records = stable_records(pq.read_table(right_path).to_pylist())
        left_schema = {
            "arrow": graphloom["parquet"][name]["arrow_schema"],
            "metadata": graphloom["parquet"][name]["schema_metadata"],
        }
        right_schema = {
            "arrow": graphrag["parquet"][name]["arrow_schema"],
            "metadata": graphrag["parquet"][name]["schema_metadata"],
        }
        table_diffs[name] = {
            "schema_equal": left_schema == right_schema,
            "rows": {
                "graphloom": len(left_records),
                "graphrag": len(right_records),
            },
            "records_equal": left_records == right_records,
            "record_diff": compact_record_diff(left_records, right_records),
        }
    left_vectors = graphloom["vectors"]
    right_vectors = graphrag["vectors"]
    vector_names = sorted(set(left_vectors["tables"]) | set(right_vectors["tables"]))
    vector_diffs = {}
    for name in vector_names:
        left = left_vectors["tables"].get(name)
        right = right_vectors["tables"].get(name)
        vector_diffs[name] = compare_vector_table(left, right)
    semantic_vectors = compare_semantic_vectors(
        graphloom_root, graphrag_root, left_vectors, right_vectors
    )
    result: dict[str, Any] = {
        "table_sets_equal": graphloom_tables == graphrag_tables,
        "graphloom_only_tables": sorted(graphloom_tables - graphrag_tables),
        "graphrag_only_tables": sorted(graphrag_tables - graphloom_tables),
        "required_missing": {
            "graphloom": sorted(STANDARD_TABLES - graphloom_tables),
            "graphrag": sorted(STANDARD_TABLES - graphrag_tables),
        },
        "common_table_diffs": table_diffs,
        "strict_equal": (
            graphloom_tables == graphrag_tables
            and all(
                item["schema_equal"] and item["records_equal"]
                for item in table_diffs.values()
            )
        ),
        "vectors": {
            "graphloom_error": left_vectors["error"],
            "graphrag_error": right_vectors["error"],
            "table_sets_equal": set(left_vectors["tables"])
            == set(right_vectors["tables"]),
            "tables": vector_diffs,
            "equal": (
                left_vectors["error"] is None
                and right_vectors["error"] is None
                and set(left_vectors["tables"]) == set(right_vectors["tables"])
                and all(item["equal"] for item in vector_diffs.values())
            ),
            "semantic": semantic_vectors,
        },
        "semantic_comparison": {
            "performed": False,
            "reason": "both outputs must contain all seven standard tables",
        },
    }
    if STANDARD_TABLES <= graphloom_tables and STANDARD_TABLES <= graphrag_tables:
        compat_path = Path(__file__).resolve().parents[1] / "tests" / "compat"
        sys.path.insert(0, str(compat_path))
        from compat_harness import (
            canonical_index,
        )  # pylint: disable=import-outside-toplevel

        try:
            graphloom_index = canonical_index(
                graphloom_root / "output",
                identity_sources=update_identity_sources(graphloom_root),
            )
            graphrag_index = canonical_index(
                graphrag_root / "output",
                identity_sources=update_identity_sources(graphrag_root),
            )
            semantic_diffs = {
                name: compact_record_diff(
                    stable_records(graphloom_index[name]),
                    stable_records(graphrag_index[name]),
                )
                for name in sorted(STANDARD_TABLES)
            }
            result["semantic_comparison"] = {
                "performed": True,
                "equal": graphloom_index == graphrag_index,
                "tables": semantic_diffs,
            }
        except Exception as error:  # noqa: BLE001 - comparison failure is evidence
            result["semantic_comparison"] = {
                "performed": False,
                "equal": False,
                "reason": f"{type(error).__name__}: {error}",
            }
    return result


def json_value(value: Any) -> Any:
    """Convert Arrow/Pandas values to stable JSON-compatible values."""
    if isinstance(value, (bytes, bytearray)):
        return {"bytes_hex": bytes(value).hex()}
    if isinstance(value, (datetime, date)):
        return value.isoformat()
    if isinstance(value, float):
        if math.isnan(value):
            return {"float": "nan"}
        if math.isinf(value):
            return {"float": "inf" if value > 0 else "-inf"}
        return value
    if isinstance(value, dict):
        return {str(key): json_value(child) for key, child in sorted(value.items())}
    if isinstance(value, (list, tuple)):
        return [json_value(child) for child in value]
    return value


def stable_records(records: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Sort records without changing semantically meaningful list order."""
    converted = [json_value(record) for record in records]
    return sorted(
        converted,
        key=lambda record: json.dumps(
            record, ensure_ascii=False, sort_keys=True, separators=(",", ":")
        ),
    )


def compact_record_diff(
    left: list[dict[str, Any]], right: list[dict[str, Any]]
) -> dict[str, Any]:
    """Return counts, hashes, and field-level differences without record payloads."""
    encode = lambda rows: json.dumps(  # noqa: E731 - compact stable encoder
        rows, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode()
    left_serialized = [encode([row]) for row in left]
    right_serialized = [encode([row]) for row in right]
    left_set = set(left_serialized)
    right_set = set(right_serialized)
    field_mismatches: dict[str, int] = {}
    for left_row, right_row in zip(left, right, strict=False):
        for field in set(left_row) | set(right_row):
            if left_row.get(field) != right_row.get(field):
                field_mismatches[field] = field_mismatches.get(field, 0) + 1
    return {
        "equal": left == right,
        "graphloom_count": len(left),
        "graphrag_count": len(right),
        "graphloom_sha256": hashlib.sha256(encode(left)).hexdigest(),
        "graphrag_sha256": hashlib.sha256(encode(right)).hexdigest(),
        "graphloom_only_count": len(left_set - right_set),
        "graphrag_only_count": len(right_set - left_set),
        "field_mismatch_counts": dict(sorted(field_mismatches.items())),
    }


def compare_vector_table(left: Any, right: Any) -> dict[str, Any]:
    """Compare one logical vector collection exactly at Float32 precision."""
    if left is None or right is None:
        return {"equal": False, "missing": "graphloom" if left is None else "graphrag"}
    left_records = left["records"]
    right_records = right["records"]
    mismatch = []
    for index, (left_row, right_row) in enumerate(
        zip(left_records, right_records, strict=False)
    ):
        if left_row != right_row and len(mismatch) < 3:
            mismatch.append(
                {
                    "index": index,
                    "graphloom_id": left_row["id"],
                    "graphrag_id": right_row["id"],
                    "max_abs_delta": (
                        max(
                            abs(a - b)
                            for a, b in zip(
                                left_row["vector"], right_row["vector"], strict=False
                            )
                        )
                        if left_row["vector"] and right_row["vector"]
                        else None
                    ),
                }
            )
    equal = (
        left["arrow_schema"] == right["arrow_schema"]
        and left["dimension"] == right["dimension"]
        and left["ids"] == right["ids"]
        and left["records_sha256"] == right["records_sha256"]
    )
    return {
        "equal": equal,
        "schema_equal": left["arrow_schema"] == right["arrow_schema"],
        "arrow_schema": {
            "graphloom": left["arrow_schema"],
            "graphrag": right["arrow_schema"],
        },
        "dimension": {
            "graphloom": left["dimension"],
            "graphrag": right["dimension"],
        },
        "rows": {"graphloom": left["rows"], "graphrag": right["rows"]},
        "ids_equal": left["ids"] == right["ids"],
        "vectors_sha256": {
            "graphloom": left["records_sha256"],
            "graphrag": right["records_sha256"],
        },
        "mismatch_examples": mismatch,
    }


def compare_semantic_vectors(
    graphloom_root: Path,
    graphrag_root: Path,
    left: dict[str, Any],
    right: dict[str, Any],
    tolerance: float = 1e-7,
) -> dict[str, Any]:
    """Map random entity UUIDs by title and compare vectors with a strict tolerance."""
    if left["error"] or right["error"]:
        return {"equal": False, "reason": "logical vector read failed"}
    if not left["tables"] and not right["tables"]:
        return {"equal": True, "tables": {}}
    left_entities = pq.read_table(
        graphloom_root / "output" / "entities.parquet"
    ).to_pylist()
    right_entities = pq.read_table(
        graphrag_root / "output" / "entities.parquet"
    ).to_pylist()
    mappings = {
        "graphloom": {str(row["id"]): str(row["title"]) for row in left_entities},
        "graphrag": {str(row["id"]): str(row["title"]) for row in right_entities},
    }
    results = {}
    for name in sorted(set(left["tables"]) | set(right["tables"])):
        left_table = left["tables"].get(name)
        right_table = right["tables"].get(name)
        if left_table is None or right_table is None:
            results[name] = {"equal": False, "reason": "missing collection"}
            continue

        def keyed(table: dict[str, Any], side: str) -> dict[str, list[float]]:
            return {
                (
                    mappings[side].get(record["id"], record["id"])
                    if name == "entity_description"
                    else record["id"]
                ): record["vector"]
                for record in table["records"]
            }

        left_records = keyed(left_table, "graphloom")
        right_records = keyed(right_table, "graphrag")
        common = sorted(set(left_records) & set(right_records))
        max_delta = max(
            (
                abs(a - b)
                for key in common
                for a, b in zip(left_records[key], right_records[key], strict=True)
            ),
            default=0.0,
        )
        equal = (
            set(left_records) == set(right_records)
            and left_table["dimension"] == right_table["dimension"]
            and max_delta <= tolerance
        )
        results[name] = {
            "equal": equal,
            "ids_equal_after_mapping": set(left_records) == set(right_records),
            "rows": {"graphloom": len(left_records), "graphrag": len(right_records)},
            "dimension": left_table["dimension"],
            "max_abs_delta": max_delta,
            "tolerance": tolerance,
        }
    return {"equal": all(item["equal"] for item in results.values()), "tables": results}


def main() -> int:
    """Run the audit and write one machine-readable report."""
    args = parse_args()
    graphloom_root = args.graphloom_root.resolve()
    graphrag_root = args.graphrag_root.resolve()
    graphloom_output = output_snapshot(
        graphloom_root,
        graphloom_helper=(
            args.graphloom_vector_helper.resolve()
            if args.graphloom_vector_helper
            else None
        ),
        dimension=args.vector_dimension,
    )
    graphrag_output = output_snapshot(
        graphrag_root,
        dimension=args.vector_dimension,
    )
    report = {
        "stage": args.stage,
        "graphloom": {
            "root": str(graphloom_root),
            "settings": settings_snapshot(graphloom_root),
            "input": input_snapshot(graphloom_root),
            "split": verify_split(graphloom_root, args.graphloom_source.resolve()),
            "cache": cache_snapshot(graphloom_root),
            "output": graphloom_output,
        },
        "graphrag": {
            "root": str(graphrag_root),
            "settings": settings_snapshot(graphrag_root),
            "input": input_snapshot(graphrag_root),
            "split": verify_split(graphrag_root, args.graphrag_source.resolve()),
            "cache": cache_snapshot(graphrag_root),
            "output": graphrag_output,
        },
    }
    report["cross_fixture"] = {
        "source_sha256_equal": report["graphloom"]["split"]["source_sha256"]
        == report["graphrag"]["split"]["source_sha256"],
        "active_input_equal": report["graphloom"]["input"]["input"]
        == report["graphrag"]["input"]["input"],
        "staged_input_equal": report["graphloom"]["input"]["update_batch"]
        == report["graphrag"]["input"]["update_batch"],
        "cache_manifest_equal": report["graphloom"]["cache"]["manifest_sha256"]
        == report["graphrag"]["cache"]["manifest_sha256"],
        "outputs": compare_outputs(
            graphloom_root,
            graphrag_root,
            graphloom_output,
            graphrag_output,
        ),
    }
    for side in ("graphloom", "graphrag"):
        report[side]["root"] = portable_path(
            graphloom_root if side == "graphloom" else graphrag_root
        )
        report[side]["cache"].pop("files", None)
        for table in report[side]["output"]["vectors"]["tables"].values():
            table.pop("records", None)
    failures = []
    differences = []
    for side in ("graphloom", "graphrag"):
        if not report[side]["split"]["exact"]:
            failures.append({"kind": "split_not_exact", "side": side})
    for name in ("source_sha256_equal", "active_input_equal", "staged_input_equal"):
        if not report["cross_fixture"][name] and not (
            args.allow_input_layout_difference
            and name in {"active_input_equal", "staged_input_equal"}
        ):
            failures.append({"kind": name})
    if args.expected_cache_manifest:
        for side in ("graphloom", "graphrag"):
            actual = report[side]["cache"]["manifest_sha256"]
            if actual != args.expected_cache_manifest:
                failures.append(
                    {
                        "kind": "cache_manifest_changed",
                        "side": side,
                        "expected": args.expected_cache_manifest,
                        "actual": actual,
                    }
                )
    elif not report["cross_fixture"]["cache_manifest_equal"]:
        failures.append({"kind": "cache_manifest_mismatch"})
    if args.effective_config_report:
        effective = json.loads(args.effective_config_report.read_text(encoding="utf-8"))
        report["effective_config"] = effective
        if not effective.get("compatible"):
            failures.append({"kind": "effective_config_mismatch"})
    if args.gate_comparison:
        outputs = report["cross_fixture"]["outputs"]
        left_integrity = report["graphloom"]["output"]["integrity"]
        right_integrity = report["graphrag"]["output"]["integrity"]
        if left_integrity != right_integrity:
            failures.append(
                {
                    "kind": "reference_integrity_mismatch",
                    "graphloom": left_integrity,
                    "graphrag": right_integrity,
                }
            )
        elif not left_integrity["passed"]:
            differences.append(
                {
                    "kind": "shared_reference_integrity_violations",
                    "violations": left_integrity["violations"],
                }
            )
        if (
            outputs["required_missing"]["graphloom"]
            or outputs["required_missing"]["graphrag"]
        ):
            failures.append(
                {"kind": "missing_required_tables", **outputs["required_missing"]}
            )
        if not outputs["strict_equal"]:
            differences.append({"kind": "strict_storage_mismatch"})
        if not outputs["semantic_comparison"].get("equal", False):
            failures.append({"kind": "semantic_mismatch"})
        if not outputs["vectors"]["equal"]:
            differences.append({"kind": "strict_vector_store_mismatch"})
        if not outputs["vectors"]["semantic"]["equal"]:
            failures.append({"kind": "semantic_vector_store_mismatch"})
    report["gate"] = {
        "passed": not failures,
        "failures": failures,
        "reported_differences": differences,
    }
    destination = (
        args.output or graphloom_root / "artifacts" / f"{args.stage}-audit.json"
    )
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    summary = {
        "report": str(destination),
        "graphloom_cache_entries": report["graphloom"]["cache"]["entry_count"],
        "graphrag_cache_entries": report["graphrag"]["cache"]["entry_count"],
        "cache_manifest_equal": report["cross_fixture"]["cache_manifest_equal"],
        "splits_exact": report["graphloom"]["split"]["exact"]
        and report["graphrag"]["split"]["exact"],
        "table_sets_equal": report["cross_fixture"]["outputs"]["table_sets_equal"],
        "gate_passed": not failures,
        "failures": failures,
        "reported_differences": differences,
    }
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0 if not failures else 1


def portable_path(path: Path) -> str:
    """Render repository-adjacent paths without embedding a machine home path."""
    try:
        return str(path.relative_to(REPOSITORY_ROOT))
    except ValueError:
        try:
            return str(Path("..") / path.relative_to(REPOSITORY_ROOT.parent))
        except ValueError:
            return f"<external>/{path.name}"


if __name__ == "__main__":
    raise SystemExit(main())
