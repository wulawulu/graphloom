"""Audit raw Parquet and logical vector IDs retained across an update."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

import pyarrow.parquet as pq

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "update_debug"))
from audit_update_fixture import STANDARD_TABLES, vector_inventory  # noqa: E402


def parquet_stability(initial: Path, final: Path) -> dict[str, Any]:
    """Compare raw IDs in each standard Parquet table."""
    result: dict[str, Any] = {}
    for table in sorted(STANDARD_TABLES):
        initial_rows = pq.read_table(initial / f"{table}.parquet").to_pylist()
        final_rows = pq.read_table(final / f"{table}.parquet").to_pylist()
        initial_by_id = {
            str(row["id"]): row for row in initial_rows if row.get("id") is not None
        }
        final_by_id = {
            str(row["id"]): row for row in final_rows if row.get("id") is not None
        }
        retained = sorted(set(initial_by_id) & set(final_by_id))
        changed_human_readable_ids = [
            identifier
            for identifier in retained
            if initial_by_id[identifier].get("human_readable_id")
            != final_by_id[identifier].get("human_readable_id")
        ]
        result[table] = {
            "initial": len(initial_by_id),
            "final": len(final_by_id),
            "retained": len(retained),
            "removed": len(set(initial_by_id) - set(final_by_id)),
            "added": len(set(final_by_id) - set(initial_by_id)),
            "retained_fraction": (
                len(retained) / len(initial_by_id) if initial_by_id else 1.0
            ),
            "retained_human_readable_id_changes": len(changed_human_readable_ids),
            "retained_human_readable_id_change_examples": (
                changed_human_readable_ids[:3]
            ),
        }
    return result


def vector_stability(
    initial: Path,
    final: Path,
    helper: Path,
) -> dict[str, Any]:
    """Compare logical vector IDs retained in each collection."""
    initial_vectors = vector_inventory(initial, graphloom_helper=helper)
    final_vectors = vector_inventory(final, graphloom_helper=helper)
    if initial_vectors["error"] or final_vectors["error"]:
        return {
            "initial_error": initial_vectors["error"],
            "final_error": final_vectors["error"],
        }
    result: dict[str, Any] = {}
    names = sorted(set(initial_vectors["tables"]) | set(final_vectors["tables"]))
    for name in names:
        initial_ids = {
            str(record["id"])
            for record in initial_vectors["tables"].get(name, {}).get("records", [])
        }
        final_ids = {
            str(record["id"])
            for record in final_vectors["tables"].get(name, {}).get("records", [])
        }
        result[name] = {
            "initial": len(initial_ids),
            "final": len(final_ids),
            "retained": len(initial_ids & final_ids),
            "removed": len(initial_ids - final_ids),
            "added": len(final_ids - initial_ids),
        }
    return result


def main() -> None:
    """Write one compact ID-stability report for all supplied lanes."""
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--lane",
        nargs=3,
        action="append",
        metavar=("NAME", "INITIAL_OUTPUT", "FINAL_OUTPUT"),
        required=True,
    )
    parser.add_argument("--vector-helper", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    lanes = {}
    for name, initial, final in args.lane:
        initial_path = Path(initial)
        final_path = Path(final)
        lanes[name] = {
            "parquet": parquet_stability(initial_path, final_path),
            "vectors": vector_stability(
                initial_path,
                final_path,
                args.vector_helper,
            ),
        }
    args.output.write_text(
        json.dumps({"lanes": lanes}, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
