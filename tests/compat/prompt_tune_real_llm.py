"""Run a local GraphRAG-live to GraphLoom-replay prompt-tune acceptance."""

from __future__ import annotations

import argparse
import asyncio
import collections
import json
import os
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

import yaml
from dotenv import dotenv_values

from prompt_tune_top_reference import (
    FIXTURE_ROOT,
    GRAPH_RAG_COMMIT,
    GRAPH_RAG_VERSION,
    OPERATION_ORDER,
    OUTPUT_NAMES,
    ReplayEntry,
    ReplayServer,
    byte_evidence,
    classify_operation,
    default_graphloom_bin,
    fixed_graphrag_source,
    message_bytes,
    normalize_messages,
    selected_chunk_records,
    sha256_bytes,
    stable_json_bytes,
)

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUTPUT_ROOT = REPOSITORY_ROOT / "prompt_tune_real_llm"
RUN_NAME_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
SECRET_KEYS = {
    "api_key",
    "apikey",
    "authorization",
    "password",
    "secret",
    "token",
}


def is_secret_key(key: str) -> bool:
    """Return whether a configuration field contains credential material."""
    normalized = key.lower().replace("-", "_")
    return normalized in SECRET_KEYS or normalized.endswith(("_key", "_token"))


def redact(value: Any) -> Any:
    """Recursively redact credential-bearing configuration fields."""
    if isinstance(value, dict):
        return {
            key: ("<redacted>" if is_secret_key(str(key)) else redact(item))
            for key, item in value.items()
        }
    if isinstance(value, list):
        return [redact(item) for item in value]
    return value


def load_environment(path: Path | None) -> tuple[str, ...]:
    """Load an optional dotenv file into this process without logging values."""
    if path is None:
        return ()
    if not path.is_file():
        raise FileNotFoundError(f"environment file does not exist: {path}")
    loaded: list[str] = []
    for key, value in dotenv_values(path).items():
        if value is None:
            continue
        os.environ.setdefault(key, value)
        loaded.append(key)
    return tuple(loaded)


def resolve_environment_reference(value: str) -> str | None:
    """Resolve a complete ``${NAME}`` reference without exposing its value."""
    match = re.fullmatch(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}", value)
    return match.group(1) if match else None


@dataclass
class LiveSettings:
    """Sanitized live completion configuration and report metadata."""

    project_settings: dict[str, Any]
    provider: str
    model: str
    model_parameters: dict[str, Any]


def build_live_settings(source: Path) -> LiveSettings:
    """Combine a real completion model with the committed Top fixture settings."""
    if not source.is_file():
        raise FileNotFoundError(f"settings file does not exist: {source}")
    raw = yaml.safe_load(source.read_text("utf-8")) or {}
    completion_models = raw.get("completion_models")
    if not isinstance(completion_models, dict) or not completion_models:
        raise ValueError("settings must define at least one completion model")
    model_id = (
        raw.get("extract_graph", {}).get("completion_model_id")
        or "default_completion_model"
    )
    model_config = completion_models.get(model_id)
    if not isinstance(model_config, dict):
        raise ValueError(f"completion model {model_id!r} is not configured")
    model_config = json.loads(json.dumps(model_config))
    api_base = model_config.get("api_base")
    if isinstance(api_base, str):
        parsed_api_base = urlsplit(api_base)
        if parsed_api_base.hostname in {"127.0.0.1", "::1", "localhost"} and (
            parsed_api_base.port == 9
        ):
            raise ValueError(
                "completion api_base is the disabled loopback port 9 sentinel"
            )

    api_key = model_config.get("api_key")
    if isinstance(api_key, str):
        environment_name = resolve_environment_reference(api_key)
        if environment_name is None:
            environment_name = "PROMPT_TUNE_REAL_LLM_API_KEY"
            os.environ[environment_name] = api_key
            model_config["api_key"] = f"${{{environment_name}}}"
        if not os.environ.get(environment_name):
            raise ValueError(
                f"completion credential environment variable is unset: {environment_name}"
            )
    elif model_config.get("auth_method", "api_key") == "api_key":
        raise ValueError("completion model does not define an api_key reference")

    fixture_settings = yaml.safe_load(
        (FIXTURE_ROOT / "settings.yaml").read_text("utf-8")
    )
    fixture_settings["completion_models"] = {"default_completion_model": model_config}
    model_parameters = {
        key: value
        for key, value in model_config.items()
        if not is_secret_key(key)
        and key
        not in {
            "api_base",
            "organization",
            "proxy",
        }
    }
    return LiveSettings(
        project_settings=fixture_settings,
        provider=str(model_config.get("model_provider", "")),
        model=str(model_config.get("model", "")),
        model_parameters=redact(model_parameters),
    )


@dataclass
class LiveRecord:
    """One real GraphRAG completion request and its raw response content."""

    scenario: str
    operation: str
    producer_local_ordinal: int
    input_identity: str
    messages: list[dict[str, str]]
    logical_response_format: str | None
    graphrag_call_fields: list[str]
    response: str

    def request_json(self) -> dict[str, Any]:
        """Return stable request evidence without transport secrets."""
        encoded = message_bytes(self.messages)
        return {
            "graphrag_call_fields": self.graphrag_call_fields,
            "input_identity": self.input_identity,
            "logical_response_format": self.logical_response_format,
            "messages": self.messages,
            "messages_sha256": sha256_bytes(encoded),
            "messages_utf8_length": len(encoded),
            "operation": self.operation,
            "producer_local_ordinal": self.producer_local_ordinal,
            "scenario": self.scenario,
        }

    def response_json(self) -> dict[str, Any]:
        """Return raw response bytes and stable evidence."""
        return {
            "operation": self.operation,
            "producer_local_ordinal": self.producer_local_ordinal,
            "response": self.response,
            **{
                f"response_{key}": value
                for key, value in byte_evidence(self.response).items()
            },
            "scenario": self.scenario,
        }


class LiveRecordingModel:
    """GraphRAG completion wrapper that records the actual model responses."""

    def __init__(self, delegate: Any, selected_chunks: list[str]) -> None:
        self._delegate = delegate
        self._selected_chunks = selected_chunks
        self._records: list[LiveRecord] = []
        self._counts: collections.Counter[str] = collections.Counter()

    @property
    def records(self) -> list[LiveRecord]:
        """Return records in producer-local semantic order."""
        return sorted(
            self._records,
            key=lambda record: (
                OPERATION_ORDER[record.operation],
                record.producer_local_ordinal,
            ),
        )

    async def completion_async(self, **kwargs: Any) -> Any:
        """Call the real model and retain exact logical request/response bytes."""
        allowed_fields = {"messages", "response_format", "response_format_json_object"}
        unknown_fields = set(kwargs) - allowed_fields
        if unknown_fields:
            raise ValueError(
                f"unapproved GraphRAG call fields: {sorted(unknown_fields)}"
            )
        response_format = kwargs.get("response_format")
        messages = normalize_messages(kwargs["messages"])
        operation = classify_operation(messages, response_format)
        ordinal = self._counts[operation]
        self._counts[operation] += 1
        response = await self._delegate.completion_async(**kwargs)
        content = response.content
        if not isinstance(content, str):
            raise TypeError(f"{operation} completion content is not text")

        if operation == "entity_relationship":
            if ordinal >= len(self._selected_chunks):
                raise ValueError("too many entity relationship requests")
            identity_source = self._selected_chunks[ordinal]
        elif operation == "persona":
            identity_source = next(
                record.response
                for record in self._records
                if record.operation == "domain"
            )
        else:
            identity_source = "\n".join(self._selected_chunks)
        self._records.append(
            LiveRecord(
                scenario="typed-live",
                operation=operation,
                producer_local_ordinal=ordinal,
                input_identity=sha256_bytes(identity_source.encode("utf-8")),
                messages=messages,
                logical_response_format=(
                    response_format.__name__ if response_format is not None else None
                ),
                graphrag_call_fields=sorted(kwargs),
                response=content,
            )
        )
        return response


def write_bytes(path: Path, data: bytes) -> None:
    """Write one local acceptance artifact with parent creation."""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


async def run_graphrag_live(
    project: Path,
) -> tuple[list[str], list[str], list[LiveRecord], tuple[str, str, str], Any]:
    """Run fixed GraphRAG Top prompt tuning against the configured live model."""
    import graphrag.api.prompt_tune as prompt_tune_api
    from graphrag.config.load_config import load_config
    from graphrag.prompt_tune.types import DocSelectionType
    from graphrag_llm.embedding import create_embedding

    config = load_config(root_dir=project)
    selected_chunks: list[str] = []
    original_loader = prompt_tune_api.load_docs_in_chunks
    original_create_completion = prompt_tune_api.create_completion
    recorder: LiveRecordingModel | None = None

    async def recording_loader(**kwargs: Any) -> list[str]:
        chunks = await original_loader(**kwargs)
        selected_chunks.extend(chunks)
        return chunks

    def recording_factory(model_config: Any) -> LiveRecordingModel:
        nonlocal recorder
        recorder = LiveRecordingModel(
            original_create_completion(model_config), selected_chunks
        )
        return recorder

    original_cwd = Path.cwd()
    prompt_tune_api.load_docs_in_chunks = recording_loader
    prompt_tune_api.create_completion = recording_factory
    try:
        outputs = await prompt_tune_api.generate_indexing_prompts(
            config=config,
            limit=3,
            selection_method=DocSelectionType.TOP,
            max_tokens=2000,
            discover_entity_types=True,
            min_examples_required=2,
            n_subset_max=300,
            k=15,
        )
        all_chunks = await original_loader(
            config=config,
            limit=3,
            select_method=DocSelectionType.ALL,
            logger=__import__("logging").getLogger("prompt-tune-real-llm"),
            n_subset_max=300,
            k=15,
        )
    finally:
        prompt_tune_api.load_docs_in_chunks = original_loader
        prompt_tune_api.create_completion = original_create_completion
        os.chdir(original_cwd)
    if recorder is None:
        raise RuntimeError("GraphRAG did not create a completion model")
    embedding_config = config.get_embedding_model_config(
        config.embed_text.embedding_model_id
    )
    tokenizer = create_embedding(embedding_config).tokenizer
    return all_chunks, selected_chunks, recorder.records, outputs, tokenizer


def prepare_run_directory(output_root: Path, run_name: str, clean: bool) -> Path:
    """Create one bounded run directory without implicit overwrite."""
    if not RUN_NAME_PATTERN.fullmatch(run_name):
        raise ValueError("run name must match [A-Za-z0-9][A-Za-z0-9._-]{0,63}")
    output_root = output_root.resolve()
    run_directory = output_root / run_name
    if run_directory.exists():
        if not clean:
            raise FileExistsError(
                f"run directory already exists; choose another name or pass --clean: "
                f"{run_directory}"
            )
        if run_directory.parent != output_root:
            raise ValueError("refusing to clean a directory outside the output root")
        shutil.rmtree(run_directory)
    run_directory.mkdir(parents=True)
    return run_directory


def prepare_graphrag_project(project: Path, settings: dict[str, Any]) -> None:
    """Create a sanitized local GraphRAG project using committed inputs."""
    project.mkdir(parents=True)
    shutil.copytree(FIXTURE_ROOT / "input", project / "input")
    write_bytes(
        project / "settings.yaml",
        yaml.safe_dump(
            settings,
            sort_keys=False,
            allow_unicode=True,
        ).encode("utf-8"),
    )


def replay_entries(records: list[LiveRecord]) -> list[ReplayEntry]:
    """Build request-aware replay entries and validate duplicate identities."""
    entries = [
        ReplayEntry(record.request_json(), record.response_json()) for record in records
    ]
    grouped: dict[tuple[str, str], set[str]] = collections.defaultdict(set)
    for entry in entries:
        grouped[
            (
                entry.request["operation"],
                entry.request["messages_sha256"],
            )
        ].add(entry.response["response"])
    ambiguous = {
        key: len(responses) for key, responses in grouped.items() if len(responses) != 1
    }
    if ambiguous:
        raise ValueError(
            "identical logical requests returned different live responses; "
            f"request-aware replay cannot assign them without FIFO: {ambiguous}"
        )
    return entries


def run_graphloom_live_replay(
    graphloom_bin: Path,
    run_directory: Path,
    records: list[LiveRecord],
) -> dict[str, bytes]:
    """Replay exact live responses through GraphLoom and return output bytes."""
    server = ReplayServer("typed-live", replay_entries(records))
    server.start()
    project = run_directory / "graphloom" / "project"
    try:
        from prompt_tune_top_reference import copy_project

        project.parent.mkdir(parents=True)
        copy_project(project, server.api_base)
        result = subprocess.run(
            [
                str(graphloom_bin.resolve()),
                "prompt-tune",
                "--root",
                str(project),
                "--selection-method",
                "top",
                "--limit",
                "3",
                "--chunk-size",
                "38",
                "--overlap",
                "0",
                "--max-tokens",
                "2000",
                "--min-examples-required",
                "2",
                "--output",
                "generated",
            ],
            cwd=project,
            env={**os.environ, "GRAPHRAG_API_KEY": "compat-replay-key"},
            capture_output=True,
            text=True,
            check=False,
        )
        write_bytes(
            run_directory / "graphloom" / "stdout.txt",
            result.stdout.encode("utf-8"),
        )
        write_bytes(
            run_directory / "graphloom" / "stderr.txt",
            result.stderr.encode("utf-8"),
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"GraphLoom replay failed with exit code {result.returncode}; "
                f"see {run_directory / 'graphloom'}"
            )
        server.assert_exhausted()
        return {
            name: (project / "generated" / name).read_bytes() for name in OUTPUT_NAMES
        }
    finally:
        server.close()


def git_head() -> str:
    """Return the exact GraphLoom commit used for the run."""
    return subprocess.check_output(
        ["git", "rev-parse", "HEAD"],
        cwd=REPOSITORY_ROOT,
        text=True,
    ).strip()


def compare_outputs(
    reference: dict[str, bytes], actual: dict[str, bytes]
) -> dict[str, Any]:
    """Require full output byte equality and return stable evidence."""
    comparisons = {}
    for name in OUTPUT_NAMES:
        expected = reference[name]
        observed = actual[name]
        comparisons[name] = {
            "equal": expected == observed,
            "graphrag": byte_evidence(expected.decode("utf-8")),
            "graphloom": byte_evidence(observed.decode("utf-8")),
        }
        if expected != observed:
            raise AssertionError(f"{name} differs between GraphRAG and GraphLoom")
    return comparisons


def markdown_report(summary: dict[str, Any]) -> str:
    """Render a secret-free local acceptance report."""
    lines = [
        "# Prompt-tune real-LLM compatibility report",
        "",
        f"- Result: **{summary['result']}**",
        f"- GraphRAG: `{summary['graphrag_commit']}` ({summary['graphrag_version']})",
        f"- GraphLoom: `{summary['graphloom_commit']}`",
        f"- Provider: `{summary['provider']}`",
        f"- Model: `{summary['model']}`",
        f"- Selected chunks: `{summary['selected_chunk_count']}` (exact order matched)",
        f"- GraphRAG live requests: `{summary['graphrag_live_request_count']}`",
        f"- GraphLoom replay requests: `{summary['graphloom_replay_request_count']}`",
        "- Logical requests: exact full role/content byte match",
        "- Outputs: exact UTF-8 byte match for all three prompt files",
        "",
        "Secrets, authorization headers, and raw provider envelopes are not recorded.",
        "",
    ]
    return "\n".join(lines)


def run_acceptance(arguments: argparse.Namespace) -> Path:
    """Execute GraphRAG live record followed by GraphLoom exact replay."""
    load_environment(arguments.env_file)
    live_settings = build_live_settings(arguments.settings)
    run_directory = prepare_run_directory(
        arguments.output_root, arguments.run_name, arguments.clean
    )
    graphloom_bin = arguments.graphloom_bin.resolve()
    if not graphloom_bin.is_file():
        raise FileNotFoundError(f"GraphLoom binary does not exist: {graphloom_bin}")
    graphrag_project = run_directory / "graphrag" / "project"
    prepare_graphrag_project(graphrag_project, live_settings.project_settings)

    phase = "graphrag_live_record"
    try:
        with fixed_graphrag_source(arguments.graphrag_source):
            (
                all_chunks,
                selected_chunks,
                records,
                graphrag_outputs,
                tokenizer,
            ) = asyncio.run(run_graphrag_live(graphrag_project))
        selected = selected_chunk_records(all_chunks, selected_chunks, tokenizer)
        requests = [record.request_json() for record in records]
        responses = [record.response_json() for record in records]
        write_bytes(
            run_directory / "record" / "requests.json", stable_json_bytes(requests)
        )
        write_bytes(
            run_directory / "record" / "responses.json", stable_json_bytes(responses)
        )
        write_bytes(
            run_directory / "graphrag" / "selected_chunks.json",
            stable_json_bytes(selected),
        )
        reference_outputs = {}
        for name, content in zip(OUTPUT_NAMES, graphrag_outputs, strict=True):
            data = content.encode("utf-8")
            reference_outputs[name] = data
            write_bytes(run_directory / "graphrag" / "generated" / name, data)

        phase = "graphloom_replay"
        replay_outputs = run_graphloom_live_replay(
            graphloom_bin, run_directory, records
        )
        phase = "comparison"
        output_comparison = compare_outputs(reference_outputs, replay_outputs)
        operation_counts = collections.Counter(record.operation for record in records)
        summary = {
            "cache_strategy": "disabled for prompt-tune completion calls",
            "graphrag_commit": GRAPH_RAG_COMMIT,
            "graphrag_version": GRAPH_RAG_VERSION,
            "graphloom_commit": git_head(),
            "graphrag_live_request_count": len(records),
            "graphloom_replay_request_count": len(records),
            "input_sha256": [
                sha256_bytes(path.read_bytes())
                for path in sorted((FIXTURE_ROOT / "input").glob("*.txt"))
            ],
            "logical_request_comparison": "exact full role/content byte equality",
            "model": live_settings.model,
            "model_parameters": live_settings.model_parameters,
            "operation_request_counts": dict(sorted(operation_counts.items())),
            "output_comparison": output_comparison,
            "prompt_tune_parameters": {
                "chunk_size": 38,
                "discover_entity_types": True,
                "limit": 3,
                "max_tokens": 2000,
                "min_examples_required": 2,
                "overlap": 0,
                "selection_method": "top",
            },
            "provider": live_settings.provider,
            "result": "success",
            "selected_chunk_count": len(selected),
            "selected_chunk_sha256": [chunk["chunk_sha256"] for chunk in selected],
            "selected_chunk_top_ordinals": [chunk["top_ordinal"] for chunk in selected],
            "settings_sha256": sha256_bytes(
                (graphrag_project / "settings.yaml").read_bytes()
            ),
            "top_selection_comparison": (
                "exact ordered chunks established by full request replay"
            ),
        }
        write_bytes(
            run_directory / "comparison" / "summary.json",
            stable_json_bytes(summary),
        )
        write_bytes(
            run_directory / "REPORT.md",
            markdown_report(summary).encode("utf-8"),
        )
        return run_directory
    except Exception as error:
        failure = {
            "error_type": type(error).__name__,
            "graphrag_commit": GRAPH_RAG_COMMIT,
            "graphloom_commit": git_head(),
            "phase": phase,
            "result": "failure",
        }
        write_bytes(
            run_directory / "comparison" / "failure.json",
            stable_json_bytes(failure),
        )
        write_bytes(
            run_directory / "REPORT.md",
            (
                "# Prompt-tune real-LLM compatibility report\n\n"
                "- Result: **failure**\n"
                f"- Phase: `{phase}`\n"
                f"- Error type: `{type(error).__name__}`\n\n"
                "See the local GraphRAG/GraphLoom logs. The provider exception "
                "text is intentionally omitted to avoid persisting secrets.\n"
            ).encode("utf-8"),
        )
        raise


def validate(arguments: argparse.Namespace) -> None:
    """Validate local prerequisites without contacting a model."""
    load_environment(arguments.env_file)
    settings = build_live_settings(arguments.settings)
    if not arguments.graphrag_source.is_dir():
        raise FileNotFoundError(
            f"GraphRAG repository does not exist: {arguments.graphrag_source}"
        )
    subprocess.run(
        [
            "git",
            "-C",
            str(arguments.graphrag_source),
            "cat-file",
            "-e",
            f"{GRAPH_RAG_COMMIT}^{{commit}}",
        ],
        check=True,
        capture_output=True,
    )
    print(
        "real-LLM prerequisites valid: "
        f"GraphRAG {GRAPH_RAG_VERSION} {GRAPH_RAG_COMMIT}, "
        f"provider={settings.provider}, model={settings.model}"
    )


def parse_args() -> argparse.Namespace:
    """Parse safe, explicit live-run arguments."""
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument(
        "--check",
        action="store_true",
        help="validate configuration without network calls or output writes",
    )
    mode.add_argument(
        "--run",
        action="store_true",
        help="perform GraphRAG live record followed by GraphLoom replay",
    )
    parser.add_argument("--settings", type=Path, required=True)
    parser.add_argument("--env-file", type=Path)
    parser.add_argument("--graphrag-source", type=Path, default=Path("../graphrag"))
    parser.add_argument("--graphloom-bin", type=Path, default=default_graphloom_bin())
    parser.add_argument("--output-root", type=Path, default=DEFAULT_OUTPUT_ROOT)
    parser.add_argument("--run-name", default="typed-top")
    parser.add_argument(
        "--clean",
        action="store_true",
        help="remove only the selected run directory before a live run",
    )
    return parser.parse_args()


def main() -> int:
    """Validate prerequisites or execute the explicit live acceptance."""
    arguments = parse_args()
    try:
        if arguments.check:
            validate(arguments)
            return 0
        run_directory = run_acceptance(arguments)
        print(f"real-LLM record/replay succeeded; local artifacts: {run_directory}")
        return 0
    except (FileExistsError, FileNotFoundError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    except Exception as error:
        run_directory = arguments.output_root.resolve() / arguments.run_name
        print(
            f"error: {type(error).__name__}; see ignored local artifacts in "
            f"{run_directory}",
            file=sys.stderr,
        )
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
