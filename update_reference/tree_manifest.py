#!/usr/bin/env python3
"""Write or compare byte-level manifests for fixture directory trees."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path


def hash_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def manifest(root: Path) -> dict[str, object]:
    files = [
        {
            "path": str(path.relative_to(root)),
            "size": path.stat().st_size,
            "sha256": hash_file(path),
        }
        for path in sorted(root.rglob("*"))
        if path.is_file()
    ]
    encoded = json.dumps(
        files, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode()
    return {
        "root": portable_path(root),
        "file_count": len(files),
        "total_bytes": sum(int(item["size"]) for item in files),
        "manifest_sha256": hashlib.sha256(encoded).hexdigest(),
        "files": files,
    }


def portable_path(path: Path) -> str:
    """Avoid embedding a machine-specific absolute path in evidence."""
    return f"<external>/{path.name}" if path.is_absolute() else str(path)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--expect", type=Path)
    args = parser.parse_args()
    result = manifest(args.root)
    expected_sha = None
    matches = True
    if args.expect:
        expected_sha = json.loads(args.expect.read_text())["manifest_sha256"]
        matches = result["manifest_sha256"] == expected_sha
    result["expected_manifest_sha256"] = expected_sha
    result["matches_expected"] = matches
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(
        json.dumps(
            {
                "output": str(args.output),
                "manifest_sha256": result["manifest_sha256"],
                "matches_expected": matches,
            },
            ensure_ascii=False,
        )
    )
    return 0 if matches else 1


if __name__ == "__main__":
    sys.exit(main())
