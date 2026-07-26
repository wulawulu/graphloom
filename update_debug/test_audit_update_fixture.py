"""Regression tests for the update fixture audit gate."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq
import yaml

REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPOSITORY_ROOT))

from update_debug.audit_update_fixture import integrity_inventory  # noqa: E402

SCRIPT = Path(__file__).with_name("audit_update_fixture.py")


def fixture(root: Path, initial: bytes, update: bytes) -> Path:
    """Create the smallest valid inventory fixture."""
    for folder in ("input", "update_batch", "cache", "artifacts"):
        (root / folder).mkdir(parents=True, exist_ok=True)
    (root / "input" / "initial.txt").write_bytes(initial)
    (root / "update_batch" / "update.txt").write_bytes(update)
    (root / "cache" / "entry").write_text("cached", encoding="utf-8")
    (root / "settings.yaml").write_text(
        yaml.safe_dump({"input": {"type": "text"}}), encoding="utf-8"
    )
    source = root / "source.txt"
    source.write_bytes(initial + update)
    return source


def run_audit(
    left: Path, right: Path, source: Path, *extra: str
) -> subprocess.CompletedProcess[str]:
    """Run the CLI exactly as the Make gate does."""
    return subprocess.run(
        [
            sys.executable,
            str(SCRIPT),
            "--graphloom-root",
            str(left),
            "--graphrag-root",
            str(right),
            "--graphloom-source",
            str(source),
            "--graphrag-source",
            str(source),
            "--output",
            str(left / "artifacts" / "audit.json"),
            *extra,
        ],
        text=True,
        capture_output=True,
        check=False,
    )


def test_inventory_gate_returns_zero_when_invariants_hold(tmp_path: Path) -> None:
    """A valid inventory-only fixture passes."""
    left = tmp_path / "left"
    right = tmp_path / "right"
    source = fixture(left, b"first", b"second")
    fixture(right, b"first", b"second")
    result = run_audit(left, right, source)
    assert result.returncode == 0, result.stdout + result.stderr
    report = json.loads((left / "artifacts" / "audit.json").read_text())
    assert "files" not in report["graphloom"]["cache"]
    assert report["graphloom"]["root"].startswith("<external>/")


def test_invariant_failure_returns_nonzero(tmp_path: Path) -> None:
    """A cross-fixture byte mismatch is a hard failure."""
    left = tmp_path / "left"
    right = tmp_path / "right"
    source = fixture(left, b"first", b"second")
    fixture(right, b"FIRST", b"second")
    result = run_audit(left, right, source)
    assert result.returncode != 0
    assert "active_input_equal" in result.stdout


def test_comparison_failure_returns_nonzero(tmp_path: Path) -> None:
    """Missing standard output tables cannot pass a comparison gate."""
    left = tmp_path / "left"
    right = tmp_path / "right"
    source = fixture(left, b"first", b"second")
    fixture(right, b"first", b"second")
    result = run_audit(left, right, source, "--gate-comparison")
    assert result.returncode != 0
    assert "missing_required_tables" in result.stdout


def test_integrity_maps_retained_delta_orphans_to_canonical_ids(
    tmp_path: Path,
) -> None:
    """Stale update UUIDs compare by observable entity/relationship identity."""
    output = tmp_path / "output"
    delta = tmp_path / "update_output" / "timestamp" / "delta"
    output.mkdir()
    delta.mkdir(parents=True)

    tables = {
        "documents": [
            {"id": "document", "text_unit_ids": ["text-unit"]},
        ],
        "text_units": [
            {
                "id": "text-unit",
                "document_id": "document",
                "entity_ids": [],
                "relationship_ids": ["delta-relationship"],
                "covariate_ids": [],
            }
        ],
        "entities": [
            {
                "id": "final-entity",
                "human_readable_id": 7,
                "title": "ENTITY",
                "text_unit_ids": [],
            }
        ],
        "relationships": [
            {
                "id": "final-relationship",
                "human_readable_id": 11,
                "source": "ENTITY",
                "target": "ENTITY",
                "text_unit_ids": [],
            }
        ],
        "communities": [
            {
                "id": "community",
                "community": 0,
                "entity_ids": ["delta-entity"],
                "relationship_ids": ["delta-relationship"],
                "text_unit_ids": [],
            }
        ],
        "community_reports": [
            {"id": "report", "community": 0},
        ],
        "covariates": [],
    }
    for name, rows in tables.items():
        table = (
            pa.Table.from_pylist(rows)
            if rows
            else pa.table({"id": pa.array([], type=pa.string())})
        )
        pq.write_table(table, output / f"{name}.parquet")
    pq.write_table(
        pa.Table.from_pylist(
            [
                {
                    "id": "delta-entity",
                    "title": "ENTITY",
                }
            ]
        ),
        delta / "entities.parquet",
    )
    pq.write_table(
        pa.Table.from_pylist(
            [
                {
                    "id": "delta-relationship",
                    "source": "ENTITY",
                    "target": "ENTITY",
                }
            ]
        ),
        delta / "relationships.parquet",
    )

    inventory = integrity_inventory(output, identity_sources=(delta,))

    assert not inventory["passed"]
    assert [
        (violation["field"], violation["examples"])
        for violation in inventory["violations"]
    ] == [
        ("relationship_ids", [11]),
        ("entity_ids", [7]),
        ("relationship_ids", [11]),
    ]
