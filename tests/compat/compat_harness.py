"""Cross-language GraphLoom/GraphRAG compatibility test harness."""

from __future__ import annotations

import hashlib
import json
import math
import os
import re
import shutil
import subprocess
import sys
import threading
from collections import Counter
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Event
from typing import Any

import pandas as pd
import yaml

STANDARD_TABLES = (
    "documents",
    "text_units",
    "entities",
    "relationships",
    "covariates",
    "communities",
    "community_reports",
)
FIXTURE_INPUT = Path(__file__).parent / "fixtures" / "input"


@dataclass(frozen=True)
class CommandResult:
    """Captured subprocess result."""

    returncode: int
    stdout: str
    stderr: str


@dataclass(frozen=True)
class RecordedRequest:
    """Secret-free provider request projection used by Query assertions."""

    operation: str
    endpoint: str
    model: str | None
    message_roles: tuple[str, ...]
    system_prompt: str
    user_message: str
    response_format: Any
    temperature: Any
    top_p: Any
    n: Any
    max_tokens: Any
    max_completion_tokens: Any
    stream: bool
    embedding_input: tuple[str, ...]


@dataclass(frozen=True)
class DelayedStream:
    """Synchronization points for one delayed SSE response."""

    first_delta_sent: Event
    release_remaining: Event


@dataclass(frozen=True)
class CompatibilityRun:
    """Paths and services produced by one paired indexing run."""

    base_project: Path
    graphloom_project: Path
    graphrag_project: Path
    graphloom_bin: Path
    vector_manifest_bin: Path
    server: FixtureModelServer


class FixtureModelServer:
    """Deterministic OpenAI-compatible server shared by both indexers."""

    def __init__(self) -> None:
        self._requests: list[RecordedRequest] = []
        self._delayed_stream: DelayedStream | None = None
        self._lock = threading.Lock()
        handler = self._handler_type()
        self._server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        self._server.fixture = self  # type: ignore[attr-defined]
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)

    @property
    def api_base(self) -> str:
        """Return the provider API base URL."""
        host, port = self._server.server_address
        return f"http://{host}:{port}/v1"

    def start(self) -> None:
        """Start serving requests."""
        self._thread.start()

    def close(self) -> None:
        """Stop the server and join its thread."""
        self._server.shutdown()
        self._server.server_close()
        self._thread.join(timeout=5)

    def snapshot(self) -> Counter[str]:
        """Return request counts by semantic operation."""
        with self._lock:
            return Counter(request.operation for request in self._requests)

    def offset(self) -> int:
        """Return a stable recorder offset for one test phase."""
        with self._lock:
            return len(self._requests)

    def requests_since(self, offset: int) -> tuple[RecordedRequest, ...]:
        """Return requests recorded after ``offset``."""
        with self._lock:
            return tuple(self._requests[offset:])

    def delay_next_stream(self) -> DelayedStream:
        """Delay one streaming response after its first non-empty delta."""
        delayed = DelayedStream(Event(), Event())
        with self._lock:
            if self._delayed_stream is not None:
                raise AssertionError("a delayed stream is already pending")
            self._delayed_stream = delayed
        return delayed

    def _take_delayed_stream(self) -> DelayedStream | None:
        with self._lock:
            delayed = self._delayed_stream
            self._delayed_stream = None
            return delayed

    def _record(
        self,
        operation: str,
        endpoint: str,
        payload: dict[str, Any],
    ) -> None:
        messages = [
            message
            for message in payload.get("messages", [])
            if isinstance(message, dict)
        ]
        roles = tuple(str(message.get("role", "")) for message in messages)
        system_prompt = "\n".join(
            str(message.get("content", ""))
            for message in messages
            if message.get("role") == "system"
        )
        user_message = "\n".join(
            str(message.get("content", ""))
            for message in messages
            if message.get("role") == "user"
        )
        raw_input = payload.get("input", [])
        embedding_input = raw_input if isinstance(raw_input, list) else [raw_input]
        request = RecordedRequest(
            operation=operation,
            endpoint=endpoint,
            model=payload.get("model"),
            message_roles=roles,
            system_prompt=system_prompt,
            user_message=user_message,
            response_format=payload.get("response_format"),
            temperature=payload.get("temperature"),
            top_p=payload.get("top_p"),
            n=payload.get("n"),
            max_tokens=payload.get("max_tokens"),
            max_completion_tokens=payload.get("max_completion_tokens"),
            stream=payload.get("stream") is True,
            embedding_input=tuple(str(value) for value in embedding_input),
        )
        with self._lock:
            self._requests.append(request)

    @staticmethod
    def _handler_type() -> type[BaseHTTPRequestHandler]:
        class Handler(BaseHTTPRequestHandler):
            def do_POST(self) -> None:  # noqa: N802
                length = int(self.headers.get("Content-Length", "0"))
                payload = json.loads(self.rfile.read(length))
                fixture: FixtureModelServer = self.server.fixture  # type: ignore[attr-defined]
                if self.path.endswith("/embeddings"):
                    operation = "embedding"
                    response = _embedding_response(payload)
                elif self.path.endswith("/chat/completions"):
                    operation, content = _completion_content(payload)
                    if payload.get("stream") is True:
                        fixture._record(operation, self.path, payload)
                        events = _streaming_completion_events(payload, content)
                        delayed = fixture._take_delayed_stream()
                        self.send_response(200)
                        self.send_header("Content-Type", "text/event-stream")
                        self.send_header("Cache-Control", "no-cache")
                        if delayed is None:
                            self.send_header(
                                "Content-Length",
                                str(sum(len(event) for event in events)),
                            )
                        else:
                            self.send_header("Connection", "close")
                        self.end_headers()
                        self.wfile.write(events[0])
                        self.wfile.flush()
                        if delayed is not None:
                            delayed.first_delta_sent.set()
                            if not delayed.release_remaining.wait(timeout=15):
                                return
                        for event in events[1:]:
                            self.wfile.write(event)
                            self.wfile.flush()
                        if delayed is not None:
                            self.close_connection = True
                        return
                    response = _completion_response(payload, content)
                else:
                    self.send_error(404, "unsupported fixture endpoint")
                    return
                fixture._record(operation, self.path, payload)
                body = json.dumps(response).encode("utf-8")
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, _format: str, *_args: object) -> None:
                return

        return Handler


def initialize_fixture_project(graphloom_bin: Path, root: Path, api_base: str) -> None:
    """Create one deterministic project template accepted by both implementations."""
    run_command(
        [
            str(graphloom_bin),
            "init",
            "--root",
            str(root),
            "--model",
            "gpt-test",
            "--embedding",
            "embed-test",
        ]
    )
    for source in sorted(FIXTURE_INPUT.glob("*.txt")):
        shutil.copyfile(source, root / "input" / source.name)
    (root / ".env").write_text("GRAPHRAG_API_KEY=compat-test-key\n", encoding="utf-8")
    settings_path = root / "settings.yaml"
    settings = yaml.safe_load(settings_path.read_text(encoding="utf-8"))
    for model in settings["completion_models"].values():
        model["api_base"] = api_base
    for model in settings["embedding_models"].values():
        model["api_base"] = api_base
    settings["concurrent_requests"] = 1
    settings["chunking"]["size"] = 80
    settings["chunking"]["overlap"] = 20
    settings["cluster_graph"]["max_cluster_size"] = 3
    settings["extract_graph"]["max_gleanings"] = 0
    settings["extract_claims"]["enabled"] = True
    settings["extract_claims"]["max_gleanings"] = 0
    settings["vector_store"]["vector_size"] = 4
    settings["snapshots"]["embeddings"] = True
    settings["drift_search"]["n_depth"] = 1
    settings["drift_search"]["primer_folds"] = 1
    settings["drift_search"]["drift_k_followups"] = 1
    settings["drift_search"]["concurrency"] = 1
    settings_path.write_text(
        yaml.safe_dump(settings, sort_keys=False, allow_unicode=True),
        encoding="utf-8",
    )


def clone_project(source: Path, destination: Path) -> None:
    """Clone an unindexed fixture project."""
    shutil.copytree(source, destination)


def replace_cache(project: Path, source_cache: Path) -> None:
    """Replace a project's cache with an unmodified cross-implementation cache."""
    destination = project / "cache"
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(source_cache, destination)


def convert_prompts_for_graphrag(project: Path) -> None:
    """Convert equivalent Tera prompts to Python ``str.format`` syntax."""
    for path in (project / "prompts").glob("*.txt"):
        template = path.read_text(encoding="utf-8")
        variables: list[str] = []

        def reserve(match: re.Match[str]) -> str:
            variables.append(match.group(1))
            return f"__GRAPHLOOM_COMPAT_VARIABLE_{len(variables) - 1}__"

        converted = re.sub(r"\{\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*}}", reserve, template)
        converted = converted.replace("{", "{{").replace("}", "}}")
        for index, variable in enumerate(variables):
            converted = converted.replace(
                f"__GRAPHLOOM_COMPAT_VARIABLE_{index}__",
                f"{{{variable}}}",
            )
        path.write_text(converted, encoding="utf-8")


def run_graphloom_index(graphloom_bin: Path, project: Path) -> CommandResult:
    """Run GraphLoom standard indexing without external connectivity preflight."""
    return run_command(
        [str(graphloom_bin), "index", "--root", str(project), "--skip-validation"],
        cwd=project,
    )


def run_graphrag_index(project: Path) -> CommandResult:
    """Run the pinned GraphRAG standard index in the active Python environment."""
    return run_command(
        [
            sys.executable,
            "-m",
            "graphrag",
            "index",
            "--root",
            str(project),
            "--skip-validation",
        ],
        cwd=project,
    )


def run_graphrag_global_query(project: Path, query: str) -> CommandResult:
    """Run GraphRAG Global Search directly against a GraphLoom output directory."""
    return run_command(
        [
            sys.executable,
            "-m",
            "graphrag",
            "query",
            query,
            "--root",
            str(project),
            "--method",
            "global",
        ],
        cwd=project,
    )


def configure_consumer_vector_db(project: Path, db_uri: Path) -> None:
    """Point a consumer-only project view at its native bridge database."""
    settings_path = project / "settings.yaml"
    settings = yaml.safe_load(settings_path.read_text(encoding="utf-8"))
    settings["vector_store"]["db_uri"] = str(db_uri.resolve())
    settings_path.write_text(
        yaml.safe_dump(settings, sort_keys=False, allow_unicode=True),
        encoding="utf-8",
    )


def run_graphloom_query(
    graphloom_bin: Path,
    project: Path,
    data: Path,
    method: str,
    query: str,
    *,
    streaming: bool,
    dynamic: bool = False,
) -> CommandResult:
    """Run the real GraphLoom Query CLI over producer-owned Parquet."""
    command = [
        str(graphloom_bin),
        "query",
        "--root",
        str(project),
        "--data",
        str(data),
        "--method",
        method,
    ]
    if dynamic:
        command.append("--dynamic-community-selection")
    if streaming:
        command.append("--streaming")
    command.append(query)
    return run_command(command, cwd=project)


def run_graphrag_query(
    project: Path,
    data: Path,
    method: str,
    query: str,
    *,
    streaming: bool,
    dynamic: bool = False,
) -> CommandResult:
    """Run the pinned GraphRAG 3.1.0 Query CLI over producer-owned Parquet."""
    command = [
        sys.executable,
        "-m",
        "graphrag",
        "query",
        "--root",
        str(project),
        "--data",
        str(data),
        "--method",
        method,
    ]
    if dynamic:
        command.append("--dynamic-community-selection")
    if streaming:
        command.append("--streaming")
    command.append(query)
    return run_command(command, cwd=project)


def run_command(command: list[str], cwd: Path | None = None) -> CommandResult:
    """Run one bounded subprocess and retain diagnostics on failure."""
    environment = os.environ.copy()
    environment["GRAPHRAG_API_KEY"] = "compat-test-key"
    try:
        result = subprocess.run(
            command,
            cwd=cwd,
            env=environment,
            check=False,
            capture_output=True,
            text=True,
            timeout=180,
        )
    except subprocess.TimeoutExpired as error:
        raise AssertionError(f"command timed out: {' '.join(command)}") from error
    if result.returncode != 0:
        raise AssertionError(
            f"command failed ({result.returncode}): {' '.join(command)}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return CommandResult(
        returncode=result.returncode,
        stdout=result.stdout,
        stderr=result.stderr,
    )


def load_tables(output: Path) -> dict[str, pd.DataFrame]:
    """Load every standard table through the Python Parquet stack."""
    return {
        name: pd.read_parquet(output / f"{name}.parquet") for name in STANDARD_TABLES
    }


def assert_reference_integrity(tables: dict[str, pd.DataFrame]) -> None:
    """Validate all standard cross-table ID references."""
    ids = {
        name: set(frame["id"].dropna().astype(str)) for name, frame in tables.items()
    }
    _assert_unique_ids(tables)
    _assert_scalar_subset(tables["text_units"], "document_id", ids["documents"])
    _assert_list_subset(tables["documents"], "text_unit_ids", ids["text_units"])
    _assert_list_subset(tables["entities"], "text_unit_ids", ids["text_units"])
    _assert_list_subset(tables["relationships"], "text_unit_ids", ids["text_units"])
    _assert_scalar_subset(tables["covariates"], "text_unit_id", ids["text_units"])
    _assert_list_subset(tables["communities"], "entity_ids", ids["entities"])
    _assert_list_subset(tables["communities"], "relationship_ids", ids["relationships"])
    _assert_list_subset(tables["communities"], "text_unit_ids", ids["text_units"])
    _assert_list_subset(tables["text_units"], "entity_ids", ids["entities"])
    _assert_list_subset(tables["text_units"], "relationship_ids", ids["relationships"])
    _assert_list_subset(tables["text_units"], "covariate_ids", ids["covariates"])
    entity_titles = set(tables["entities"]["title"].dropna().astype(str))
    _assert_scalar_subset(tables["relationships"], "source", entity_titles)
    _assert_scalar_subset(tables["relationships"], "target", entity_titles)
    _assert_scalar_reverse_relation(
        tables["documents"],
        "text_unit_ids",
        tables["text_units"],
        "document_id",
    )
    _assert_list_reverse_relation(
        tables["entities"],
        "text_unit_ids",
        tables["text_units"],
        "entity_ids",
    )
    _assert_list_reverse_relation(
        tables["relationships"],
        "text_unit_ids",
        tables["text_units"],
        "relationship_ids",
    )
    _assert_scalar_reverse_relation(
        tables["text_units"],
        "covariate_ids",
        tables["covariates"],
        "text_unit_id",
    )
    _assert_community_references(tables["communities"], tables["community_reports"])


def canonical_index(output: Path) -> dict[str, list[dict[str, Any]]]:
    """Convert one index to stable, UUID-independent semantic records."""
    tables = load_tables(output)
    text_by_id = _value_map(tables["text_units"], "id", "human_readable_id")
    document_by_id = _value_map(tables["documents"], "id", "human_readable_id")
    entity_by_id = _value_map(tables["entities"], "id", "human_readable_id")
    relationship_by_id = _value_map(
        tables["relationships"],
        "id",
        "human_readable_id",
    )
    covariate_by_id = _value_map(tables["covariates"], "id", "human_readable_id")
    community_by_id = {
        int(row["community"]): _community_identity(row, entity_by_id)
        for _, row in tables["communities"].iterrows()
    }

    result: dict[str, list[dict[str, Any]]] = {}
    result["documents"] = _sorted_records(
        {
            "human_readable_id": row["human_readable_id"],
            "title": row["title"],
            "text": row["text"],
            "text_units": _map_list(row["text_unit_ids"], text_by_id),
            "raw_data": row["raw_data"],
        }
        for _, row in tables["documents"].iterrows()
    )
    result["text_units"] = _sorted_records(
        {
            "human_readable_id": row["human_readable_id"],
            "text": row["text"],
            "n_tokens": row["n_tokens"],
            "document": document_by_id.get(str(row["document_id"])),
            "entities": _map_list(row["entity_ids"], entity_by_id),
            "relationships": _map_list(row["relationship_ids"], relationship_by_id),
            "covariates": _map_list(row["covariate_ids"], covariate_by_id),
        }
        for _, row in tables["text_units"].iterrows()
    )
    result["entities"] = _sorted_records(
        {
            "human_readable_id": row["human_readable_id"],
            "title": row["title"],
            "type": row["type"],
            "description": row["description"],
            "frequency": row["frequency"],
            "degree": row["degree"],
            "text_units": _map_list(row["text_unit_ids"], text_by_id),
        }
        for _, row in tables["entities"].iterrows()
    )
    result["relationships"] = _sorted_records(
        {
            "human_readable_id": row["human_readable_id"],
            "source": row["source"],
            "target": row["target"],
            "description": row["description"],
            "weight": row["weight"],
            "combined_degree": row["combined_degree"],
            "text_units": _map_list(row["text_unit_ids"], text_by_id),
        }
        for _, row in tables["relationships"].iterrows()
    )
    result["covariates"] = _sorted_records(
        {
            key: row[key]
            for key in (
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
            )
        }
        | {"text_unit": text_by_id.get(str(row["text_unit_id"]))}
        for _, row in tables["covariates"].iterrows()
    )
    result["communities"] = _sorted_records(
        {
            "community": community_by_id[int(row["community"])],
            "level": row["level"],
            "parent": _community_parent(row["parent"], community_by_id),
            "children": _community_children(row["children"], community_by_id),
            "title": _canonical_community_title(row["title"]),
            "entities": _map_list(row["entity_ids"], entity_by_id),
            "relationships": _map_list(row["relationship_ids"], relationship_by_id),
            "text_units": _map_list(row["text_unit_ids"], text_by_id),
            "size": row["size"],
        }
        for _, row in tables["communities"].iterrows()
    )
    result["community_reports"] = _sorted_records(
        {
            "community": community_by_id[int(row["community"])],
            "level": row["level"],
            "parent": _community_parent(row["parent"], community_by_id),
            "children": _community_children(row["children"], community_by_id),
            "title": row["title"],
            "summary": row["summary"],
            "full_content": row["full_content"],
            "rank": row["rank"],
            "rating_explanation": row["rating_explanation"],
            "findings": row["findings"],
            "full_content_json": (
                json.loads(row["full_content_json"])
                if isinstance(row["full_content_json"], str)
                else row["full_content_json"]
            ),
            "size": row["size"],
        }
        for _, row in tables["community_reports"].iterrows()
    )
    return result


def _completion_content(payload: dict[str, Any]) -> tuple[str, str]:
    messages = payload.get("messages", [])
    prompt = "\n".join(
        str(message.get("content", ""))
        for message in messages
        if isinstance(message, dict)
    )
    if "This is an LLM connectivity test" in prompt:
        return "connectivity", "Hello World"
    if "Given a text document" in prompt and "identify all entities" in prompt:
        return (
            "extract_graph",
            '("entity"<|>西门庆<|>person<|>西门庆在清河县召集众人结义)##'
            '("entity"<|>应伯爵<|>person<|>应伯爵参加西门庆组织的结义)##'
            '("entity"<|>玉皇庙<|>organization<|>玉皇庙是众人结义的地点)##'
            '("entity"<|>花子虚<|>person<|>花子虚参加玉皇庙结义)##'
            '("entity"<|>武松<|>person<|>武松在清河县担任巡捕都头)##'
            '("entity"<|>武大<|>person<|>武大是武松的哥哥)##'
            '("entity"<|>潘金莲<|>person<|>潘金莲是武大的妻子)##'
            '("entity"<|>知县<|>person<|>知县任命武松为巡捕都头)##'
            '("relationship"<|>西门庆<|>应伯爵<|>二人一同组织结义<|>8)##'
            '("relationship"<|>应伯爵<|>玉皇庙<|>应伯爵前往玉皇庙结义<|>7)##'
            '("relationship"<|>玉皇庙<|>花子虚<|>花子虚前往玉皇庙结义<|>7)##'
            '("relationship"<|>花子虚<|>西门庆<|>西门庆邀请花子虚结义<|>7)##'
            '("relationship"<|>武松<|>武大<|>二人是亲兄弟<|>9)##'
            '("relationship"<|>武大<|>潘金莲<|>二人是夫妻<|>9)##'
            '("relationship"<|>潘金莲<|>知县<|>二人都在清河县生活<|>2)##'
            '("relationship"<|>知县<|>武松<|>知县任命武松为都头<|>8)##'
            '("relationship"<|>西门庆<|>潘金莲<|>二人同在清河县生活<|>1)##'
            "<|COMPLETE|>",
        )
    if "Target activity" in prompt and "extract all claims" in prompt:
        return (
            "extract_claims",
            "(武松<|>NONE<|>APPOINTMENT<|>TRUE<|>NONE<|>NONE<|>"
            "武松担任清河县巡捕都头<|>知县任命武松为巡捕都头。)<|COMPLETE|>",
        )
    if "well-formed JSON-formatted string" in prompt:
        return "community_report", _community_report(prompt)
    if "Create a hypothetical answer to the following query" in prompt:
        return "drift_hyde", "A hypothetical answer connecting the fixture characters."
    if "top-ranked community summaries" in prompt:
        return (
            "drift_primer",
            json.dumps(
                {
                    "intermediate_answer": (
                        "Producer community reports connect the fixture characters."
                    ),
                    "score": 85,
                    "follow_up_queries": [
                        "How are 西门庆 and 武松 connected through 清河县?"
                    ],
                }
            ),
        )
    if "how relevant or helpful is the provided information" in prompt:
        return (
            "dynamic_rating",
            json.dumps(
                {
                    "reason": "The producer community report is relevant.",
                    "rating": 5,
                }
            ),
        )
    if "---Data Reports---" in prompt:
        return "drift_reduce", "DRIFT interoperable answer."
    if "'follow_up_queries': List[str]" in prompt and "'score': int" in prompt:
        return (
            "drift_action",
            json.dumps(
                {
                    "response": "Producer entity and source evidence was retrieved.",
                    "score": 90,
                    "follow_up_queries": [],
                }
            ),
        )
    if "list of key points" in prompt and '"points"' in prompt:
        return (
            "global_search_map",
            json.dumps(
                {
                    "points": [
                        {
                            "description": "西门庆与武松通过清河县人物网络间接相连 "
                            "[Data: Reports (0)].",
                            "score": 90,
                        }
                    ]
                }
            ),
        )
    if "multiple analysts" in prompt:
        return "global_search_reduce", "Global interoperable answer."
    if "---Data tables---" in prompt and "Sources" in prompt:
        if "Relationships" in prompt and "Entities" in prompt:
            return "local_search", "Local interoperable answer."
        return "basic_search", "Basic interoperable answer."
    return "summarize_descriptions", "A concise summary of the provided descriptions."


def _community_report(prompt: str) -> str:
    members = [
        name
        for name in (
            "西门庆",
            "应伯爵",
            "玉皇庙",
            "花子虚",
            "武松",
            "武大",
            "潘金莲",
            "知县",
        )
        if name in prompt
    ]
    member_title = "、".join(members) if members else "清河县人物"
    signature = hashlib.sha256(prompt.encode("utf-8")).hexdigest()[:12]
    title = f"{member_title} {signature}"
    return json.dumps(
        {
            "title": title,
            "summary": f"{title}通过亲属或社会关系形成社群。",
            "rating": 5.0,
            "rating_explanation": "The community is small but coherent.",
            "findings": [
                {
                    "summary": f"{title}的人物联系",
                    "explanation": f"{title}中的人物相互联系 "
                    "[Data: Entities (0, 1); Relationships (0)].",
                }
            ],
        }
    )


def _completion_response(payload: dict[str, Any], content: str) -> dict[str, Any]:
    return {
        "id": "chatcmpl-compat",
        "object": "chat.completion",
        "created": 0,
        "model": payload.get("model", "gpt-test"),
        "choices": [
            {
                "index": 0,
                "message": {"role": "assistant", "content": content},
                "finish_reason": "stop",
            }
        ],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
    }


def _streaming_completion_events(
    payload: dict[str, Any],
    content: str,
) -> list[bytes]:
    model = payload.get("model", "gpt-test")
    split_at = max(1, len(content) // 2)
    content_parts = [content[:split_at], content[split_at:]]
    chunks = [
        {
            "id": "chatcmpl-compat",
            "object": "chat.completion.chunk",
            "created": 0,
            "model": model,
            "choices": [
                {
                    "index": 0,
                    "delta": {"role": "assistant", "content": content_part},
                    "finish_reason": None,
                }
            ],
        }
        for content_part in content_parts
        if content_part
    ] + [
        {
            "id": "chatcmpl-compat",
            "object": "chat.completion.chunk",
            "created": 0,
            "model": model,
            "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
        }
    ]
    events = [f"data: {json.dumps(chunk)}\n\n".encode() for chunk in chunks]
    events.append(b"data: [DONE]\n\n")
    return events


def _embedding_response(payload: dict[str, Any]) -> dict[str, Any]:
    raw_inputs = payload.get("input", [])
    inputs = raw_inputs if isinstance(raw_inputs, list) else [raw_inputs]
    data = []
    for index, value in enumerate(inputs):
        digest = hashlib.sha256(str(value).encode("utf-8")).digest()
        vector = [1.0, digest[0] / 255.0, digest[1] / 255.0, digest[2] / 255.0]
        data.append({"object": "embedding", "index": index, "embedding": vector})
    return {
        "object": "list",
        "data": data,
        "model": payload.get("model", "embed-test"),
        "usage": {"prompt_tokens": len(inputs), "total_tokens": len(inputs)},
    }


def _assert_unique_ids(tables: dict[str, pd.DataFrame]) -> None:
    for name, frame in tables.items():
        for column in ("id", "human_readable_id"):
            values = frame[column].dropna().astype(str)
            if len(values) != len(frame):
                raise AssertionError(f"{name}.{column} contains null values")
            if values.duplicated().any():
                raise AssertionError(f"{name}.{column} contains duplicates")


def _assert_scalar_subset(frame: pd.DataFrame, column: str, expected: set[str]) -> None:
    actual = {str(value) for value in frame[column].dropna()}
    missing = actual - expected
    if missing:
        raise AssertionError(
            f"{column} contains dangling references: {sorted(missing)}"
        )


def _assert_list_subset(frame: pd.DataFrame, column: str, expected: set[str]) -> None:
    for values in frame[column]:
        items = [str(value) for value in _as_list(values) if value is not None]
        if len(items) != len(set(items)):
            raise AssertionError(f"{column} contains duplicate references")
    actual = {
        str(value)
        for values in frame[column]
        for value in _as_list(values)
        if value is not None
    }
    missing = actual - expected
    if missing:
        raise AssertionError(
            f"{column} contains dangling references: {sorted(missing)}"
        )


def _assert_scalar_reverse_relation(
    left: pd.DataFrame,
    left_ids_column: str,
    right: pd.DataFrame,
    right_parent_column: str,
) -> None:
    left_pairs = {
        (str(row["id"]), str(right_id))
        for _, row in left.iterrows()
        for right_id in _as_list(row[left_ids_column])
    }
    right_pairs = {
        (str(row[right_parent_column]), str(row["id"]))
        for _, row in right.iterrows()
        if not pd.isna(row[right_parent_column])
    }
    if left_pairs != right_pairs:
        raise AssertionError(
            f"{left_ids_column} and {right_parent_column} are not bidirectional"
        )


def _assert_list_reverse_relation(
    left: pd.DataFrame,
    left_ids_column: str,
    right: pd.DataFrame,
    right_ids_column: str,
) -> None:
    left_pairs = {
        (str(row["id"]), str(right_id))
        for _, row in left.iterrows()
        for right_id in _as_list(row[left_ids_column])
    }
    right_pairs = {
        (str(left_id), str(row["id"]))
        for _, row in right.iterrows()
        for left_id in _as_list(row[right_ids_column])
    }
    if left_pairs != right_pairs:
        raise AssertionError(
            f"{left_ids_column} and {right_ids_column} are not bidirectional"
        )


def _assert_community_references(
    communities: pd.DataFrame,
    reports: pd.DataFrame,
) -> None:
    community_rows = {int(row["community"]): row for _, row in communities.iterrows()}
    if len(community_rows) != len(communities):
        raise AssertionError("communities.community contains duplicates")
    community_ids = set(community_rows)
    for community, row in community_rows.items():
        parent = int(row["parent"])
        child_values = [int(value) for value in _as_list(row["children"])]
        children = set(child_values)
        if len(child_values) != len(children):
            raise AssertionError(f"community {community} contains duplicate children")
        if parent != -1 and parent not in community_ids:
            raise AssertionError(f"community {community} has missing parent {parent}")
        missing_children = children - community_ids
        if missing_children:
            raise AssertionError(
                f"community {community} has missing children {sorted(missing_children)}"
            )
        for child in children:
            if int(community_rows[child]["parent"]) != community:
                raise AssertionError(
                    f"community {child} does not point back to parent {community}"
                )
        if parent != -1 and community not in {
            int(value) for value in _as_list(community_rows[parent]["children"])
        }:
            raise AssertionError(
                f"parent community {parent} does not contain child {community}"
            )

    report_communities = set(reports["community"].astype(int))
    if len(report_communities) != len(reports):
        raise AssertionError("community_reports.community contains duplicates")
    if report_communities != community_ids:
        raise AssertionError("community reports do not cover every community")
    for _, report in reports.iterrows():
        community = int(report["community"])
        source = community_rows[community]
        for column in ("level", "parent"):
            if int(report[column]) != int(source[column]):
                raise AssertionError(
                    f"community report {community} has mismatched {column}"
                )
        report_child_values = [int(value) for value in _as_list(report["children"])]
        report_children = set(report_child_values)
        if len(report_children) != len(report_child_values):
            raise AssertionError(
                f"community report {community} contains duplicate children"
            )
        source_children = {int(value) for value in _as_list(source["children"])}
        if report_children != source_children:
            raise AssertionError(
                f"community report {community} has mismatched children"
            )


def _value_map(frame: pd.DataFrame, key: str, value: str) -> dict[str, Any]:
    return {str(row[key]): _normalize(row[value]) for _, row in frame.iterrows()}


def _community_identity(row: pd.Series, entity_by_id: dict[str, Any]) -> dict[str, Any]:
    return {
        "level": _normalize(row["level"]),
        "entities": _map_list(row["entity_ids"], entity_by_id),
    }


def _community_parent(
    parent: Any,
    community_by_id: dict[int, dict[str, Any]],
) -> dict[str, Any] | None:
    parent_id = int(parent)
    return None if parent_id == -1 else community_by_id[parent_id]


def _community_children(
    children: Any,
    community_by_id: dict[int, dict[str, Any]],
) -> list[dict[str, Any]]:
    return sorted(
        (community_by_id[int(child)] for child in _as_list(children)),
        key=_json_key,
    )


def _canonical_community_title(title: Any) -> Any:
    if isinstance(title, str) and re.fullmatch(r"Community \d+", title):
        return "Community <opaque>"
    return title


def _map_list(value: Any, lookup: dict[str, Any]) -> list[Any]:
    return sorted(
        (
            _normalize(lookup.get(str(item), f"<missing:{item}>"))
            for item in _as_list(value)
        ),
        key=_json_key,
    )


def _as_list(value: Any) -> list[Any]:
    if value is None:
        return []
    if isinstance(value, (list, tuple)):
        return list(value)
    if hasattr(value, "tolist"):
        converted = value.tolist()
        return converted if isinstance(converted, list) else [converted]
    try:
        if bool(pd.isna(value)):
            return []
    except (TypeError, ValueError):
        pass
    raise AssertionError(
        f"expected list-like value, found {type(value).__name__}: {value!r}"
    )


def _normalize(value: Any) -> Any:
    if isinstance(value, dict):
        return {str(key): _normalize(item) for key, item in sorted(value.items())}
    if isinstance(value, (list, tuple)):
        return [_normalize(item) for item in value]
    if hasattr(value, "tolist") and not isinstance(value, (str, bytes)):
        return _normalize(value.tolist())
    if isinstance(value, float):
        if math.isnan(value):
            return None
        return round(value, 8)
    if value is None:
        return None
    try:
        if bool(pd.isna(value)):
            return None
    except (TypeError, ValueError):
        pass
    if hasattr(value, "item"):
        return _normalize(value.item())
    return value


def _sorted_records(records: Any) -> list[dict[str, Any]]:
    normalized = [_normalize(record) for record in records]
    return sorted(normalized, key=_json_key)


def _json_key(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
