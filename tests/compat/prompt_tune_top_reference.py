"""Generate and verify deterministic GraphRAG prompt-tune Top fixtures."""

from __future__ import annotations

import argparse
import asyncio
import collections
import contextlib
import difflib
import hashlib
import importlib.metadata
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

import yaml

GRAPH_RAG_COMMIT = "7fc6607edda3d387d23e52ededbf8a75b6730f97"
GRAPH_RAG_VERSION = "3.1.0"
FIXTURE_SCHEMA_VERSION = 1
FIXTURE_ROOT = (Path(__file__).parent / "fixtures" / "prompt_tune" / "top").resolve()
REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SCENARIOS = ("typed", "untyped")
OUTPUT_NAMES = (
    "extract_graph.txt",
    "summarize_descriptions.txt",
    "community_report_graph.txt",
)
OPERATION_ORDER = {
    "domain": 0,
    "language": 1,
    "persona": 2,
    "community_report_rating": 3,
    "entity_types": 4,
    "entity_relationship": 5,
    "community_reporter_role": 6,
}
RESPONSES = {
    "domain": "technology organizations and professional collaboration",
    "language": "English",
    "persona": (
        "You are an exacting technology intelligence analyst who maps organizations, "
        "people, platforms, and operational relationships. You preserve source wording "
        "while explaining how collaboration and information exchange shape the domain."
    ),
    "community_report_rating": (
        "Assign a higher rating when the community materially influences shared "
        "technology, operational coordination, or professional collaboration."
    ),
    "community_reporter_role": (
        "Technology ecosystem analyst reporting on organizations, people, platforms, "
        "and their documented collaborations."
    ),
}
ENTITY_TYPE_RESPONSES = {
    "typed": '{\n  "entity_types": [\n    "person",\n    "organization"\n  ]\n}',
    "untyped": '{\n  "entity_types": []\n}',
}
RELATIONSHIP_RESPONSE = {
    "typed": (
        '  ("entity"<|>"NORTHSTAR LABS"<|>"organization"<|>"Developer of Aurora.")'
        '##("entity"<|>"ALICE CHEN"<|>"person"<|>"Leader of Northstar Labs.")'
        '##("relationship"<|>"ALICE CHEN"<|>"NORTHSTAR LABS"<|>"Alice leads Northstar Labs."<|>9)'
        "##<|COMPLETE|>  \n"
    ),
    "untyped": (
        '  ("entity"<|>"NORTHSTAR LABS"<|>"organization"<|>"Developer of Aurora.")'
        '##("entity"<|>"ALICE CHEN"<|>"person"<|>"Leader of Northstar Labs.")'
        '##("relationship"<|>"ALICE CHEN"<|>"NORTHSTAR LABS"<|>"Alice leads Northstar Labs."<|>9)'
        "##<|COMPLETE|>  \n"
    ),
}
GENERATION_COMMAND = (
    "GRAPHRAG_API_KEY=compat-test-key uv run --project tests/compat --locked "
    "python tests/compat/prompt_tune_top_reference.py --update "
    '--graphrag-source ../graphrag --graphloom-bin "$GRAPHLOOM_BIN"'
)


def sha256_bytes(data: bytes) -> str:
    """Return the lowercase SHA-256 digest of bytes."""
    return hashlib.sha256(data).hexdigest()


def default_graphloom_bin() -> Path:
    """Resolve the workspace target binary without assuming target location."""
    configured = os.environ.get("GRAPHLOOM_BIN")
    if configured:
        return Path(configured)
    metadata = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            cwd=REPOSITORY_ROOT,
            text=True,
        )
    )
    suffix = ".exe" if os.name == "nt" else ""
    return Path(metadata["target_directory"]) / "debug" / f"graphloom{suffix}"


def stable_json_bytes(value: Any) -> bytes:
    """Serialize JSON with stable keys, separators, Unicode, and one final LF."""
    return (
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    ).encode("utf-8")


def message_bytes(messages: list[dict[str, str]]) -> bytes:
    """Serialize complete logical messages for stable identity matching."""
    return json.dumps(
        messages,
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")


def byte_evidence(content: str) -> dict[str, Any]:
    """Describe committed text without normalizing it."""
    data = content.encode("utf-8")
    return {
        "utf8_length": len(data),
        "sha256": sha256_bytes(data),
    }


def first_text_difference(expected: str, actual: str) -> str:
    """Describe the first byte-relevant text difference without normalization."""
    limit = min(len(expected), len(actual))
    index = next(
        (
            position
            for position in range(limit)
            if expected[position] != actual[position]
        ),
        limit,
    )
    start = max(0, index - 40)
    end = index + 80
    return (
        f"first difference at character {index}; "
        f"expected[{start}:{end}]={expected[start:end]!r}; "
        f"actual[{start}:{end}]={actual[start:end]!r}; "
        f"expected/actual character lengths={len(expected)}/{len(actual)}"
    )


def text_difference_summary(expected: str, actual: str) -> str:
    """Summarize all non-equal text spans for compatibility diagnostics."""
    changes = []
    for (
        tag,
        expected_start,
        expected_end,
        actual_start,
        actual_end,
    ) in difflib.SequenceMatcher(a=expected, b=actual, autojunk=False).get_opcodes():
        if tag == "equal":
            continue
        changes.append(
            f"{tag} expected[{expected_start}:{expected_end}]="
            f"{expected[expected_start:expected_end]!r} "
            f"actual[{actual_start}:{actual_end}]="
            f"{actual[actual_start:actual_end]!r}"
        )
    return "; ".join(changes[:10])


def normalize_messages(raw_messages: Any) -> list[dict[str, str]]:
    """Normalize GraphRAG's string shorthand to explicit role/content messages."""
    if isinstance(raw_messages, str):
        return [{"role": "user", "content": raw_messages}]
    if not isinstance(raw_messages, list):
        raise TypeError(
            f"completion messages must be str or list, got {type(raw_messages)}"
        )

    messages: list[dict[str, str]] = []
    for raw_message in raw_messages:
        if not isinstance(raw_message, dict):
            raise TypeError("completion message must be a mapping")
        if set(raw_message) != {"role", "content"}:
            raise ValueError(
                f"completion message has unapproved fields: {sorted(raw_message)}"
            )
        role = raw_message["role"]
        content = raw_message["content"]
        if not isinstance(role, str) or not isinstance(content, str):
            raise TypeError("completion role and content must be strings")
        messages.append({"role": role, "content": content})
    return messages


def classify_operation(
    messages: list[dict[str, str]], response_format: type[Any] | None
) -> str:
    """Identify one GraphRAG prompt-tune generator from its exact prompt family."""
    if response_format is not None:
        if response_format.__name__ != "EntityTypesResponse":
            raise ValueError(f"unknown structured response: {response_format.__name__}")
        return "entity_types"

    content = "\n".join(message["content"] for message in messages)
    markers = {
        "domain": "assigning a descriptive domain",
        "language": "what's the primary language",
        "persona": "Given a specific type of task and sample text",
        "community_report_rating": "rating the importance of a given text",
        "entity_relationship": "-Goal-\nGiven a text document",
        "community_reporter_role": "creating a role definition",
    }
    matches = [operation for operation, marker in markers.items() if marker in content]
    if len(matches) != 1:
        raise ValueError(f"expected one prompt operation marker, found {matches}")
    return matches[0]


def input_files() -> list[Path]:
    """Return fixture inputs in the explicit producer order."""
    return sorted((FIXTURE_ROOT / "input").glob("*.txt"))


@dataclass
class GraphRagRecord:
    """One reference completion captured from GraphRAG."""

    scenario: str
    operation: str
    producer_local_ordinal: int
    input_identity: str
    messages: list[dict[str, str]]
    logical_response_format: str | None
    graphrag_call_fields: list[str]
    response: str

    def request_json(self) -> dict[str, Any]:
        """Return the committed request record."""
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
        """Return the committed response record."""
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


class GraphRagRecorder:
    """Duck-typed GraphRAG completion model with deterministic responses."""

    def __init__(self, scenario: str, selected_chunks: list[str]) -> None:
        self._scenario = scenario
        self._selected_chunks = selected_chunks
        self._records: list[GraphRagRecord] = []
        self._operation_counts: collections.Counter[str] = collections.Counter()

    @property
    def records(self) -> list[GraphRagRecord]:
        """Return records in producer-local semantic order."""
        return sorted(
            self._records,
            key=lambda record: (
                OPERATION_ORDER[record.operation],
                record.producer_local_ordinal,
            ),
        )

    async def completion_async(self, **kwargs: Any) -> Any:
        """Record GraphRAG's real generator request and return fixture content."""
        from graphrag_llm.utils import create_completion_response

        allowed_fields = {"messages", "response_format", "response_format_json_object"}
        unknown_fields = set(kwargs) - allowed_fields
        if unknown_fields:
            raise ValueError(
                f"unapproved GraphRAG call fields: {sorted(unknown_fields)}"
            )
        if kwargs.get("response_format_json_object") not in (None, False):
            raise ValueError("JSON-object relationship mode must remain disabled")

        response_format = kwargs.get("response_format")
        messages = normalize_messages(kwargs["messages"])
        operation = classify_operation(messages, response_format)
        ordinal = self._operation_counts[operation]
        self._operation_counts[operation] += 1

        if operation == "entity_relationship":
            if ordinal >= len(self._selected_chunks):
                raise ValueError("too many entity relationship requests")
            user_content = "\n".join(
                message["content"] for message in messages if message["role"] == "user"
            )
            missing_chunks = [
                chunk for chunk in self._selected_chunks if chunk not in user_content
            ]
            if missing_chunks:
                raise ValueError(
                    "GraphRAG relationship request did not retain every accumulated "
                    "selected chunk"
                )
            # GraphRAG 3.1.0 reuses one mutable CompletionMessagesBuilder. Its
            # async calls therefore observe the same accumulated message list.
            # Identical request bytes deliberately map to identical responses.
            response_content = RELATIONSHIP_RESPONSE[self._scenario]
            identity = sha256_bytes(self._selected_chunks[ordinal].encode("utf-8"))
        elif operation == "entity_types":
            response_content = ENTITY_TYPE_RESPONSES[self._scenario]
            identity = sha256_bytes("\n".join(self._selected_chunks).encode("utf-8"))
        else:
            response_content = RESPONSES[operation]
            identity_source = (
                RESPONSES["domain"]
                if operation == "persona"
                else "\n".join(self._selected_chunks)
            )
            identity = sha256_bytes(identity_source.encode("utf-8"))

        logical_response_format = (
            response_format.__name__ if response_format is not None else None
        )
        self._records.append(
            GraphRagRecord(
                scenario=self._scenario,
                operation=operation,
                producer_local_ordinal=ordinal,
                input_identity=identity,
                messages=messages,
                logical_response_format=logical_response_format,
                graphrag_call_fields=sorted(kwargs),
                response=response_content,
            )
        )

        response = create_completion_response(response_content)
        if response_format is not None:
            response.formatted_response = response_format.model_validate_json(
                response_content
            )
        return response


@contextlib.contextmanager
def fixed_graphrag_source(source: Path) -> Any:
    """Import GraphRAG from an archive of the fixed release commit."""
    source = source.resolve()
    actual_type = subprocess.check_output(
        ["git", "-C", str(source), "cat-file", "-t", GRAPH_RAG_COMMIT],
        text=True,
    ).strip()
    if actual_type != "commit":
        raise RuntimeError(f"{GRAPH_RAG_COMMIT} is not a commit in {source}")
    version_line = subprocess.check_output(
        [
            "git",
            "-C",
            str(source),
            "show",
            f"{GRAPH_RAG_COMMIT}:packages/graphrag/pyproject.toml",
        ],
        text=True,
    )
    if f'version = "{GRAPH_RAG_VERSION}"' not in version_line:
        raise RuntimeError("fixed GraphRAG commit does not declare version 3.1.0")

    archive = subprocess.check_output(
        ["git", "-C", str(source), "archive", "--format=tar", GRAPH_RAG_COMMIT]
    )
    with tempfile.TemporaryDirectory(prefix="graphloom-graphrag-3.1.0-") as temp:
        archive_root = Path(temp)
        with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as tar:
            tar.extractall(archive_root, filter="data")
        package_paths = [
            str(path)
            for path in sorted((archive_root / "packages").iterdir())
            if path.is_dir()
        ]
        original_path = sys.path[:]
        sys.path[:0] = package_paths
        try:
            yield
        finally:
            sys.path[:] = original_path


async def run_graphrag_reference(
    scenario: str,
) -> tuple[list[str], list[str], list[GraphRagRecord], tuple[str, str, str]]:
    """Run the real GraphRAG Top flow against a deterministic recorder."""
    import graphrag.api.prompt_tune as prompt_tune_api
    from graphrag.config.load_config import load_config
    from graphrag.prompt_tune.types import DocSelectionType

    if importlib.metadata.version("graphrag") != GRAPH_RAG_VERSION:
        raise RuntimeError("active GraphRAG distribution is not version 3.1.0")

    original_cwd = Path.cwd()
    with tempfile.TemporaryDirectory(prefix="graphloom-graphrag-prompt-tune-") as temp:
        project = Path(temp) / "project"
        project.mkdir()
        shutil.copytree(FIXTURE_ROOT / "input", project / "input")
        shutil.copyfile(FIXTURE_ROOT / "settings.yaml", project / "settings.yaml")
        config = load_config(root_dir=project)
        original_loader = prompt_tune_api.load_docs_in_chunks
        selected_chunks: list[str] = []

        async def recording_loader(**kwargs: Any) -> list[str]:
            chunks = await original_loader(**kwargs)
            selected_chunks.extend(chunks)
            return chunks

        recorder = GraphRagRecorder(scenario, selected_chunks)
        original_create_completion = prompt_tune_api.create_completion
        prompt_tune_api.load_docs_in_chunks = recording_loader
        prompt_tune_api.create_completion = lambda _config: recorder
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
                logger=__import__("logging").getLogger("prompt-tune-reference"),
                n_subset_max=300,
                k=15,
            )
        finally:
            prompt_tune_api.load_docs_in_chunks = original_loader
            prompt_tune_api.create_completion = original_create_completion
            os.chdir(original_cwd)

    if len(selected_chunks) != 3:
        raise AssertionError(f"expected 3 Top chunks, got {len(selected_chunks)}")
    return all_chunks, selected_chunks, recorder.records, outputs


def source_for_chunk(chunk: str) -> tuple[str, int]:
    """Resolve a chunk to one fixture-relative source and per-document ordinal."""
    matches: list[tuple[str, int]] = []
    for path in input_files():
        text = path.read_text(encoding="utf-8")
        if chunk in text:
            matches.append(
                (path.relative_to(FIXTURE_ROOT).as_posix(), text.index(chunk))
            )
    if len(matches) != 1:
        raise ValueError(f"chunk must map to one input file, found {matches}")
    path, byte_offset = matches[0]
    return path, byte_offset


def selected_chunk_records(
    all_chunks: list[str], selected_chunks: list[str], tokenizer: Any
) -> list[dict[str, Any]]:
    """Build stable selected chunk provenance in exact Top order."""
    per_document_ordinals: collections.Counter[str] = collections.Counter()
    all_metadata: list[tuple[str, int, int]] = []
    for global_ordinal, chunk in enumerate(all_chunks):
        source, _offset = source_for_chunk(chunk)
        chunk_ordinal = per_document_ordinals[source]
        per_document_ordinals[source] += 1
        all_metadata.append((source, chunk_ordinal, global_ordinal))

    records = []
    for selected_ordinal, chunk in enumerate(selected_chunks):
        matches = [
            metadata
            for candidate, metadata in zip(all_chunks, all_metadata, strict=True)
            if candidate == chunk
        ]
        if len(matches) != 1:
            raise ValueError("selected chunk must map to one all-chunks entry")
        source, chunk_ordinal, global_ordinal = matches[0]
        records.append(
            {
                "chunk_ordinal": chunk_ordinal,
                "chunk_sha256": sha256_bytes(chunk.encode("utf-8")),
                "chunk_text": chunk,
                "chunk_token_count": len(tokenizer.encode(chunk)),
                "chunk_utf8_length": len(chunk.encode("utf-8")),
                "document_path": source,
                "selected_ordinal": selected_ordinal,
                "top_ordinal": global_ordinal,
            }
        )
    return records


def write_if_changed(
    path: Path, data: bytes, changes: list[tuple[Path, str, str]]
) -> None:
    """Write generated bytes while reporting old and new hashes."""
    old = path.read_bytes() if path.exists() else None
    if old == data:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    old_sha = sha256_bytes(old) if old is not None else "<missing>"
    new_sha = sha256_bytes(data)
    path.write_bytes(data)
    changes.append((path, old_sha, new_sha))


def build_manifest(
    scenario: str,
    all_chunks: list[str],
    selected: list[dict[str, Any]],
    requests: list[dict[str, Any]],
    responses: list[dict[str, Any]],
    outputs: dict[str, str],
) -> dict[str, Any]:
    """Build a deterministic fixture manifest."""
    operation_counts = collections.Counter(request["operation"] for request in requests)
    inputs = []
    for path in input_files():
        data = path.read_bytes()
        inputs.append(
            {
                "path": path.relative_to(FIXTURE_ROOT).as_posix(),
                "sha256": sha256_bytes(data),
                "utf8_length": len(data),
            }
        )
    settings_data = (FIXTURE_ROOT / "settings.yaml").read_bytes()
    return {
        "approved_differences": [
            {
                "classification": "approved implementation difference",
                "field": "entity_types.response_format",
                "graphrag": "EntityTypesResponse",
                "graphloom": None,
                "reason": (
                    "GraphLoom uses provider-neutral requests and client-side JSON "
                    "extraction"
                ),
            },
            {
                "classification": "equivalent disabled option",
                "field": "entity_relationship.response_format_json_object",
                "graphrag": False,
                "graphloom": "absent",
                "reason": (
                    "GraphRAG passes false to its model abstraction; both implementations "
                    "emit no logical response format"
                ),
            },
        ],
        "chunk_count": len(all_chunks),
        "document_count": len(inputs),
        "fixture_schema_version": FIXTURE_SCHEMA_VERSION,
        "generation_command": GENERATION_COMMAND,
        "graphrag_commit": GRAPH_RAG_COMMIT,
        "graphrag_version": GRAPH_RAG_VERSION,
        "inputs": inputs,
        "operation_request_counts": dict(sorted(operation_counts.items())),
        "outputs": {
            name: byte_evidence(content) for name, content in sorted(outputs.items())
        },
        "request_total": len(requests),
        "requests": [
            {
                "messages_sha256": request["messages_sha256"],
                "messages_utf8_length": request["messages_utf8_length"],
                "operation": request["operation"],
                "producer_local_ordinal": request["producer_local_ordinal"],
            }
            for request in requests
        ],
        "response_total": len(responses),
        "responses": [
            {
                "operation": response["operation"],
                "producer_local_ordinal": response["producer_local_ordinal"],
                "response_sha256": response["response_sha256"],
                "response_utf8_length": response["response_utf8_length"],
            }
            for response in responses
        ],
        "scenario": scenario,
        "settings": {
            "path": "settings.yaml",
            "sha256": sha256_bytes(settings_data),
            "utf8_length": len(settings_data),
        },
        "top_selected_chunk_sha256": [chunk["chunk_sha256"] for chunk in selected],
        "top_selected_count": len(selected),
        "top_selected_ordinals": [chunk["top_ordinal"] for chunk in selected],
    }


def assert_accumulated_relationship_contract(
    scenario: str,
    requests: list[dict[str, Any]],
    responses: list[dict[str, Any]],
) -> None:
    """Require GraphRAG's three accumulated requests and responses to be identical."""
    relationship_requests = [
        request for request in requests if request["operation"] == "entity_relationship"
    ]
    relationship_responses = [
        response
        for response in responses
        if response["operation"] == "entity_relationship"
    ]
    if len(relationship_requests) != 3:
        raise AssertionError(
            f"{scenario} relationship request multiplicity must be 3, "
            f"got {len(relationship_requests)}"
        )
    if len(relationship_responses) != 3:
        raise AssertionError(
            f"{scenario} relationship response multiplicity must be 3, "
            f"got {len(relationship_responses)}"
        )

    request_bytes = {
        message_bytes(request["messages"]) for request in relationship_requests
    }
    if len(request_bytes) != 1:
        raise AssertionError(
            f"{scenario} accumulated relationship request messages must be byte-identical"
        )
    response_bytes = {
        response["response"].encode("utf-8") for response in relationship_responses
    }
    if len(response_bytes) != 1:
        raise AssertionError(
            f"{scenario} accumulated relationship responses must be byte-identical"
        )


def update_fixtures(graphrag_source: Path) -> None:
    """Regenerate both scenarios from the fixed GraphRAG source archive."""
    changes: list[tuple[Path, str, str]] = []
    with fixed_graphrag_source(graphrag_source):
        from graphrag.config.load_config import load_config
        from graphrag_llm.embedding import create_embedding

        config = load_config(root_dir=FIXTURE_ROOT)
        embedding_config = config.get_embedding_model_config(
            config.embed_text.embedding_model_id
        )
        tokenizer = create_embedding(embedding_config).tokenizer

        for scenario in SCENARIOS:
            all_chunks, selected_chunks, records, output_values = asyncio.run(
                run_graphrag_reference(scenario)
            )
            selected = selected_chunk_records(all_chunks, selected_chunks, tokenizer)
            requests = [record.request_json() for record in records]
            responses = [record.response_json() for record in records]
            assert_accumulated_relationship_contract(
                scenario,
                requests,
                responses,
            )
            outputs = dict(zip(OUTPUT_NAMES, output_values, strict=True))
            scenario_root = FIXTURE_ROOT / scenario
            write_if_changed(
                scenario_root / "requests.json",
                stable_json_bytes(requests),
                changes,
            )
            write_if_changed(
                scenario_root / "responses.json",
                stable_json_bytes(responses),
                changes,
            )
            write_if_changed(
                scenario_root / "selected_chunks.json",
                stable_json_bytes(selected),
                changes,
            )
            for name, content in outputs.items():
                write_if_changed(
                    scenario_root / "expected" / name,
                    content.encode("utf-8"),
                    changes,
                )
            manifest = build_manifest(
                scenario, all_chunks, selected, requests, responses, outputs
            )
            write_if_changed(
                scenario_root / "manifest.json",
                stable_json_bytes(manifest),
                changes,
            )

    if changes:
        print("updated deterministic GraphRAG fixtures:")
        for path, old_sha, new_sha in changes:
            relative = (
                path.relative_to(Path.cwd())
                if path.is_relative_to(Path.cwd())
                else path
            )
            print(f"  {relative}: {old_sha} -> {new_sha}")
    else:
        print("deterministic GraphRAG fixtures already up to date")


@dataclass
class ReplayEntry:
    """One exact request-aware replay entry."""

    request: dict[str, Any]
    response: dict[str, Any]
    consumed: bool = False


class ReplayServer:
    """Concurrent OpenAI-compatible server keyed by exact logical messages."""

    def __init__(
        self,
        scenario: str,
        entries: list[ReplayEntry],
        expected_model: str = "prompt-tune-compat",
    ) -> None:
        self.scenario = scenario
        self.entries = entries
        self.expected_model = expected_model
        self.errors: list[str] = []
        self._lock = threading.Lock()
        self._started = False
        self._server = ThreadingHTTPServer(("127.0.0.1", 0), self._handler_type())
        self._server.replay = self  # type: ignore[attr-defined]
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)

    @property
    def api_base(self) -> str:
        """Return the loopback OpenAI API base."""
        host, port = self._server.server_address
        return f"http://{host}:{port}/v1"

    def start(self) -> None:
        """Start serving requests."""
        self._thread.start()
        self._started = True

    def close(self) -> None:
        """Stop the server."""
        if self._started:
            self._server.shutdown()
        self._server.server_close()
        if self._started:
            self._thread.join(timeout=5)

    def assert_exhausted(self) -> None:
        """Require exact single consumption of every expected request."""
        with self._lock:
            remaining = [
                (
                    entry.request["operation"],
                    entry.request["producer_local_ordinal"],
                )
                for entry in self.entries
                if not entry.consumed
            ]
            errors = list(self.errors)
        if errors:
            raise AssertionError(
                f"{self.scenario} replay errors:\n" + "\n".join(errors)
            )
        if remaining:
            raise AssertionError(
                f"{self.scenario} left unconsumed replay records: {remaining}"
            )

    def replay(self, payload: dict[str, Any]) -> tuple[int, dict[str, Any]]:
        """Match one request exactly and build an OpenAI-compatible response."""
        try:
            allowed_payload_fields = {"messages", "model"}
            unknown_fields = set(payload) - allowed_payload_fields
            if unknown_fields:
                raise ValueError(
                    f"unapproved GraphLoom request fields: {sorted(unknown_fields)}"
                )
            if payload.get("model") != self.expected_model:
                raise ValueError(f"unexpected model: {payload.get('model')!r}")
            messages = normalize_messages(payload.get("messages"))
            encoded = message_bytes(messages)
            digest = sha256_bytes(encoded)
            with self._lock:
                matches = [
                    entry
                    for entry in self.entries
                    if not entry.consumed
                    and entry.request["messages_sha256"] == digest
                    and entry.request["messages"] == messages
                ]
                if not matches:
                    multiplicity = sum(
                        1
                        for entry in self.entries
                        if entry.request["messages_sha256"] == digest
                        and entry.request["messages"] == messages
                    )
                    try:
                        operation = classify_operation(messages, None)
                    except ValueError:
                        operation = None
                    candidates = [
                        entry
                        for entry in self.entries
                        if entry.request["operation"] == operation
                    ]
                    detail = ""
                    if len(candidates) == 1 and len(messages) == len(
                        candidates[0].request["messages"]
                    ):
                        differences = [
                            first_text_difference(
                                expected["content"], actual["content"]
                            )
                            for expected, actual in zip(
                                candidates[0].request["messages"],
                                messages,
                                strict=True,
                            )
                            if expected != actual
                        ]
                        detail = f", {'; '.join(differences)}"
                    raise ValueError(
                        "expected exactly one unconsumed exact request match, "
                        f"found {len(matches)} (fixture multiplicity {multiplicity}, "
                        f"messages SHA-256 {digest}, "
                        f"roles {[message['role'] for message in messages]}, "
                        f"content prefix {messages[0]['content'][:120]!r}{detail})"
                    )
                response_values = {entry.response["response"] for entry in matches}
                if len(response_values) != 1:
                    raise ValueError(
                        f"{len(matches)} identical unconsumed requests map to "
                        "different responses"
                    )
                entry = matches[0]
                if entry.consumed:
                    raise ValueError("duplicate replay consumption")
                if (
                    entry.request["operation"] == "entity_types"
                    and "response_format" in payload
                ):
                    raise ValueError(
                        "GraphLoom entity-types response_format must be absent"
                    )
                entry.consumed = True
                response_content = entry.response["response"]
            return (
                200,
                {
                    "choices": [
                        {
                            "finish_reason": "stop",
                            "index": 0,
                            "message": {
                                "content": response_content,
                                "role": "assistant",
                            },
                        }
                    ],
                    "created": 0,
                    "id": "prompt-tune-replay",
                    "model": "prompt-tune-compat",
                    "object": "chat.completion",
                    "usage": {
                        "completion_tokens": 0,
                        "prompt_tokens": 0,
                        "total_tokens": 0,
                    },
                },
            )
        except (TypeError, ValueError) as error:
            message = str(error)
            with self._lock:
                self.errors.append(message)
            return 400, {"error": {"message": message, "type": "invalid_request_error"}}

    @staticmethod
    def _handler_type() -> type[BaseHTTPRequestHandler]:
        class Handler(BaseHTTPRequestHandler):
            def do_POST(self) -> None:  # noqa: N802
                if not self.path.endswith("/chat/completions"):
                    self.send_error(404)
                    return
                length = int(self.headers.get("Content-Length", "0"))
                payload = json.loads(self.rfile.read(length))
                replay: ReplayServer = self.server.replay  # type: ignore[attr-defined]
                status, response = replay.replay(payload)
                data = json.dumps(response, separators=(",", ":")).encode("utf-8")
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def log_message(self, _format: str, *_args: object) -> None:
                return

        return Handler


def load_replay_entries(scenario: str) -> list[ReplayEntry]:
    """Load and cross-check committed request/response records."""
    scenario_root = FIXTURE_ROOT / scenario
    requests = json.loads((scenario_root / "requests.json").read_text("utf-8"))
    responses = json.loads((scenario_root / "responses.json").read_text("utf-8"))
    response_by_key = {
        (response["operation"], response["producer_local_ordinal"]): response
        for response in responses
    }
    if len(response_by_key) != len(responses):
        raise AssertionError(f"{scenario} has duplicate response identities")
    entries = []
    for request in requests:
        key = (request["operation"], request["producer_local_ordinal"])
        response = response_by_key.pop(key, None)
        if response is None:
            raise AssertionError(f"{scenario} request has no response: {key}")
        entries.append(ReplayEntry(request=request, response=response))
    if response_by_key:
        raise AssertionError(
            f"{scenario} has responses without requests: {sorted(response_by_key)}"
        )
    return entries


def copy_project(destination: Path, api_base: str) -> None:
    """Create a disposable GraphLoom project with a loopback replay endpoint."""
    destination.mkdir()
    shutil.copytree(FIXTURE_ROOT / "input", destination / "input")
    settings = yaml.safe_load((FIXTURE_ROOT / "settings.yaml").read_text("utf-8"))
    settings["completion_models"]["default_completion_model"]["api_base"] = api_base
    (destination / "settings.yaml").write_text(
        yaml.safe_dump(settings, sort_keys=False, allow_unicode=True),
        encoding="utf-8",
        newline="\n",
    )
    (destination / ".env").write_text(
        "GRAPHRAG_API_KEY=compat-test-key\n", encoding="utf-8", newline="\n"
    )


def verify_static_files(scenario: str) -> dict[str, Any]:
    """Validate hashes, LF bytes, and manifest evidence."""
    scenario_root = FIXTURE_ROOT / scenario
    manifest_path = scenario_root / "manifest.json"
    manifest = json.loads(manifest_path.read_text("utf-8"))
    if manifest["fixture_schema_version"] != FIXTURE_SCHEMA_VERSION:
        raise AssertionError(f"{scenario} fixture schema version mismatch")
    if manifest["graphrag_commit"] != GRAPH_RAG_COMMIT:
        raise AssertionError(f"{scenario} GraphRAG commit mismatch")
    if manifest["graphrag_version"] != GRAPH_RAG_VERSION:
        raise AssertionError(f"{scenario} GraphRAG version mismatch")

    for input_record in manifest["inputs"]:
        data = (FIXTURE_ROOT / input_record["path"]).read_bytes()
        assert_file_evidence(scenario, input_record["path"], data, input_record)
    settings_data = (FIXTURE_ROOT / manifest["settings"]["path"]).read_bytes()
    assert_file_evidence(
        scenario, manifest["settings"]["path"], settings_data, manifest["settings"]
    )
    for name, evidence in manifest["outputs"].items():
        data = (scenario_root / "expected" / name).read_bytes()
        assert_file_evidence(scenario, f"expected/{name}", data, evidence)
        if data.startswith(b"\xef\xbb\xbf") or b"\r" in data:
            raise AssertionError(f"{scenario} expected/{name} must be UTF-8 LF-only")

    requests = json.loads((scenario_root / "requests.json").read_text("utf-8"))
    responses = json.loads((scenario_root / "responses.json").read_text("utf-8"))
    assert_accumulated_relationship_contract(scenario, requests, responses)
    selected = json.loads((scenario_root / "selected_chunks.json").read_text("utf-8"))
    if len(requests) != manifest["request_total"]:
        raise AssertionError(f"{scenario} request total mismatch")
    if len(responses) != manifest["response_total"]:
        raise AssertionError(f"{scenario} response total mismatch")
    if [chunk["top_ordinal"] for chunk in selected] != manifest[
        "top_selected_ordinals"
    ]:
        raise AssertionError(f"{scenario} Top order mismatch")
    for request in requests:
        encoded = message_bytes(request["messages"])
        assert_file_evidence(
            scenario,
            f"{request['operation']} messages",
            encoded,
            {
                "utf8_length": request["messages_utf8_length"],
                "sha256": request["messages_sha256"],
            },
        )
    for response in responses:
        encoded = response["response"].encode("utf-8")
        assert_file_evidence(
            scenario,
            f"{response['operation']} response",
            encoded,
            {
                "utf8_length": response["response_utf8_length"],
                "sha256": response["response_sha256"],
            },
        )
    return manifest


def assert_file_evidence(
    scenario: str, name: str, data: bytes, evidence: dict[str, Any]
) -> None:
    """Compare bytes with committed length and digest evidence."""
    actual = {"utf8_length": len(data), "sha256": sha256_bytes(data)}
    expected = {
        "utf8_length": evidence["utf8_length"],
        "sha256": evidence["sha256"],
    }
    if actual != expected:
        raise AssertionError(
            f"{scenario} {name} byte evidence mismatch: {actual} != {expected}"
        )


def run_graphloom_replay(graphloom_bin: Path, scenario: str) -> None:
    """Replay one committed GraphRAG scenario through the GraphLoom CLI."""
    entries = load_replay_entries(scenario)
    server = ReplayServer(scenario, entries)
    server.start()
    try:
        with tempfile.TemporaryDirectory(
            prefix=f"graphloom-prompt-tune-{scenario}-"
        ) as temp:
            project = Path(temp) / "project"
            copy_project(project, server.api_base)
            environment = os.environ.copy()
            environment.pop("PYTHONPATH", None)
            environment["GRAPHRAG_API_KEY"] = "compat-test-key"
            result = subprocess.run(
                [
                    str(graphloom_bin),
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
                env=environment,
                capture_output=True,
                text=True,
                check=False,
            )
            if result.returncode != 0:
                raise AssertionError(
                    f"{scenario} GraphLoom replay failed ({result.returncode})\n"
                    f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
                )
            server.assert_exhausted()
            for name in OUTPUT_NAMES:
                actual = (project / "generated" / name).read_bytes()
                expected = (FIXTURE_ROOT / scenario / "expected" / name).read_bytes()
                if actual != expected:
                    difference = first_text_difference(
                        expected.decode("utf-8"), actual.decode("utf-8")
                    )
                    summary = text_difference_summary(
                        expected.decode("utf-8"), actual.decode("utf-8")
                    )
                    raise AssertionError(
                        f"{scenario} {name} differs: "
                        f"actual len/SHA {len(actual)}/{sha256_bytes(actual)}, "
                        f"expected len/SHA {len(expected)}/{sha256_bytes(expected)}; "
                        f"{difference}; changes: {summary}"
                    )
                if actual.startswith(b"\xef\xbb\xbf") or b"\r" in actual:
                    raise AssertionError(f"{scenario} {name} is not UTF-8 LF-only")
            extract_graph = (project / "generated" / "extract_graph.txt").read_text(
                "utf-8"
            )
            if scenario == "typed":
                required = (
                    "entity_types: [person, organization]",
                    "One of the following types:",
                )
                if any(value not in extract_graph for value in required):
                    raise AssertionError("typed extract_graph semantic markers missing")
            else:
                if "entity_types: [" in extract_graph:
                    raise AssertionError("untyped extract_graph contains entity_types")
                if (
                    "Suggest several labels or categories for the entity."
                    not in extract_graph
                ):
                    raise AssertionError("untyped extract_graph marker missing")
    finally:
        server.close()


def verify_fixtures(graphloom_bin: Path) -> None:
    """Verify committed evidence and both request-aware GraphLoom replays."""
    graphloom_bin = graphloom_bin.resolve()
    if not graphloom_bin.is_file():
        raise FileNotFoundError(f"GraphLoom binary does not exist: {graphloom_bin}")
    for scenario in SCENARIOS:
        manifest = verify_static_files(scenario)
        run_graphloom_replay(graphloom_bin, scenario)
        print(
            f"{scenario}: {manifest['top_selected_count']} Top chunks, "
            f"{manifest['request_total']} exact requests, outputs match"
        )


def parse_args() -> argparse.Namespace:
    """Parse command-line arguments."""
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--verify",
        action="store_true",
        help="verify committed fixtures (default)",
    )
    mode.add_argument(
        "--update",
        action="store_true",
        help="regenerate fixtures from the fixed GraphRAG commit, then verify",
    )
    parser.add_argument(
        "--graphrag-source",
        type=Path,
        default=Path("../graphrag"),
        help="GraphRAG git repository used only for --update",
    )
    parser.add_argument(
        "--graphloom-bin",
        type=Path,
        default=default_graphloom_bin(),
        help="built GraphLoom binary",
    )
    return parser.parse_args()


def main() -> int:
    """Run update and/or verification."""
    arguments = parse_args()
    graphloom_bin = arguments.graphloom_bin.resolve()
    if arguments.update:
        update_fixtures(arguments.graphrag_source)
    verify_fixtures(graphloom_bin)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
