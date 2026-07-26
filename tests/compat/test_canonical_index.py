"""Focused regression tests for semantic index canonicalization."""

from __future__ import annotations

import copy
from pathlib import Path
from typing import Any

import pandas as pd
import pytest

from compat_harness import (
    assert_reference_integrity,
    assert_retained_input_ordinals_stable,
    canonical_index,
    load_tables,
)


def _documents(
    *,
    ordinals: tuple[int, int] = (0, 1),
) -> list[dict[str, Any]]:
    return [
        {
            "id": "document-a",
            "human_readable_id": ordinals[0],
            "title": "a.txt",
            "text": "Document A",
            "text_unit_ids": ["text-unit-a"],
            "raw_data": {"source": "a"},
        },
        {
            "id": "document-b",
            "human_readable_id": ordinals[1],
            "title": "b.txt",
            "text": "Document B",
            "text_unit_ids": ["text-unit-b"],
            "raw_data": {"source": "b"},
        },
    ]


def _text_units(
    *,
    ordinals: tuple[int, int] = (0, 1),
) -> list[dict[str, Any]]:
    return [
        {
            "id": "text-unit-a",
            "human_readable_id": ordinals[0],
            "text": "Chunk A",
            "n_tokens": 2,
            "document_id": "document-a",
            "entity_ids": [],
            "relationship_ids": [],
            "covariate_ids": [],
        },
        {
            "id": "text-unit-b",
            "human_readable_id": ordinals[1],
            "text": "Chunk B",
            "n_tokens": 2,
            "document_id": "document-b",
            "entity_ids": [],
            "relationship_ids": [],
            "covariate_ids": [],
        },
    ]


def _covariates(
    *,
    ordinals: tuple[int, int] = (0, 1),
) -> list[dict[str, Any]]:
    return [
        {
            "id": "covariate-a",
            "human_readable_id": ordinals[0],
            "covariate_type": "claim",
            "type": "TYPE",
            "description": "Claim A",
            "subject_id": "Subject A",
            "object_id": "Object A",
            "status": "TRUE",
            "start_date": "NONE",
            "end_date": "NONE",
            "source_text": "Source A",
            "text_unit_id": "text-unit-a",
        },
        {
            "id": "covariate-b",
            "human_readable_id": ordinals[1],
            "covariate_type": "claim",
            "type": "TYPE",
            "description": "Claim B",
            "subject_id": "Subject B",
            "object_id": "Object B",
            "status": "TRUE",
            "start_date": "NONE",
            "end_date": "NONE",
            "source_text": "Source B",
            "text_unit_id": "text-unit-b",
        },
    ]


def _write_index(
    root: Path,
    *,
    documents: list[dict[str, Any]] | None = None,
    text_units: list[dict[str, Any]] | None = None,
    covariates: list[dict[str, Any]] | None = None,
) -> None:
    root.mkdir(parents=True)
    tables = {
        "documents": pd.DataFrame(documents or _documents()),
        "text_units": pd.DataFrame(text_units or _text_units()),
        "entities": pd.DataFrame(
            columns=[
                "id",
                "human_readable_id",
                "title",
                "type",
                "description",
                "frequency",
                "degree",
                "text_unit_ids",
            ]
        ),
        "relationships": pd.DataFrame(
            columns=[
                "id",
                "human_readable_id",
                "source",
                "target",
                "description",
                "weight",
                "combined_degree",
                "text_unit_ids",
            ]
        ),
        "covariates": (
            pd.DataFrame(covariates)
            if covariates is not None
            else pd.DataFrame(
                columns=[
                    "id",
                    "human_readable_id",
                    "covariate_type",
                    "type",
                    "description",
                    "subject_id",
                    "object_id",
                    "status",
                    "start_date",
                    "end_date",
                    "source_text",
                    "text_unit_id",
                ]
            )
        ),
        "communities": pd.DataFrame(
            columns=[
                "id",
                "human_readable_id",
                "community",
                "level",
                "parent",
                "children",
                "title",
                "entity_ids",
                "relationship_ids",
                "text_unit_ids",
                "size",
            ]
        ),
        "community_reports": pd.DataFrame(
            columns=[
                "id",
                "human_readable_id",
                "community",
                "level",
                "parent",
                "children",
                "title",
                "summary",
                "full_content",
                "rank",
                "rating_explanation",
                "findings",
                "full_content_json",
                "size",
            ]
        ),
    }
    for name, frame in tables.items():
        frame.to_parquet(root / f"{name}.parquet", index=False)


def test_should_ignore_cross_producer_document_and_text_unit_ordinals(
    tmp_path: Path,
) -> None:
    left = tmp_path / "left"
    right = tmp_path / "right"
    _write_index(left)
    _write_index(
        right,
        documents=_documents(ordinals=(1, 0)),
        text_units=_text_units(ordinals=(1, 0)),
    )

    assert canonical_index(left) == canonical_index(right)


def test_should_reject_text_unit_linked_to_wrong_document(tmp_path: Path) -> None:
    left = tmp_path / "left"
    right = tmp_path / "right"
    wrong_documents = _documents(ordinals=(1, 0))
    wrong_documents[0]["text_unit_ids"] = ["text-unit-b"]
    wrong_documents[1]["text_unit_ids"] = ["text-unit-a"]
    wrong_text_units = _text_units(ordinals=(1, 0))
    wrong_text_units[0]["document_id"] = "document-b"
    wrong_text_units[1]["document_id"] = "document-a"
    _write_index(left)
    _write_index(
        right,
        documents=wrong_documents,
        text_units=wrong_text_units,
    )

    assert_reference_integrity(load_tables(right))
    assert canonical_index(left) != canonical_index(right)


@pytest.mark.parametrize(
    ("field", "value"),
    [
        ("title", "changed.txt"),
        ("text", "Changed document"),
        ("raw_data", {"source": "changed"}),
    ],
)
def test_should_reject_changed_document_semantics(
    tmp_path: Path,
    field: str,
    value: Any,
) -> None:
    left = tmp_path / "left"
    right = tmp_path / "right"
    changed_documents = copy.deepcopy(_documents())
    changed_documents[0][field] = value
    _write_index(left)
    _write_index(right, documents=changed_documents)

    assert canonical_index(left) != canonical_index(right)


@pytest.mark.parametrize(
    ("field", "value"),
    [
        ("text", "Changed chunk"),
        ("n_tokens", 3),
        ("document_id", "document-b"),
    ],
)
def test_should_reject_changed_text_unit_semantics(
    tmp_path: Path,
    field: str,
    value: Any,
) -> None:
    left = tmp_path / "left"
    right = tmp_path / "right"
    changed_text_units = copy.deepcopy(_text_units())
    changed_text_units[0][field] = value
    _write_index(left)
    _write_index(right, text_units=changed_text_units)

    assert canonical_index(left) != canonical_index(right)


def test_should_ignore_update_delta_document_ordinal(tmp_path: Path) -> None:
    left = tmp_path / "left"
    right = tmp_path / "right"
    left_documents = [_documents(ordinals=(0, 1))[0]]
    right_documents = [_documents(ordinals=(1, 0))[0]]
    left_text_units = [_text_units(ordinals=(0, 1))[0]]
    right_text_units = [_text_units(ordinals=(1, 0))[0]]
    _write_index(left, documents=left_documents, text_units=left_text_units)
    _write_index(right, documents=right_documents, text_units=right_text_units)

    assert canonical_index(left) == canonical_index(right)


def test_should_ignore_cross_producer_covariate_ordinals(tmp_path: Path) -> None:
    left = tmp_path / "left"
    right = tmp_path / "right"
    left_text_units = _text_units()
    right_text_units = _text_units(ordinals=(1, 0))
    left_text_units[0]["covariate_ids"] = ["covariate-a"]
    left_text_units[1]["covariate_ids"] = ["covariate-b"]
    right_text_units[0]["covariate_ids"] = ["covariate-a"]
    right_text_units[1]["covariate_ids"] = ["covariate-b"]
    _write_index(
        left,
        text_units=left_text_units,
        covariates=_covariates(),
    )
    _write_index(
        right,
        documents=_documents(ordinals=(1, 0)),
        text_units=right_text_units,
        covariates=_covariates(ordinals=(1, 0)),
    )

    assert_reference_integrity(load_tables(left))
    assert_reference_integrity(load_tables(right))
    assert canonical_index(left) == canonical_index(right)


def test_should_reject_covariate_linked_to_wrong_text_unit(tmp_path: Path) -> None:
    left = tmp_path / "left"
    right = tmp_path / "right"
    left_text_units = _text_units()
    left_text_units[0]["covariate_ids"] = ["covariate-a"]
    left_text_units[1]["covariate_ids"] = ["covariate-b"]
    wrong_text_units = _text_units(ordinals=(1, 0))
    wrong_text_units[0]["covariate_ids"] = ["covariate-b"]
    wrong_text_units[1]["covariate_ids"] = ["covariate-a"]
    wrong_covariates = _covariates(ordinals=(1, 0))
    wrong_covariates[0]["text_unit_id"] = "text-unit-b"
    wrong_covariates[1]["text_unit_id"] = "text-unit-a"
    _write_index(
        left,
        text_units=left_text_units,
        covariates=_covariates(),
    )
    _write_index(
        right,
        documents=_documents(ordinals=(1, 0)),
        text_units=wrong_text_units,
        covariates=wrong_covariates,
    )

    assert_reference_integrity(load_tables(right))
    assert canonical_index(left) != canonical_index(right)


def test_should_preserve_duplicate_text_unit_multiplicity(tmp_path: Path) -> None:
    duplicated = tmp_path / "duplicated"
    single = tmp_path / "single"
    duplicate_documents = [_documents()[0]]
    duplicate_documents[0]["text_unit_ids"] = ["text-unit-a", "text-unit-a-copy"]
    duplicate_text_units = [_text_units()[0]]
    duplicate_text_units.append(
        {
            **copy.deepcopy(duplicate_text_units[0]),
            "id": "text-unit-a-copy",
            "human_readable_id": 1,
        }
    )
    _write_index(
        duplicated,
        documents=duplicate_documents,
        text_units=duplicate_text_units,
    )
    _write_index(
        single,
        documents=[_documents()[0]],
        text_units=[_text_units()[0]],
    )

    canonical = canonical_index(duplicated)
    assert len(canonical["text_units"]) == 2
    assert len(canonical["documents"][0]["text_units"]) == 2
    assert canonical != canonical_index(single)


def test_should_enforce_retained_ordinals_within_one_producer(
    tmp_path: Path,
) -> None:
    previous = tmp_path / "previous"
    stable = tmp_path / "stable"
    changed = tmp_path / "changed"
    _write_index(previous)
    _write_index(stable)
    _write_index(changed, documents=_documents(ordinals=(1, 0)))

    assert_retained_input_ordinals_stable(previous, stable)
    with pytest.raises(AssertionError, match="retained documents ordinal changed"):
        assert_retained_input_ordinals_stable(previous, changed)
