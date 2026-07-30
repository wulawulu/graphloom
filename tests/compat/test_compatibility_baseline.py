"""Consistency checks for the GraphRAG 3.1.0 compatibility baseline."""

from __future__ import annotations

import re
import tomllib
from pathlib import Path
from typing import Any

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
MANIFEST_PATH = REPOSITORY_ROOT / "tests/compat/compatibility-baseline.toml"

CONTRACT_PREFIXES = {
    "compatible": "C",
    "approved_differences": "AD",
    "unsupported": "U",
    "pending_validation": "V",
}
ENGLISH_SECTIONS = {
    "## Compatible": "compatible",
    "## Approved differences": "approved_differences",
    "## Unsupported": "unsupported",
    "## Pending validation": "pending_validation",
}
CHINESE_SECTIONS = {
    "## 已兼容": "compatible",
    "## 批准差异": "approved_differences",
    "## 未支持": "unsupported",
    "## 待验证": "pending_validation",
}
CONTRACT_ID_PATTERN = re.compile(r"^\|\s*((?:AD|C|U|V)-\d+)\s*\|")
OPTIMIZATION_ID_PATTERN = re.compile(r"^\|\s*(O-\d+)\s*\|")
LOWERCASE_SHA_PATTERN = re.compile(r"[0-9a-f]{40}")


def _load_manifest() -> dict[str, Any]:
    with MANIFEST_PATH.open("rb") as manifest_file:
        return tomllib.load(manifest_file)


def _repository_path(relative_path: str) -> Path:
    return REPOSITORY_ROOT / relative_path


def _extract_contract_ids(
    path: Path,
    section_names: dict[str, str],
) -> dict[str, list[str]]:
    ids = {classification: [] for classification in CONTRACT_PREFIXES}
    current_classification: str | None = None
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("## "):
            current_classification = section_names.get(line)
            continue
        if current_classification is None:
            continue
        match = CONTRACT_ID_PATTERN.match(line)
        if match is not None:
            ids[current_classification].append(match.group(1))
    return ids


def _extract_optimization_ids(path: Path) -> list[str]:
    return [
        match.group(1)
        for line in path.read_text(encoding="utf-8").splitlines()
        if (match := OPTIMIZATION_ID_PATTERN.match(line)) is not None
    ]


def _numeric_id(identifier: str) -> int:
    return int(identifier.rsplit("-", maxsplit=1)[1])


def _make_target_recipe(makefile: str, target: str) -> str:
    lines = makefile.splitlines()
    marker = f"{target}:"
    for index, line in enumerate(lines):
        if not line.startswith(marker):
            continue
        recipe: list[str] = []
        for candidate in lines[index + 1 :]:
            if candidate.startswith("\t") or not candidate:
                recipe.append(candidate)
                continue
            break
        return "\n".join(recipe)
    raise AssertionError(f"Make target does not exist: {target}")


def test_should_validate_manifest_identity_paths_and_contract_ids() -> None:
    manifest = _load_manifest()

    assert manifest["schema_version"] == 1
    assert manifest["profile"] == "graphrag-3.1.0"
    assert manifest["baseline_tag"] == "graphrag-3.1.0-compat-v1"

    reference = manifest["reference"]
    assert reference == {
        "implementation": "microsoft/graphrag",
        "version": "3.1.0",
        "source_commit": "7fc6607edda3d387d23e52ededbf8a75b6730f97",
        "annotated_tag_object": "2077c4205add901e6594aced159fca81b7a6d522",
    }
    assert LOWERCASE_SHA_PATTERN.fullmatch(reference["source_commit"])
    assert LOWERCASE_SHA_PATTERN.fullmatch(reference["annotated_tag_object"])

    documents = manifest["documents"]
    assert set(documents) == {
        "matrix_en",
        "matrix_zh",
        "optimization_backlog_en",
        "optimization_backlog_zh",
        "test_guide_en",
        "test_guide_zh",
    }
    for relative_path in documents.values():
        assert _repository_path(relative_path).is_file(), relative_path

    gate = manifest["gate"]
    assert gate == {
        "make_target": "test-compat",
        "python_project": "tests/compat",
    }
    assert _repository_path(gate["python_project"]).is_dir()

    contract = manifest["contract"]
    assert set(contract) == set(CONTRACT_PREFIXES)
    all_ids: set[str] = set()
    for classification, prefix in CONTRACT_PREFIXES.items():
        ids = contract[classification]
        assert ids
        assert len(ids) == len(set(ids))
        assert ids == sorted(ids, key=_numeric_id)
        assert all(re.fullmatch(rf"{prefix}-\d+", identifier) for identifier in ids)
        assert all_ids.isdisjoint(ids)
        all_ids.update(ids)


def test_should_match_english_matrix_to_manifest() -> None:
    manifest = _load_manifest()
    matrix_path = _repository_path(manifest["documents"]["matrix_en"])

    assert _extract_contract_ids(matrix_path, ENGLISH_SECTIONS) == manifest["contract"]


def test_should_match_chinese_matrix_to_english_matrix() -> None:
    manifest = _load_manifest()
    documents = manifest["documents"]
    english_ids = _extract_contract_ids(
        _repository_path(documents["matrix_en"]),
        ENGLISH_SECTIONS,
    )
    chinese_ids = _extract_contract_ids(
        _repository_path(documents["matrix_zh"]),
        CHINESE_SECTIONS,
    )

    assert chinese_ids == english_ids


def test_should_keep_optimization_backlogs_linked_and_synchronized() -> None:
    manifest = _load_manifest()
    documents = manifest["documents"]
    english_path = _repository_path(documents["optimization_backlog_en"])
    chinese_path = _repository_path(documents["optimization_backlog_zh"])
    english_text = english_path.read_text(encoding="utf-8")
    chinese_text = chinese_path.read_text(encoding="utf-8")

    assert "(compatibility-matrix.md)" in english_text
    assert "(compatibility-matrix-zh.md)" in chinese_text

    english_ids = _extract_optimization_ids(english_path)
    chinese_ids = _extract_optimization_ids(chinese_path)
    assert english_ids
    assert len(english_ids) == len(set(english_ids))
    assert len(chinese_ids) == len(set(chinese_ids))
    assert chinese_ids == english_ids


def test_should_keep_readme_links_language_matched() -> None:
    english_readme = (REPOSITORY_ROOT / "README.md").read_text(encoding="utf-8")
    chinese_readme = (REPOSITORY_ROOT / "README-zh.md").read_text(encoding="utf-8")

    assert "(docs/compatibility-matrix.md)" in english_readme
    assert "(docs/compatibility-matrix-zh.md)" in chinese_readme


def test_should_collect_baseline_test_through_compatibility_gate() -> None:
    manifest = _load_manifest()
    makefile = (REPOSITORY_ROOT / "Makefile").read_text(encoding="utf-8")
    recipe = _make_target_recipe(makefile, manifest["gate"]["make_target"])

    assert re.search(r"\bpytest\s+-q\s+tests/compat\b", recipe)
