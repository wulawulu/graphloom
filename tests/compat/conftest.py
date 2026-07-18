"""Pytest fixtures for cross-language compatibility tests."""

from __future__ import annotations

import os
from pathlib import Path

import pytest

from compat_harness import (
    CompatibilityRun,
    FixtureModelServer,
    clone_project,
    convert_prompts_for_graphrag,
    initialize_fixture_project,
    run_graphloom_index,
    run_graphrag_index,
)
from probe_environment import DistributionEvidence, inspect_locked_environment


@pytest.fixture(scope="session")
def graphloom_bin() -> Path:
    """Resolve the already-built GraphLoom binary."""
    value = os.environ.get("GRAPHLOOM_BIN")
    if not value:
        pytest.fail("GRAPHLOOM_BIN must point to the built graphloom binary")
    binary = Path(value).resolve()
    if not binary.is_file():
        pytest.fail(f"GRAPHLOOM_BIN does not exist: {binary}")
    return binary


@pytest.fixture(scope="session")
def vector_manifest_bin() -> Path:
    """Resolve the built test-only logical vector bridge."""
    value = os.environ.get("GRAPHLOOM_VECTOR_MANIFEST_BIN")
    if not value:
        pytest.fail(
            "GRAPHLOOM_VECTOR_MANIFEST_BIN must point to the built vector helper"
        )
    binary = Path(value).resolve()
    if not binary.is_file():
        pytest.fail(f"GRAPHLOOM_VECTOR_MANIFEST_BIN does not exist: {binary}")
    return binary


@pytest.fixture(scope="session", autouse=True)
def require_isolated_python_distributions() -> dict[str, DistributionEvidence]:
    """Reject non-locked, editable, local, or neighboring Python imports."""
    return inspect_locked_environment()


@pytest.fixture(scope="session")
def compatibility_run(
    tmp_path_factory: pytest.TempPathFactory,
    graphloom_bin: Path,
    vector_manifest_bin: Path,
) -> CompatibilityRun:
    """Run both indexers once against the same deterministic fixture."""
    root = tmp_path_factory.mktemp("graphloom-compat")
    server = FixtureModelServer()
    server.start()
    base_project = root / "base"
    graphloom_project = root / "graphloom"
    graphrag_project = root / "graphrag"
    try:
        initialize_fixture_project(graphloom_bin, base_project, server.api_base)
        clone_project(base_project, graphloom_project)
        clone_project(base_project, graphrag_project)
        convert_prompts_for_graphrag(graphrag_project)
        run_graphloom_index(graphloom_bin, graphloom_project)
        run_graphrag_index(graphrag_project)
        yield CompatibilityRun(
            base_project=base_project,
            graphloom_project=graphloom_project,
            graphrag_project=graphrag_project,
            graphloom_bin=graphloom_bin,
            vector_manifest_bin=vector_manifest_bin,
            server=server,
        )
    finally:
        server.close()
