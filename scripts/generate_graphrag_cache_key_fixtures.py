#!/usr/bin/env python3
"""Generate PyYAML and GraphRAG cache-key golden fixtures from local GraphRAG."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

import yaml


def configure_imports(root: Path) -> None:
    if not root.is_dir():
        raise SystemExit(f"GraphRAG root does not exist or is not a directory: {root}")
    for package in ("graphrag", "graphrag-common", "graphrag-cache", "graphrag-llm"):
        package_root = root / "packages" / package
        if not package_root.is_dir():
            raise SystemExit(f"required GraphRAG package is missing: {package_root}")
        sys.path.insert(0, str(package_root))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--graphrag-root", type=Path, required=True)
    parser.add_argument(
        "--fixtures-root",
        type=Path,
        default=Path("crates/graphloom-llm/tests/fixtures/graphrag/cache_keys"),
    )
    args = parser.parse_args()
    root = args.graphrag_root.resolve()
    configure_imports(root)

    from graphrag.cache.cache_key_creator import cache_key_creator
    from graphrag_llm.cache.create_cache_key import _get_parameters

    cases_path = args.fixtures_root / "cases.json"
    if not cases_path.is_file():
        raise SystemExit(f"cache-key case input does not exist: {cases_path}")
    cases = json.loads(cases_path.read_text(encoding="utf-8"))
    names = [case.get("name") for case in cases]
    if any(not isinstance(name, str) or not name for name in names):
        raise SystemExit("every cache-key case must have a non-empty string name")
    duplicate_names = sorted({name for name in names if names.count(name) > 1})
    if duplicate_names:
        raise SystemExit(
            f"duplicate cache-key case names: {', '.join(duplicate_names)}"
        )
    if any(not isinstance(case.get("kwargs"), dict) for case in cases):
        raise SystemExit(
            "every cache-key case must contain an object-valued kwargs field"
        )
    expected_yaml: dict[str, str] = {}
    expected_keys: dict[str, str] = {}
    for case in sorted(cases, key=lambda item: item["name"]):
        name = case["name"]
        kwargs = case["kwargs"]
        parameters = _get_parameters(kwargs)
        serialized = yaml.dump(parameters, sort_keys=True)
        raw_controls = [
            character
            for character in serialized
            if (ord(character) < 0x20 and character != "\n")
            or 0x7F <= ord(character) <= 0x9F
        ]
        if raw_controls:
            raise SystemExit(
                "PyYAML emitted raw control characters for "
                f"{name}: serialized={serialized!r}, controls={raw_controls!r}"
            )
        decoded = yaml.safe_load(serialized)
        if decoded != parameters:
            raise SystemExit(
                "PyYAML semantic round-trip failed for "
                f"{name}: original={parameters!r}, yaml={serialized!r}, decoded={decoded!r}"
            )
        expected_yaml[name] = serialized
        expected_keys[name] = cache_key_creator(kwargs)
    if set(expected_yaml) != set(names) or set(expected_keys) != set(names):
        raise SystemExit(
            "failed to generate complete YAML and key output for every case"
        )

    commit = subprocess.run(
        ["git", "-C", str(root), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    args.fixtures_root.mkdir(parents=True, exist_ok=True)
    (args.fixtures_root / "expected.yaml.json").write_text(
        json.dumps(expected_yaml, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    (args.fixtures_root / "expected_keys.json").write_text(
        json.dumps(expected_keys, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    (args.fixtures_root / "metadata.json").write_text(
        json.dumps(
            {
                "graphrag_commit": commit,
                "pyyaml_version": yaml.__version__,
                "python_version": sys.version.split()[0],
            },
            sort_keys=True,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    print(
        f"generated {len(cases)} cases from GraphRAG {commit} / PyYAML {yaml.__version__}"
    )


if __name__ == "__main__":
    main()
