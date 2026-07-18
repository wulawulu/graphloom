"""Validate that compatibility tests use only the locked PyPI environment."""

from __future__ import annotations

import importlib
import importlib.metadata
import inspect
import json
import site
import sys
import sysconfig
from dataclasses import dataclass
from pathlib import Path

LOCKED_DISTRIBUTIONS = {
    "graphrag": ("graphrag", "3.1.0"),
    "graphrag_vectors": ("graphrag-vectors", "3.1.0"),
    "lancedb": ("lancedb", "0.24.3"),
}

KEY_MODULES = (
    "graphrag",
    "graphrag.cli.main",
    "graphrag.cli.query",
    "graphrag.query.structured_search.local_search",
    "graphrag.query.structured_search.global_search",
    "graphrag.query.structured_search.drift_search",
    "graphrag.query.context_builder.dynamic_community_selection",
    "graphrag_vectors",
    "lancedb",
)


@dataclass(frozen=True)
class DistributionEvidence:
    """Installed-distribution provenance for one imported top-level package."""

    distribution: str
    version: str
    package_path: Path
    installation_root: Path
    distribution_package_root: Path
    direct_url: dict[str, object]


def _is_relative_to(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
    except ValueError:
        return False
    return True


def inspect_locked_environment() -> dict[str, DistributionEvidence]:
    """Import and validate every package and key module used by compatibility tests."""
    package_distributions = importlib.metadata.packages_distributions()
    current_prefix = Path(sys.prefix).resolve()
    configured_roots = {
        Path(value).resolve()
        for value in [
            *site.getsitepackages(),
            sysconfig.get_paths().get("purelib"),
            sysconfig.get_paths().get("platlib"),
        ]
        if value
    }
    environment_roots = {
        root for root in configured_roots if _is_relative_to(root, current_prefix)
    }
    assert environment_roots, (
        f"no site-packages root belongs to active prefix {current_prefix}; "
        f"configured roots: {sorted(map(str, configured_roots))}"
    )

    module_paths = {
        name: Path(inspect.getfile(importlib.import_module(name))).resolve()
        for name in KEY_MODULES
    }
    repository_root = Path(__file__).resolve().parents[2]
    neighboring_source_root = repository_root.parent
    evidence: dict[str, DistributionEvidence] = {}

    for package, (distribution_name, expected_version) in LOCKED_DISTRIBUTIONS.items():
        mapped_distributions = package_distributions.get(package, [])
        if mapped_distributions != [distribution_name]:
            raise AssertionError(
                f"{package} maps to {mapped_distributions}, "
                f"expected [{distribution_name!r}]"
            )
        distribution = importlib.metadata.distribution(distribution_name)
        version = distribution.version
        installation_root = Path(distribution.locate_file("")).resolve()
        distribution_package_root = Path(distribution.locate_file(package)).resolve()
        package_path = module_paths[package]
        direct_url_text = distribution.read_text("direct_url.json")
        direct_url = json.loads(direct_url_text) if direct_url_text else {}
        editable = direct_url.get("dir_info", {}).get("editable", False)
        local_direct_url = str(direct_url.get("url", "")).startswith("file:")

        assert version == expected_version
        assert not editable
        assert not local_direct_url
        assert any(
            _is_relative_to(installation_root, root) for root in environment_roots
        )
        assert _is_relative_to(distribution_package_root, installation_root)
        assert _is_relative_to(package_path, distribution_package_root)
        assert any(_is_relative_to(package_path, root) for root in environment_roots)
        assert not (
            _is_relative_to(package_path, neighboring_source_root)
            and not any(
                _is_relative_to(package_path, root) for root in environment_roots
            )
        )
        evidence[package] = DistributionEvidence(
            distribution=distribution_name,
            version=version,
            package_path=package_path,
            installation_root=installation_root,
            distribution_package_root=distribution_package_root,
            direct_url=direct_url,
        )

    graphrag_root = evidence["graphrag"].distribution_package_root
    for name, path in module_paths.items():
        expected_package = (
            "graphrag" if name.startswith("graphrag.") else name.partition(".")[0]
        )
        package_root = evidence[expected_package].distribution_package_root
        if not _is_relative_to(path, package_root):
            raise AssertionError(
                f"{name} imported from {path}, "
                f"outside locked package root {package_root}"
            )
        if name.startswith("graphrag."):
            assert _is_relative_to(path, graphrag_root)

    print(format_environment_diagnostic(evidence, module_paths, environment_roots))
    return evidence


def format_environment_diagnostic(
    evidence: dict[str, DistributionEvidence],
    module_paths: dict[str, Path],
    environment_roots: set[Path],
) -> str:
    """Return stable human-readable provenance diagnostics."""
    lines = ["locked compatibility environment:"]
    for package in LOCKED_DISTRIBUTIONS:
        item = evidence[package]
        lines.extend(
            [
                (
                    f"  {package}: distribution={item.distribution} "
                    f"version={item.version}"
                ),
                f"    package={item.package_path}",
                f"    installation_root={item.installation_root}",
                f"    distribution_package_root={item.distribution_package_root}",
                f"    direct_url.json={item.direct_url}",
            ]
        )
    lines.append("key module paths:")
    lines.extend(f"  {name}: {path}" for name, path in module_paths.items())
    lines.append(f"site-packages roots: {sorted(map(str, environment_roots))}")
    return "\n".join(lines)


if __name__ == "__main__":
    inspect_locked_environment()
