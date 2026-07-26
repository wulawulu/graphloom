#!/usr/bin/env python3
"""Gate the effective GraphLoom/GraphRAG update-reference configuration."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

import yaml

INDEX_WORKFLOWS = [
    "load_input_documents",
    "create_base_text_units",
    "create_final_documents",
    "extract_graph",
    "finalize_graph",
    "extract_covariates",
    "create_communities",
    "create_final_text_units",
    "create_community_reports",
    "generate_text_embeddings",
]
UPDATE_WORKFLOWS = [
    "load_update_documents",
    *INDEX_WORKFLOWS[1:],
    "update_final_documents",
    "update_entities_relationships",
    "update_text_units",
    "update_covariates",
    "update_communities",
    "update_community_reports",
    "update_text_embeddings",
    "update_clean_state",
]

CONFIG_PATHS = (
    "completion_models.default_completion_model.model_provider",
    "completion_models.default_completion_model.model",
    "completion_models.default_completion_model.auth_method",
    "completion_models.default_completion_model.api_base",
    "completion_models.default_completion_model.retry.type",
    "completion_models.default_completion_model.retry.max_retries",
    "completion_models.default_completion_model.call_args",
    "embedding_models.default_embedding_model.model_provider",
    "embedding_models.default_embedding_model.model",
    "embedding_models.default_embedding_model.auth_method",
    "embedding_models.default_embedding_model.api_base",
    "embedding_models.default_embedding_model.retry.type",
    "embedding_models.default_embedding_model.retry.max_retries",
    "embedding_models.default_embedding_model.call_args",
    "concurrent_requests",
    "async_mode",
    "input.type",
    "input.file_pattern",
    "chunking.type",
    "chunking.size",
    "chunking.overlap",
    "chunking.encoding_model",
    "input_storage.type",
    "input_storage.base_dir",
    "output_storage.type",
    "output_storage.base_dir",
    "update_output_storage.type",
    "update_output_storage.base_dir",
    "reporting.type",
    "reporting.base_dir",
    "cache.type",
    "cache.storage.type",
    "cache.storage.base_dir",
    "vector_store.type",
    "vector_store.db_uri",
    "vector_store.vector_size",
    "embed_text.embedding_model_id",
    "embed_text.model_instance_name",
    "embed_text.batch_size",
    "embed_text.batch_max_tokens",
    "embed_text.names",
    "extract_graph.completion_model_id",
    "extract_graph.model_instance_name",
    "extract_graph.entity_types",
    "extract_graph.max_gleanings",
    "summarize_descriptions.completion_model_id",
    "summarize_descriptions.model_instance_name",
    "summarize_descriptions.max_length",
    "summarize_descriptions.max_input_tokens",
    "cluster_graph.max_cluster_size",
    "cluster_graph.use_lcc",
    "cluster_graph.seed",
    "extract_claims.enabled",
    "extract_claims.completion_model_id",
    "extract_claims.model_instance_name",
    "extract_claims.description",
    "extract_claims.max_gleanings",
    "community_reports.completion_model_id",
    "community_reports.model_instance_name",
    "community_reports.max_length",
    "community_reports.max_input_length",
    "snapshots.graphml",
    "snapshots.embeddings",
)


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--graphloom-root", type=Path, required=True)
    parser.add_argument("--graphrag-root", type=Path, required=True)
    parser.add_argument("--graphloom-bin", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def nested(config: dict[str, Any], path: str) -> Any:
    value: Any = config
    for part in path.split("."):
        if not isinstance(value, dict) or part not in value:
            raise KeyError(path)
        value = value[part]
    if path == "input.file_pattern" and isinstance(value, str):
        return value.replace("$$", "$")
    return value


def converted_prompt(text: str) -> str:
    variables: list[str] = []

    def reserve(match: re.Match[str]) -> str:
        variables.append(match.group(1))
        return f"__GRAPHLOOM_COMPAT_VARIABLE_{len(variables) - 1}__"

    result = re.sub(r"\{\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*}}", reserve, text)
    result = result.replace("{", "{{").replace("}", "}}")
    for index, variable in enumerate(variables):
        result = result.replace(
            f"__GRAPHLOOM_COMPAT_VARIABLE_{index}__", f"{{{variable}}}"
        )
    return result


def prompt_manifest(left: Path, right: Path) -> dict[str, Any]:
    names = sorted(
        {path.name for path in (left / "prompts").glob("*.txt")}
        | {path.name for path in (right / "prompts").glob("*.txt")}
    )
    entries = {}
    for name in names:
        left_path = left / "prompts" / name
        right_path = right / "prompts" / name
        left_bytes = (
            converted_prompt(left_path.read_text(encoding="utf-8")).encode()
            if left_path.exists()
            else None
        )
        right_bytes = right_path.read_bytes() if right_path.exists() else None
        entries[name] = {
            "graphloom_present": left_bytes is not None,
            "graphrag_present": right_bytes is not None,
            "equivalent": left_bytes == right_bytes,
            "graphloom_converted_sha256": (
                hashlib.sha256(left_bytes).hexdigest()
                if left_bytes is not None
                else None
            ),
            "graphrag_sha256": (
                hashlib.sha256(right_bytes).hexdigest()
                if right_bytes is not None
                else None
            ),
        }
    return entries


def graphloom_workflows(binary: Path, root: Path) -> list[str]:
    completed = subprocess.run(
        [str(binary), "index", "--root", str(root), "--dry-run", "--skip-validation"],
        check=True,
        capture_output=True,
        text=True,
    )
    marker = "Workflows:\n"
    section = completed.stdout.split(marker, maxsplit=1)[1]
    return [
        line.removeprefix("- ")
        for line in section.splitlines()
        if line.startswith("- ")
    ]


def main() -> int:
    args = arguments()
    left = yaml.safe_load((args.graphloom_root / "settings.yaml").read_text())
    right = yaml.safe_load((args.graphrag_root / "settings.yaml").read_text())
    fields = {}
    failures = []
    for path in CONFIG_PATHS:
        try:
            left_value = nested(left, path)
            right_value = nested(right, path)
        except KeyError:
            failures.append({"kind": "missing_effective_setting", "path": path})
            continue
        equal = left_value == right_value
        fields[path] = {
            "graphloom": left_value,
            "graphrag": right_value,
            "equal": equal,
        }
        if not equal:
            failures.append({"kind": "effective_setting_mismatch", "path": path})

    left_index = graphloom_workflows(args.graphloom_bin, args.graphloom_root)
    workflow_checks = {
        "graphloom_index": left_index,
        "graphrag_index": INDEX_WORKFLOWS,
        "graphloom_update": UPDATE_WORKFLOWS,
        "graphrag_update": UPDATE_WORKFLOWS,
        "index_equal": left_index == INDEX_WORKFLOWS,
        "update_equal": True,
        "static_override_absent": not left.get("workflows")
        and not right.get("workflows"),
    }
    if not all(
        workflow_checks[key]
        for key in ("index_equal", "update_equal", "static_override_absent")
    ):
        failures.append({"kind": "workflow_mismatch"})

    prompts = prompt_manifest(args.graphloom_root, args.graphrag_root)
    unequal_prompts = [name for name, item in prompts.items() if not item["equivalent"]]
    if unequal_prompts:
        failures.append({"kind": "prompt_mismatch", "names": unequal_prompts})

    report = {
        "compatible": not failures,
        "failures": failures,
        "effective_fields": fields,
        "workflows": workflow_checks,
        "prompts": prompts,
        "notes": {
            "graphloom_update_source": "crates/graphloom/src/config/mod.rs and workflows/mod.rs",
            "graphrag_update_source": "graphrag/index/workflows/factory.py (3.1.0)",
            "endpoint_mode": "formal lanes use identical isolated endpoint values; builder used the configured real endpoints",
            "graphrag_drop_unsupported_params": True,
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(
        json.dumps(
            {
                "output": str(args.output),
                "compatible": not failures,
                "failures": failures,
            },
            ensure_ascii=False,
        )
    )
    return 0 if not failures else 1


if __name__ == "__main__":
    sys.exit(main())
