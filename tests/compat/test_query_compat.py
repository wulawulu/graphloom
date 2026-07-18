"""GraphRAG 3.1.0 Query compatibility goldens.

The fixtures in this module exercise the fixed upstream package directly. Rust
tests use the same identifiers, ordering, and exact context snapshot.
"""

import asyncio
import importlib.metadata
import inspect
import json
import site
import sys
import sysconfig
from collections.abc import AsyncIterator
from io import StringIO
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import pytest
import pandas as pd
import graphrag
from graphrag.data_model.community_report import CommunityReport
from graphrag.data_model.community import Community
from graphrag.data_model.covariate import Covariate
from graphrag.data_model.entity import Entity
from graphrag.data_model.relationship import Relationship
from graphrag.data_model.text_unit import TextUnit
from graphrag.prompts.query.local_search_system_prompt import (
    LOCAL_SEARCH_SYSTEM_PROMPT,
)
from graphrag.prompts.query.global_search_map_system_prompt import (
    MAP_SYSTEM_PROMPT,
)
from graphrag.prompts.query.global_search_reduce_system_prompt import (
    REDUCE_SYSTEM_PROMPT,
)
from graphrag.query.context_builder.builders import ContextBuilderResult
from graphrag.query.context_builder.conversation_history import (
    ConversationHistory,
    ConversationRole,
)
from graphrag.query.context_builder.local_context import (
    build_covariates_context,
    build_entity_context,
    build_relationship_context,
)
from graphrag.query.context_builder.community_context import (
    build_community_context,
)
from graphrag.query.context_builder.dynamic_community_selection import (
    DynamicCommunitySelection,
)
from graphrag.query.context_builder.rate_prompt import RATE_QUERY
from graphrag.query.context_builder.rate_relevancy import rate_relevancy
from graphrag.query.context_builder.source_context import build_text_unit_context
from graphrag.query.input.retrieval.entities import get_entity_by_id
from graphrag.query.structured_search.local_search.mixed_context import (
    LocalSearchMixedContext,
)
from graphrag.query.structured_search.local_search.search import LocalSearch
from graphrag.query.structured_search.global_search.search import GlobalSearch
from graphrag.query.structured_search.drift_search.search import DRIFTSearch
from graphrag.query.structured_search.drift_search.drift_context import (
    DRIFTSearchContextBuilder,
)
from graphrag.query.structured_search.drift_search.primer import (
    DRIFTPrimer,
    PrimerQueryProcessor,
    PrimerResponse,
)
from graphrag.query.structured_search.drift_search.state import QueryState
from graphrag.query.structured_search.drift_search.action import DriftAction
from graphrag.query.llm.text_utils import try_parse_json_object
from graphrag.prompts.query.drift_search_system_prompt import DRIFT_PRIMER_PROMPT


LOCAL_CONTEXT = (
    Path(__file__).parent / "fixtures" / "query" / "local_context.txt"
).read_text(encoding="utf-8")
GLOBAL_BATCHES = json.loads(
    (Path(__file__).parent / "fixtures" / "query" / "global_batches.json").read_text(
        encoding="utf-8"
    )
)
LOCAL_SPECIAL_CHARACTERS = json.loads(
    (
        Path(__file__).parent / "fixtures" / "query" / "local_special_characters.json"
    ).read_text(encoding="utf-8")
)
REPORT_CSV_SPECIAL_CHARACTERS = json.loads(
    (
        Path(__file__).parent
        / "fixtures"
        / "query"
        / "report_csv_special_characters.json"
    ).read_text(encoding="utf-8")
)
DRIFT_HYDE_PROMPT = (
    (Path(__file__).parent / "fixtures" / "query" / "drift_hyde_prompt.txt")
    .read_text(encoding="utf-8")
    .removesuffix("\n")
)


def _is_relative_to(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
    except ValueError:
        return False
    return True


@pytest.fixture(scope="session", autouse=True)
def require_isolated_graphrag_distribution() -> None:
    """Reject editable or neighboring source-tree GraphRAG imports."""
    version = importlib.metadata.version("graphrag")
    distribution = importlib.metadata.distribution("graphrag")
    package_path = Path(inspect.getfile(graphrag)).resolve()
    local_path = Path(inspect.getfile(LocalSearchMixedContext)).resolve()
    dynamic_path = Path(inspect.getfile(DynamicCommunitySelection)).resolve()
    drift_search_path = Path(inspect.getfile(DRIFTSearch)).resolve()
    drift_context_path = Path(inspect.getfile(DRIFTSearchContextBuilder)).resolve()
    drift_primer_path = Path(inspect.getfile(DRIFTPrimer)).resolve()
    drift_state_path = Path(inspect.getfile(QueryState)).resolve()
    module_paths = [
        package_path,
        local_path,
        dynamic_path,
        drift_search_path,
        drift_context_path,
        drift_primer_path,
        drift_state_path,
    ]
    configured_roots = {
        Path(value).resolve()
        for value in [
            *site.getsitepackages(),
            site.getusersitepackages(),
            sysconfig.get_paths().get("purelib"),
            sysconfig.get_paths().get("platlib"),
        ]
        if value
    }
    installed_package = Path(distribution.locate_file("graphrag")).resolve()
    direct_url_text = distribution.read_text("direct_url.json")
    direct_url = json.loads(direct_url_text) if direct_url_text else {}
    editable = direct_url.get("dir_info", {}).get("editable", False)
    local_direct_url = str(direct_url.get("url", "")).startswith("file:")
    repository_root = Path(__file__).resolve().parents[2]
    neighboring_source_root = repository_root.parent
    from_neighboring_source = any(
        _is_relative_to(path, neighboring_source_root)
        and not any(_is_relative_to(path, root) for root in configured_roots)
        for path in module_paths
    )
    modules_match_distribution = all(
        _is_relative_to(path, installed_package) for path in module_paths
    )
    diagnostic = (
        f"graphrag version: {version}\n"
        f"graphrag package path: {package_path}\n"
        f"LocalSearchMixedContext path: {local_path}\n"
        f"DynamicCommunitySelection path: {dynamic_path}\n"
        f"DRIFTSearch path: {drift_search_path}\n"
        f"DRIFTSearchContextBuilder path: {drift_context_path}\n"
        f"DRIFTPrimer path: {drift_primer_path}\n"
        f"QueryState path: {drift_state_path}\n"
        f"distribution package path: {installed_package}\n"
        f"direct_url.json: {direct_url}\n"
        f"site-package roots: {sorted(map(str, configured_roots))}\n"
        f"sys.path: {sys.path}"
    )
    assert version == "3.1.0", diagnostic
    assert not editable and not local_direct_url, diagnostic
    assert not from_neighboring_source, diagnostic
    assert modules_match_distribution, diagnostic


class ByteTokenizer:
    """Deterministic tokenizer used on both sides of the golden."""

    def encode(self, text: str) -> list[int]:
        return list(text.encode())

    def num_tokens(self, text: str) -> int:
        return len(text.encode())

    def num_prompt_tokens(self, messages: list[Any]) -> int:
        return sum(len(message["content"].encode()) for message in messages)


def test_should_load_pypi_graphrag_3_1_0_from_isolated_environment() -> None:
    """Expose the distribution assertion as an explicit compatibility test."""
    assert importlib.metadata.version("graphrag") == "3.1.0"


class RecordingEmbedding:
    """Synchronous GraphRAG embedding facade."""

    def __init__(self) -> None:
        self.inputs: list[list[str]] = []

    def embedding(self, input: list[str]) -> SimpleNamespace:
        self.inputs.append(input)
        return SimpleNamespace(first_embedding=[0.2, 0.8])


class RecordingVectorStore:
    """Minimal vector store exposing GraphRAG's text-search callback."""

    def __init__(self) -> None:
        self.calls: list[tuple[str, list[float], int]] = []

    def similarity_search_by_text(
        self,
        text: str,
        text_embedder: Any,
        k: int,
    ) -> list[SimpleNamespace]:
        vector = text_embedder(text)
        self.calls.append((text, vector, k))
        return [
            SimpleNamespace(document=SimpleNamespace(id="entity-a")),
            SimpleNamespace(document=SimpleNamespace(id="entity-b")),
        ]


class RecordingCompletion:
    """Scripted streaming completion that records canonical inputs."""

    def __init__(self) -> None:
        self.requests: list[tuple[list[Any], dict[str, Any]]] = []

    async def completion_async(
        self,
        messages: list[Any],
        **kwargs: Any,
    ) -> AsyncIterator[SimpleNamespace]:
        self.requests.append((messages, kwargs))

        async def chunks() -> AsyncIterator[SimpleNamespace]:
            for text in ("Local ", "answer."):
                yield SimpleNamespace(
                    choices=[
                        SimpleNamespace(
                            delta=SimpleNamespace(content=text),
                        )
                    ]
                )

        return chunks()


class RecordingDriftCompletion:
    """Non-streaming DRIFT completion recorder with scripted structured output."""

    def __init__(self, responses: list[tuple[str, Any | None]]) -> None:
        self.responses = responses
        self.requests: list[tuple[Any, dict[str, Any]]] = []
        self.tokenizer = ByteTokenizer()

    async def completion_async(self, messages: Any, **kwargs: Any) -> SimpleNamespace:
        self.requests.append((messages, kwargs))
        content, formatted = self.responses.pop(0)
        return SimpleNamespace(content=content, formatted_response=formatted)


class RecordingCallbacks:
    """Capture the GraphRAG streaming callback lifecycle."""

    def __init__(self) -> None:
        self.events: list[str] = []

    def on_context(self, _context: Any) -> None:
        self.events.append("context")

    def on_llm_new_token(self, token: str) -> None:
        self.events.append(f"token:{token}")


class FixedGlobalContext:
    """Two fixed map batches shared with the Rust Global golden."""

    async def build_context(
        self,
        query: str,
        conversation_history: Any = None,
        **kwargs: Any,
    ) -> ContextBuilderResult:
        del query, conversation_history, kwargs
        return ContextBuilderResult(
            context_chunks=GLOBAL_BATCHES,
            context_records={"reports": pd.DataFrame({"id": ["3", "1", "2", "0"]})},
        )


class RecordingGlobalCompletion:
    """Scripted map/reduce completion facade with canonical request capture."""

    def __init__(self) -> None:
        self.requests: list[tuple[list[Any], dict[str, Any]]] = []

    async def completion_async(
        self,
        messages: list[Any],
        **kwargs: Any,
    ) -> AsyncIterator[SimpleNamespace]:
        self.requests.append((messages, kwargs))
        if kwargs.get("response_format_json_object"):
            context = messages[0]["content"]
            response = (
                '{"points":[{"description":"first tie","score":5}]}'
                if "Report 3" in context
                else (
                    '{"points":[{"description":"best","score":9},'
                    '{"description":"second tie","score":5}]}'
                )
            )
            chunks = (response,)
        else:
            chunks = ("Global ", "answer.")

        async def response_chunks() -> AsyncIterator[SimpleNamespace]:
            for text in chunks:
                yield SimpleNamespace(
                    choices=[
                        SimpleNamespace(
                            delta=SimpleNamespace(content=text),
                        )
                    ]
                )

        return response_chunks()


class RecordingGlobalCallbacks(RecordingCallbacks):
    """Capture map/context lifecycle in addition to provider tokens."""

    def on_map_response_start(self, contexts: list[str]) -> None:
        self.events.append(f"map_start:{len(contexts)}")

    def on_map_response_end(self, outputs: list[Any]) -> None:
        self.events.append(f"map_end:{len(outputs)}")

    def on_context(self, _context: Any) -> None:
        self.events.append("context")


class RecordingDynamicCompletion:
    """Script dynamic ratings by description marker and record every request."""

    def __init__(self, scripts: dict[str, list[str]]) -> None:
        self.scripts = {marker: list(values) for marker, values in scripts.items()}
        self.requests: list[tuple[list[Any], dict[str, Any]]] = []

    async def completion_async(
        self,
        messages: list[Any],
        **kwargs: Any,
    ) -> AsyncIterator[SimpleNamespace]:
        self.requests.append((messages, kwargs))
        system = messages[0]["content"]
        marker = next(marker for marker in self.scripts if marker in system)
        response = self.scripts[marker].pop(0)

        async def response_chunks() -> AsyncIterator[SimpleNamespace]:
            yield SimpleNamespace(
                choices=[
                    SimpleNamespace(
                        delta=SimpleNamespace(content=response),
                    )
                ]
            )

        return response_chunks()


def _entity(
    entity_id: str,
    short_id: str,
    title: str,
    rank: int,
    communities: list[str],
    text_units: list[str],
) -> Entity:
    return Entity(
        id=entity_id,
        short_id=short_id,
        title=title,
        description=f"{title} description",
        community_ids=communities,
        text_unit_ids=text_units,
        rank=rank,
    )


def _report(community: str, rank: float, content: str) -> CommunityReport:
    return CommunityReport(
        id=f"report-{community}",
        short_id=community,
        title=f"Report {community}",
        community_id=community,
        summary=f"Summary {community}",
        full_content=content,
        rank=rank,
    )


def _relationship(
    relationship_id: str,
    short_id: str,
    source: str,
    target: str,
    rank: int,
    weight: float,
    text_units: list[str],
) -> Relationship:
    return Relationship(
        id=relationship_id,
        short_id=short_id,
        source=source,
        target=target,
        description=f"{source} to {target}",
        rank=rank,
        weight=weight,
        text_unit_ids=text_units,
    )


def _text_unit(
    unit_id: str,
    short_id: str,
    text: str,
    relationships: list[str],
) -> TextUnit:
    return TextUnit(
        id=unit_id,
        short_id=short_id,
        text=text,
        relationship_ids=relationships,
    )


def _covariate(
    covariate_id: str,
    short_id: str,
    subject: str,
    description: str,
) -> Covariate:
    return Covariate(
        id=covariate_id,
        short_id=short_id,
        subject_id=subject,
        attributes={
            "object_id": None,
            "status": "TRUE",
            "start_date": None,
            "end_date": None,
            "description": description,
        },
    )


def _fixture() -> tuple[
    LocalSearchMixedContext,
    RecordingEmbedding,
    RecordingVectorStore,
    ConversationHistory,
]:
    entities = [
        _entity(
            "entity-a",
            "0",
            "Alice",
            5,
            ["1", "2"],
            ["tu-a", "missing"],
        ),
        _entity(
            "entity-b",
            "1",
            "Bob",
            4,
            ["2"],
            ["tu-b", "tu-shared"],
        ),
        _entity(
            "entity-c",
            "2",
            "Carol",
            3,
            ["3"],
            ["tu-c", "tu-shared"],
        ),
    ]
    reports = [
        _report("1", 8.0, "Alpha report"),
        _report("2", 5.0, "Shared report"),
        _report("3", 9.0, "Carol report"),
    ]
    relationships = [
        _relationship(
            "rel-ab",
            "0",
            "Alice",
            "Bob",
            9,
            1.5,
            ["tu-a", "tu-b"],
        ),
        _relationship(
            "rel-ax",
            "1",
            "Alice",
            "External",
            7,
            0.0,
            ["tu-a"],
        ),
        _relationship("rel-bx", "2", "Bob", "External", 6, 2.0, []),
        _relationship("rel-ay", "3", "Alice", "Other", 8, 3.0, []),
    ]
    text_units = [
        _text_unit("tu-a", "0", "Alice source", ["rel-ab", "rel-ax"]),
        _text_unit("tu-b", "1", "Bob source", ["rel-ab"]),
        _text_unit("tu-c", "2", "Carol source", []),
        _text_unit("tu-shared", "3", "Shared source", ["rel-ab"]),
    ]
    embedding = RecordingEmbedding()
    store = RecordingVectorStore()
    history = ConversationHistory()
    history.add_turn(ConversationRole.USER, "old question")
    history.add_turn(ConversationRole.ASSISTANT, "old answer")
    history.add_turn(ConversationRole.USER, "new question")
    history.add_turn(ConversationRole.ASSISTANT, "new answer")
    return (
        LocalSearchMixedContext(
            entities=entities,
            entity_text_embeddings=store,
            text_embedder=embedding,
            text_units=text_units,
            community_reports=reports,
            relationships=relationships,
            covariates={
                "claims": [
                    _covariate(
                        "claim-1",
                        "10",
                        "Alice",
                        "Alice claim",
                    )
                ],
                "facts": [
                    _covariate(
                        "fact-1",
                        "11",
                        "Bob",
                        "Bob fact",
                    )
                ],
            },
            tokenizer=ByteTokenizer(),
        ),
        embedding,
        store,
        history,
    )


def _context_params() -> dict[str, Any]:
    return {
        "conversation_history_max_turns": 5,
        "conversation_history_user_turns_only": True,
        "max_context_tokens": 20_000,
        "text_unit_prop": 0.3,
        "community_prop": 0.2,
        "top_k_mapped_entities": 2,
        "top_k_relationships": 1,
        "include_entity_rank": True,
        "include_relationship_weight": True,
        "include_community_rank": False,
        "return_candidate_context": False,
    }


def test_graphrag_3_1_local_context_golden() -> None:
    """Lock mapping, table shapes, ordering, and exact mixed context."""
    builder, embedding, store, history = _fixture()

    result = builder.build_context(
        query="current",
        conversation_history=history,
        **_context_params(),
    )

    assert embedding.inputs == [["current\nnew question\nold question"]]
    assert store.calls == [
        (
            "current\nnew question\nold question",
            [0.2, 0.8],
            4,
        )
    ]
    assert result.context_chunks == LOCAL_CONTEXT
    assert list(result.context_records) == [
        "conversation history",
        "reports",
        "relationships",
        "claims",
        "facts",
        "entities",
        "sources",
    ]
    # PyPI 3.1.0's mixed builder adds this marker even when candidate context is
    # disabled. The lower-level Local helpers do not; the dedicated raw-helper
    # golden below locks those unmodified standard tables for GraphLoom.
    assert {
        key: list(frame.columns) for key, frame in result.context_records.items()
    } == {
        "conversation history": ["turn", "content"],
        "reports": ["id", "title", "content"],
        "relationships": [
            "id",
            "source",
            "target",
            "description",
            "weight",
            "links",
            "in_context",
        ],
        "claims": [
            "id",
            "entity",
            "object_id",
            "status",
            "start_date",
            "end_date",
            "description",
            "in_context",
        ],
        "facts": [
            "id",
            "entity",
            "object_id",
            "status",
            "start_date",
            "end_date",
            "description",
            "in_context",
        ],
        "entities": [
            "id",
            "entity",
            "description",
            "number of relationships",
            "in_context",
        ],
        "sources": ["id", "text"],
    }
    for key in ("entities", "relationships", "claims", "facts"):
        values = result.context_records[key]["in_context"]
        assert str(values.dtype) == "bool"
        assert values.tolist() == [True] * len(values)
    for key in ("conversation history", "reports", "sources"):
        assert "in_context" not in result.context_records[key].columns


def test_graphrag_3_1_local_raw_special_character_helpers_golden() -> None:
    """Lock raw helper text/records before mixed metadata adds in_context."""
    entity = _entity("entity-a", "0", "Alice", 5, [], ["tu-a"])
    entity.description = 'Alice|Bob "quoted" \\path\nsecond line'
    relationship = _relationship(
        "rel-ab",
        "0",
        "Alice",
        "Bob",
        9,
        1.5,
        ["tu-a"],
    )
    relationship.description = 'A|B "rel" \\edge\r\nnext'
    covariate = _covariate(
        "claim-1",
        "10",
        "Alice",
        'claim|text "quoted" \\claim\nnext',
    )
    source = _text_unit(
        "tu-a",
        "0",
        'source|text "quoted" \\source\r\nnext',
        ["rel-ab"],
    )
    tokenizer = ByteTokenizer()

    entity_text, entity_records = build_entity_context(
        selected_entities=[entity],
        tokenizer=tokenizer,
        max_context_tokens=20_000,
        include_entity_rank=True,
        rank_description="number of relationships",
    )
    relationship_text, relationship_records = build_relationship_context(
        selected_entities=[entity],
        relationships=[relationship],
        tokenizer=tokenizer,
        include_relationship_weight=True,
        max_context_tokens=20_000,
        top_k_relationships=1,
        relationship_ranking_attribute="rank",
    )
    claim_text, claim_records = build_covariates_context(
        selected_entities=[entity],
        covariates=[covariate],
        tokenizer=tokenizer,
        max_context_tokens=20_000,
        context_name="claims",
    )
    source_text, source_record_map = build_text_unit_context(
        text_units=[source],
        tokenizer=tokenizer,
        shuffle_data=False,
        max_context_tokens=20_000,
    )
    frames = {
        "entities": entity_records,
        "relationships": relationship_records,
        "claims": claim_records,
        "sources": source_record_map["sources"],
    }
    context = "\n\n".join([entity_text, relationship_text, claim_text, source_text])
    records = {
        name: {
            "columns": list(frame.columns),
            "rows": (frame.astype(object).where(frame.notna(), None).values.tolist()),
        }
        for name, frame in frames.items()
    }

    assert context == LOCAL_SPECIAL_CHARACTERS["context"]
    assert records == LOCAL_SPECIAL_CHARACTERS["records"]
    assert all("in_context" not in frame.columns for frame in frames.values())

    second_source = _text_unit("tu-b", "1", "second source", [])
    expected_source = (
        '-----Sources-----\nid|text\n0|source|text "quoted" \\source\r\nnext\n'
    )
    cutoff_text, cutoff_records = build_text_unit_context(
        text_units=[source, second_source],
        tokenizer=tokenizer,
        shuffle_data=False,
        max_context_tokens=len(expected_source.encode()),
    )
    assert cutoff_text == expected_source
    assert cutoff_records["sources"].to_dict(orient="records") == [
        {
            "id": "0",
            "text": 'source|text "quoted" \\source\r\nnext',
        }
    ]


def test_graphrag_3_1_history_and_report_csv_semantics_golden() -> None:
    """Lock pandas CSV output while Reports account tokens with raw rows."""
    tokenizer = ByteTokenizer()
    csv_values = [
        "plain",
        "a|b",
        'a"b',
        "a\\b",
        "a\nb",
        "a\r\nb",
        'a\\b|c"d',
    ]
    assert (
        pd.DataFrame({"value": csv_values}).to_csv(sep="|", index=False)
        == REPORT_CSV_SPECIAL_CHARACTERS["csv_values"]
    )

    history = ConversationHistory()
    special_history = 'history|value "quoted" \\path\nsecond line'
    history.add_turn(ConversationRole.USER, special_history)
    history.add_turn(ConversationRole.USER, "second history")
    history_text, history_records = history.build_context(
        tokenizer=tokenizer,
        include_user_turns_only=True,
        max_qa_turns=5,
        max_context_tokens=REPORT_CSV_SPECIAL_CHARACTERS["history_budget"],
        recency_bias=False,
    )
    assert history_text == REPORT_CSV_SPECIAL_CHARACTERS["history_context"]
    assert {
        "columns": list(history_records["conversation history"].columns),
        "rows": history_records["conversation history"].values.tolist(),
    } == REPORT_CSV_SPECIAL_CHARACTERS["history_records"]

    special_report = 'alpha|beta "quoted" \\path\nsecond line'
    local_reports = [
        _report("0", 4.0, special_report),
        _report("1", 3.0, "plain second"),
    ]
    local_contexts, local_records = build_community_context(
        community_reports=local_reports,
        tokenizer=tokenizer,
        use_community_summary=False,
        shuffle_data=False,
        include_community_rank=False,
        include_community_weight=False,
        max_context_tokens=REPORT_CSV_SPECIAL_CHARACTERS["local_report_budget"],
        single_batch=True,
        context_name="Reports",
    )
    assert local_contexts == [REPORT_CSV_SPECIAL_CHARACTERS["local_reports_context"]]
    local_report_ids = REPORT_CSV_SPECIAL_CHARACTERS["local_report_ids"]
    assert local_records["reports"]["id"].tolist() == local_report_ids
    under_contexts, under_records = build_community_context(
        community_reports=local_reports,
        tokenizer=tokenizer,
        use_community_summary=False,
        shuffle_data=False,
        include_community_rank=False,
        include_community_weight=False,
        max_context_tokens=(REPORT_CSV_SPECIAL_CHARACTERS["local_report_budget"] - 1),
        single_batch=True,
        context_name="Reports",
    )
    under_budget_context = REPORT_CSV_SPECIAL_CHARACTERS[
        "local_reports_under_budget_context"
    ]
    assert under_contexts == under_budget_context
    assert under_records == {}

    global_reports = [
        _report(
            str(index),
            float(index),
            special_report if index == 1 else f"Full content {index}",
        )
        for index in range(4)
    ]
    global_entities = [
        _entity(
            f"entity-{index}",
            str(index),
            f"Entity {index}",
            index,
            [str(index)],
            ["unit"],
        )
        for index in range(4)
    ]
    global_batches, global_records = build_community_context(
        community_reports=global_reports,
        entities=global_entities,
        tokenizer=tokenizer,
        use_community_summary=False,
        shuffle_data=True,
        include_community_rank=True,
        include_community_weight=True,
        normalize_community_weight=True,
        max_context_tokens=REPORT_CSV_SPECIAL_CHARACTERS["global_budget"],
        single_batch=False,
        context_name="Reports",
        random_state=86,
    )
    assert global_batches == REPORT_CSV_SPECIAL_CHARACTERS["global_batches"]
    assert [
        pd.read_csv(StringIO(batch), sep="|", dtype=str)["id"].tolist()
        for batch in global_batches
    ] == REPORT_CSV_SPECIAL_CHARACTERS["global_batch_ids"]
    global_record_ids = REPORT_CSV_SPECIAL_CHARACTERS["global_records_ids"]
    assert global_records["reports"]["id"].tolist() == global_record_ids


def test_graphrag_3_1_local_uuid_and_empty_frame_boundaries() -> None:
    """Lock UUID canonicalization and empty DataFrame column behavior."""
    dashed = "550e8400-e29b-41d4-a716-446655440000"
    entity = _entity(
        dashed.replace("-", ""),
        "0",
        "Alice",
        1,
        [],
        [],
    )
    assert get_entity_by_id({entity.id: entity}, dashed) is entity

    history = ConversationHistory()
    history.add_turn(ConversationRole.USER, "long question")
    history_text, history_records = history.build_context(
        tokenizer=ByteTokenizer(),
        include_user_turns_only=True,
        max_qa_turns=5,
        max_context_tokens=len(
            "-----Conversation History-----\nturn|content\n".encode()
        ),
        recency_bias=False,
    )
    assert history_text == "-----Conversation History-----\n\n"
    assert list(history_records["conversation history"].columns) == []

    relationship_text, relationship_records = build_relationship_context(
        selected_entities=[entity],
        relationships=[],
        tokenizer=ByteTokenizer(),
        include_relationship_weight=True,
    )
    assert relationship_text == ""
    assert list(relationship_records.columns) == []


def test_graphrag_3_1_local_request_stream_and_usage_golden() -> None:
    """Lock Local model messages, call args, chunks, callbacks, and usage."""
    builder, _, _, history = _fixture()
    completion = RecordingCompletion()
    callbacks = RecordingCallbacks()
    search = LocalSearch(
        model=completion,
        context_builder=builder,
        tokenizer=ByteTokenizer(),
        response_type="Multiple Paragraphs",
        callbacks=[callbacks],
        model_params={"temperature": 0.0, "top_p": 1.0},
        context_builder_params=_context_params(),
    )

    async def collect() -> list[str]:
        return [
            chunk
            async for chunk in search.stream_search(
                query="current",
                conversation_history=history,
            )
        ]

    chunks = asyncio.run(collect())

    assert chunks == ["Local ", "answer."]
    assert callbacks.events == [
        "context",
        "token:Local ",
        "token:answer.",
    ]
    messages, kwargs = completion.requests[0]
    assert messages[0]["content"] == LOCAL_SEARCH_SYSTEM_PROMPT.format(
        context_data=LOCAL_CONTEXT,
        response_type="Multiple Paragraphs",
    )
    assert messages[1]["content"] == "current"
    assert kwargs == {
        "stream": True,
        "temperature": 0.0,
        "top_p": 1.0,
    }

    usage_completion = RecordingCompletion()
    usage_search = LocalSearch(
        model=usage_completion,
        context_builder=builder,
        tokenizer=ByteTokenizer(),
        response_type="Multiple Paragraphs",
        model_params={"temperature": 0.0, "top_p": 1.0},
        context_builder_params=_context_params(),
    )
    result = asyncio.run(
        usage_search.search(
            query="current",
            conversation_history=history,
        )
    )
    assert result.response == "Local answer."
    assert result.llm_calls_categories == {
        "build_context": 0,
        "response": 1,
    }
    assert result.output_tokens_categories == {
        "build_context": 0,
        "response": len("Local answer.".encode()),
    }


def test_graphrag_3_1_global_weight_shuffle_and_batch_golden() -> None:
    """Lock occurrence weights, seed-86 membership, and within-batch sort."""
    reports = [
        _report(str(index), float(index), f"Full content {index}") for index in range(4)
    ]
    entities = [
        _entity(
            f"entity-{index}",
            str(index),
            f"Entity {index}",
            index,
            [str(index)],
            ["shared"],
        )
        for index in range(4)
    ]
    header_tokens = len(
        "-----Reports-----\nid|title|occurrence weight|content|rank\n".encode()
    )
    row_tokens = len("0|Report 0|1.0|Full content 0|0.0\n".encode())
    batches, records = build_community_context(
        community_reports=reports,
        entities=entities,
        tokenizer=ByteTokenizer(),
        use_community_summary=False,
        shuffle_data=True,
        include_community_rank=True,
        min_community_rank=0,
        community_weight_name="occurrence weight",
        normalize_community_weight=True,
        max_context_tokens=header_tokens + row_tokens * 2,
        single_batch=False,
        context_name="Reports",
        random_state=86,
    )

    assert batches == GLOBAL_BATCHES
    assert list(records) == ["reports"]
    assert list(records["reports"].columns) == [
        "id",
        "title",
        "occurrence weight",
        "content",
        "rank",
    ]
    assert records["reports"]["id"].tolist() == ["3", "1", "2", "0"]


def test_graphrag_3_1_global_zero_max_weight_boundary() -> None:
    """Record the upstream division-by-zero edge GraphLoom handles finitely."""
    with pytest.raises(ZeroDivisionError):
        build_community_context(
            community_reports=[_report("0", 1.0, "No occurrences")],
            entities=[
                _entity(
                    "entity-other",
                    "0",
                    "Other",
                    0,
                    ["other-community"],
                    [],
                )
            ],
            tokenizer=ByteTokenizer(),
            use_community_summary=False,
            include_community_rank=True,
            include_community_weight=True,
            normalize_community_weight=True,
            single_batch=False,
        )


def test_graphrag_3_1_global_map_reduce_request_and_usage_golden() -> None:
    """Lock map order, parsing, reduce data/messages, chunks, and usage."""
    completion = RecordingGlobalCompletion()
    callbacks = RecordingGlobalCallbacks()
    search = GlobalSearch(
        model=completion,
        context_builder=FixedGlobalContext(),
        tokenizer=ByteTokenizer(),
        response_type="Multiple Paragraphs",
        callbacks=[callbacks],
        max_data_tokens=20_000,
        map_max_length=1_000,
        reduce_max_length=2_000,
        concurrent_coroutines=2,
        json_mode=False,
    )

    async def collect() -> list[str]:
        return [
            chunk
            async for chunk in search.stream_search(
                query="What are the themes?",
            )
        ]

    chunks = asyncio.run(collect())
    reduce_data = (
        "----Analyst 2----\nImportance Score: 9\nbest\n\n"
        "----Analyst 1----\nImportance Score: 5\nfirst tie\n\n"
        "----Analyst 2----\nImportance Score: 5\nsecond tie"
    )
    assert chunks == ["Global ", "answer."]
    assert callbacks.events == [
        "map_start:2",
        "map_end:2",
        "context",
        "token:Global ",
        "token:answer.",
    ]
    assert len(completion.requests) == 3
    for index, (messages, kwargs) in enumerate(completion.requests[:2]):
        assert messages[0]["content"] == MAP_SYSTEM_PROMPT.format(
            context_data=GLOBAL_BATCHES[index],
            max_length=1_000,
        )
        assert messages[1]["content"] == "What are the themes?"
        assert kwargs == {"response_format_json_object": True}
    reduce_messages, reduce_kwargs = completion.requests[2]
    assert reduce_messages[0]["content"] == REDUCE_SYSTEM_PROMPT.format(
        report_data=reduce_data,
        response_type="Multiple Paragraphs",
        max_length=2_000,
    )
    assert reduce_messages[1]["content"] == "What are the themes?"
    assert reduce_kwargs == {"stream": True}

    usage_completion = RecordingGlobalCompletion()
    usage_search = GlobalSearch(
        model=usage_completion,
        context_builder=FixedGlobalContext(),
        tokenizer=ByteTokenizer(),
        response_type="Multiple Paragraphs",
        max_data_tokens=20_000,
        map_max_length=1_000,
        reduce_max_length=2_000,
        concurrent_coroutines=2,
        json_mode=False,
    )
    result = asyncio.run(usage_search.search(query="What are the themes?"))
    assert result.response == "Global answer."
    assert result.reduce_context_text == reduce_data
    assert [point.response for point in result.map_responses] == [
        [{"answer": "first tie", "score": 5}],
        [
            {"answer": "best", "score": 9},
            {"answer": "second tie", "score": 5},
        ],
    ]
    assert result.llm_calls_categories == {
        "build_context": 0,
        "map": 2,
        "reduce": 1,
    }


def test_graphrag_3_1_dynamic_rate_prompt_parser_and_vote_golden() -> None:
    """Lock built-in rate text, JSON fallback, repeats, and tie-smallest vote."""
    expected_prompt = (
        Path(__file__).parent / "fixtures" / "query" / "rate_prompt.txt"
    ).read_text(encoding="utf-8")
    assert (
        RATE_QUERY.format(description="DESCRIPTION", question="QUESTION")
        == expected_prompt
    )
    completion = RecordingDynamicCompletion(
        {
            "FULL-TIE": [
                '{"rating":4}',
                '{"rating":2}',
            ],
            "FULL-FALLBACK": [
                "malformed",
            ],
            "FULL-TRAILING": [
                '{"rating":4,}',
            ],
            "FULL-BARE-KEY": [
                "{rating: 4}",
            ],
        }
    )
    tie = asyncio.run(
        rate_relevancy(
            query="QUESTION",
            description="FULL-TIE",
            model=completion,
            tokenizer=ByteTokenizer(),
            num_repeats=2,
        )
    )
    fallback = asyncio.run(
        rate_relevancy(
            query="QUESTION",
            description="FULL-FALLBACK",
            model=completion,
            tokenizer=ByteTokenizer(),
            num_repeats=1,
        )
    )
    repaired = [
        asyncio.run(
            rate_relevancy(
                query="QUESTION",
                description=description,
                model=completion,
                tokenizer=ByteTokenizer(),
                num_repeats=1,
            )
        )
        for description in ["FULL-TRAILING", "FULL-BARE-KEY"]
    ]
    assert tie["ratings"] == [4, 2]
    assert tie["rating"] == 2
    assert tie["llm_calls"] == 2
    assert tie["prompt_tokens"] == 2 * (
        len(
            RATE_QUERY.format(
                description="FULL-TIE",
                question="QUESTION",
            ).encode()
        )
        + len("QUESTION".encode())
    )
    assert fallback["ratings"] == [1]
    assert fallback["rating"] == 1
    assert [result["ratings"] for result in repaired] == [[4], [4]]
    assert [result["rating"] for result in repaired] == [4, 4]
    assert all(
        kwargs == {"response_format_json_object": True}
        for _, kwargs in completion.requests
    )
    illegal_completion = RecordingDynamicCompletion(
        {"FULL-ILLEGAL": ['{"rating":null}']}
    )
    with pytest.raises(TypeError):
        asyncio.run(
            rate_relevancy(
                query="QUESTION",
                description="FULL-ILLEGAL",
                model=illegal_completion,
                tokenizer=ByteTokenizer(),
            )
        )


def _community(
    community_id: int,
    level: int,
    parent: int,
    children: list[int],
) -> Community:
    return Community(
        id=f"community-{community_id}",
        short_id=str(community_id),
        title=f"Community {community_id}",
        level=str(level),
        parent=str(parent),
        children=children,
    )


def test_graphrag_3_1_dynamic_traversal_keep_parent_and_fallback_golden() -> None:
    """Lock threshold equality, parent removal, child traversal, and fallback."""

    async def select(keep_parent: bool) -> tuple[set[str], dict[str, Any]]:
        completion = RecordingDynamicCompletion(
            {
                "FULL-ROOT": ['{"rating":3}'],
                "FULL-CHILD": ['{"rating":4}'],
            }
        )
        selector = DynamicCommunitySelection(
            community_reports=[
                _report("0", 1.0, "FULL-ROOT"),
                _report("1", 1.0, "FULL-CHILD"),
            ],
            communities=[
                _community(0, 0, -1, [1]),
                _community(1, 1, 0, []),
            ],
            model=completion,
            tokenizer=ByteTokenizer(),
            threshold=3,
            keep_parent=keep_parent,
            num_repeats=1,
            max_level=2,
            concurrent_coroutines=2,
        )
        reports, info = await selector.select("QUESTION")
        return {report.community_id for report in reports}, info

    selected_keep, keep_info = asyncio.run(select(True))
    selected_remove, remove_info = asyncio.run(select(False))
    assert selected_keep == {"0", "1"}
    assert selected_remove == {"1"}
    assert keep_info["ratings"] == {"0": 3, "1": 4}
    assert remove_info["llm_calls"] == 2

    fallback_completion = RecordingDynamicCompletion(
        {
            "FULL-LEVEL0": ['{"rating":0}'],
            "FULL-LEVEL1": ['{"rating":1}'],
            "FULL-LEVEL2": ['{"rating":5}'],
        }
    )
    fallback_selector = DynamicCommunitySelection(
        community_reports=[
            _report("0", 1.0, "FULL-LEVEL0"),
            _report("1", 1.0, "FULL-LEVEL1"),
            _report("2", 1.0, "FULL-LEVEL2"),
        ],
        communities=[
            _community(0, 0, -1, []),
            _community(1, 1, -1, []),
            _community(2, 2, -1, []),
        ],
        model=fallback_completion,
        tokenizer=ByteTokenizer(),
        threshold=3,
        max_level=2,
    )
    fallback_reports, fallback_info = asyncio.run(fallback_selector.select("QUESTION"))
    assert {report.community_id for report in fallback_reports} == {"2"}
    assert fallback_info["ratings"] == {"0": 0, "1": 1, "2": 5}


def test_graphrag_3_1_dynamic_max_level_only_limits_fallback_golden() -> None:
    """Distinguish relevant child traversal from whole-level fallback."""
    traversal_completion = RecordingDynamicCompletion(
        {
            "FULL-ROOT": ['{"rating":5}'],
            "FULL-MIDDLE": ['{"rating":4}'],
            "FULL-LEAF": ['{"rating":3}'],
        }
    )
    traversal_selector = DynamicCommunitySelection(
        community_reports=[
            _report("0", 1.0, "FULL-ROOT"),
            _report("1", 1.0, "FULL-MIDDLE"),
            _report("2", 1.0, "FULL-LEAF"),
        ],
        communities=[
            _community(0, 0, -1, [1]),
            _community(1, 1, 0, [2]),
            _community(2, 2, 1, []),
        ],
        model=traversal_completion,
        tokenizer=ByteTokenizer(),
        threshold=3,
        keep_parent=True,
        max_level=1,
    )
    traversal_reports, traversal_info = asyncio.run(
        traversal_selector.select("QUESTION")
    )
    assert {report.community_id for report in traversal_reports} == {
        "0",
        "1",
        "2",
    }
    assert traversal_info["ratings"] == {"0": 5, "1": 4, "2": 3}
    assert traversal_info["llm_calls"] == 3

    fallback_completion = RecordingDynamicCompletion(
        {
            "FULL-LEVEL0": ['{"rating":0}'],
            "FULL-LEVEL1": ['{"rating":1}'],
            "FULL-LEVEL2": ['{"rating":5}'],
        }
    )
    fallback_selector = DynamicCommunitySelection(
        community_reports=[
            _report("0", 1.0, "FULL-LEVEL0"),
            _report("1", 1.0, "FULL-LEVEL1"),
            _report("2", 1.0, "FULL-LEVEL2"),
        ],
        communities=[
            _community(0, 0, -1, []),
            _community(1, 1, -1, []),
            _community(2, 2, -1, []),
        ],
        model=fallback_completion,
        tokenizer=ByteTokenizer(),
        threshold=3,
        max_level=1,
    )
    fallback_reports, fallback_info = asyncio.run(fallback_selector.select("QUESTION"))
    assert fallback_reports == []
    assert fallback_info["ratings"] == {"0": 0, "1": 1}
    assert fallback_info["llm_calls"] == 2

    relevant_completion = RecordingDynamicCompletion(
        {
            "FULL-ROOT": ['{"rating":5}'],
            "FULL-UNRELATED": ['{"rating":0}'],
        }
    )
    relevant_selector = DynamicCommunitySelection(
        community_reports=[
            _report("0", 1.0, "FULL-ROOT"),
            _report("1", 1.0, "FULL-UNRELATED"),
        ],
        communities=[
            _community(0, 0, -1, []),
            _community(1, 1, -1, []),
        ],
        model=relevant_completion,
        tokenizer=ByteTokenizer(),
        threshold=3,
        max_level=2,
    )
    relevant_reports, relevant_info = asyncio.run(relevant_selector.select("QUESTION"))
    assert {report.community_id for report in relevant_reports} == {"0"}
    assert relevant_info["ratings"] == {"0": 5}
    assert relevant_info["llm_calls"] == 1


def test_graphrag_3_1_dynamic_selection_feeds_fixed_global_batches_golden() -> None:
    """Lock the stable-selection decision before shared map/reduce batches."""
    reports = [
        _report(str(index), float(index), f"Full content {index}") for index in range(4)
    ]
    completion = RecordingDynamicCompletion(
        {f"Full content {index}": ['{"rating":5}'] for index in range(4)}
    )
    selector = DynamicCommunitySelection(
        community_reports=reports,
        communities=[_community(index, 0, -1, []) for index in range(4)],
        model=completion,
        tokenizer=ByteTokenizer(),
        threshold=3,
        max_level=0,
        concurrent_coroutines=2,
    )
    selected, info = asyncio.run(selector.select("What are the themes?"))
    selected_ids = {report.community_id for report in selected}
    assert selected_ids == {"0", "1", "2", "3"}
    assert info["ratings"] == {"0": 5, "1": 5, "2": 5, "3": 5}

    # GraphRAG returns the selected collection from a set. GraphLoom deliberately
    # stabilizes that boundary to traversal first-seen order, then applies the same
    # seed-86 shuffle used by fixed Global context.
    stable_selected = [
        report for report in reports if report.community_id in selected_ids
    ]
    entities = [
        _entity(
            f"entity-{index}",
            str(index),
            f"Entity {index}",
            index,
            [str(index)],
            ["shared"],
        )
        for index in range(4)
    ]
    header_tokens = len(
        "-----Reports-----\nid|title|occurrence weight|content|rank\n".encode()
    )
    row_tokens = len("0|Report 0|1.0|Full content 0|0.0\n".encode())
    batches, records = build_community_context(
        community_reports=stable_selected,
        entities=entities,
        tokenizer=ByteTokenizer(),
        use_community_summary=False,
        shuffle_data=True,
        include_community_rank=True,
        min_community_rank=0,
        community_weight_name="occurrence weight",
        normalize_community_weight=True,
        max_context_tokens=header_tokens + row_tokens * 2,
        single_batch=False,
        context_name="Reports",
        random_state=86,
    )
    assert batches == GLOBAL_BATCHES
    assert records["reports"]["id"].tolist() == ["3", "1", "2", "0"]


def test_graphrag_3_1_drift_hyde_prompt_and_embedding_golden(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Lock the exact odd HyDE prompt, strict-empty fallback, and embedding input."""
    reports = [_report("0", 1.0, "# Template\nFacts.")]
    completion = RecordingDriftCompletion([("", None)])
    embedding = RecordingEmbedding()
    monkeypatch.setattr("secrets.choice", lambda values: values[0])
    processor = PrimerQueryProcessor(
        chat_model=completion,
        text_embedder=embedding,
        reports=reports,
        tokenizer=ByteTokenizer(),
    )

    vector, usage = asyncio.run(processor("What changed?"))
    expected = (
        "Create a hypothetical answer to the following query: What changed?\n\n\n"
        "                  Format it to follow the structure of the template below:\n\n\n"
        '                  # Template\nFacts.\n"\n'
        "                  Ensure that the hypothetical answer does not reference new "
        "named entities that are not present in the original query."
    )
    assert expected == DRIFT_HYDE_PROMPT
    assert completion.requests == [(DRIFT_HYDE_PROMPT, {})]
    assert embedding.inputs == [["What changed?"]]
    assert vector == [0.2, 0.8]
    assert usage == {
        "llm_calls": 1,
        "prompt_tokens": len(expected.encode()),
        "output_tokens": 0,
    }


def test_graphrag_3_1_drift_ranking_and_stable_tie_golden(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Lock report-id hydration shape and pandas nlargest keep-first ordering."""
    reports = [
        _report("0", 1.0, "First"),
        _report("1", 1.0, "Second"),
        _report("2", 1.0, "Third"),
    ]
    reports[0].full_content_embedding = [1.0, 0.0]
    reports[1].full_content_embedding = [1.0, 0.0]
    reports[2].full_content_embedding = [0.0, 1.0]
    completion = RecordingDriftCompletion([("expanded", None)])
    embedding = RecordingEmbedding()
    embedding.embedding = lambda input: SimpleNamespace(first_embedding=[1.0, 0.0])
    monkeypatch.setattr("secrets.choice", lambda values: values[0])
    builder = object.__new__(DRIFTSearchContextBuilder)
    builder.reports = reports
    builder.model = completion
    builder.text_embedder = embedding
    builder.tokenizer = ByteTokenizer()
    builder.config = SimpleNamespace(drift_k_followups=2)

    top_k, usage = asyncio.run(builder.build_context("Question"))

    assert top_k["short_id"].tolist() == ["0", "1"]
    assert list(top_k.columns) == ["short_id", "community_id", "full_content"]
    assert usage["llm_calls"] == 1


@pytest.mark.parametrize(
    ("length", "folds", "sizes"),
    [
        (5, 3, [2, 2, 1]),
        (2, 4, [1, 1, 0, 0]),
        (2, 0, [2]),
    ],
)
def test_graphrag_3_1_drift_numpy_array_split_golden(
    length: int,
    folds: int,
    sizes: list[int],
) -> None:
    """Lock uneven, zero, and empty-fold primer splitting."""
    primer = object.__new__(DRIFTPrimer)
    primer.config = SimpleNamespace(primer_folds=folds)
    frame = pd.DataFrame({"full_content": [str(index) for index in range(length)]})

    assert [len(fold) for fold in primer.split_reports(frame)] == sizes


def test_graphrag_3_1_drift_primer_prompt_schema_and_usage_golden() -> None:
    """Lock primer message shape, Pydantic schema, aggregation input, and usage."""
    formatted = PrimerResponse(
        intermediate_answer="# Answer",
        score=70,
        follow_up_queries=["next", "next"],
    )
    completion = RecordingDriftCompletion(
        [
            (
                '{"intermediate_answer":"# Answer","score":70,"follow_up_queries":["next","next"]}',
                formatted,
            )
        ]
    )
    primer = object.__new__(DRIFTPrimer)
    primer.chat_model = completion
    primer.tokenizer = ByteTokenizer()
    primer.config = SimpleNamespace(primer_folds=1)
    reports = pd.DataFrame({"full_content": ["Report A", "Report B"]})

    response, usage = asyncio.run(primer.decompose_query("Question", reports))
    expected_prompt = DRIFT_PRIMER_PROMPT.format(
        query="Question",
        community_reports="Report A\n\nReport B",
    )

    assert completion.requests == [
        (expected_prompt, {"response_format": PrimerResponse})
    ]
    assert response == formatted.model_dump()
    assert usage == {
        "llm_calls": 1,
        "prompt_tokens": len(expected_prompt.encode()),
        "output_tokens": len(completion.requests[0][0].encode())
        - len(expected_prompt.encode())
        + len(
            '{"intermediate_answer":"# Answer","score":70,'
            '"follow_up_queries":["next","next"]}'.encode()
        ),
    }
    schema = PrimerResponse.model_json_schema()
    assert schema["required"] == [
        "intermediate_answer",
        "score",
        "follow_up_queries",
    ]


def test_graphrag_3_1_drift_action_repair_and_state_identity_golden() -> None:
    """Lock repair, float conversion, empty primer answers, and graph identity."""
    _, parsed = try_parse_json_object(
        'prefix {response:"answer",score:"80",follow_up_queries:["same"],} suffix'
    )
    assert parsed == {
        "response": "answer",
        "score": "80",
        "follow_up_queries": ["same"],
    }
    root = DriftAction("root", answer="root answer")
    root.score = 70
    state = QueryState()
    state.add_action(root)
    state.add_all_follow_ups(root, ["same", "same"])
    serialized, _, _ = state.serialize()

    assert [node["query"] for node in serialized["nodes"]] == ["root", "same"]
    assert [node["id"] for node in serialized["nodes"]] == [0, 1]
    assert serialized["edges"] == [
        {"source": 0, "target": 1, "weight": 1.0},
        {"source": 0, "target": 1, "weight": 1.0},
    ]

    class FixedLocalSearch:
        async def search(self, **_kwargs: Any) -> SimpleNamespace:
            return SimpleNamespace(
                response='{"response":"","score":"-inf","follow_up_queries":[]}',
                context_data={},
                prompt_tokens=0,
                output_tokens=0,
            )

    action = DriftAction("empty action")
    asyncio.run(
        action.search(
            search_engine=FixedLocalSearch(),
            global_query="root",
            k_followups=1,
        )
    )
    assert action.answer == ""
    assert action.is_complete
    assert action.score == float("-inf")

    search = object.__new__(DRIFTSearch)
    primer_action = search._process_primer_results(
        "root",
        SimpleNamespace(
            response=[
                {
                    "intermediate_answer": "",
                    "score": 0,
                    "follow_up_queries": ["next"],
                }
            ]
        ),
    )
    assert primer_action.answer == ""
    assert primer_action.follow_ups == ["next"]


def test_graphrag_3_1_drift_reduce_python_list_representation_golden() -> None:
    """Lock truthy answer filtering and Python list formatting for reduce."""
    answers = ["one", 'it\'s "quoted"\\next\nline']

    assert str(answers) == r"""['one', 'it\'s "quoted"\\next\nline']"""

    class ReduceModel:
        async def completion_async(
            self,
            messages: list[Any],
            **kwargs: Any,
        ) -> AsyncIterator[SimpleNamespace]:
            assert messages[0]["content"] == "REDUCE ['   ', 'real answer']"
            assert messages[1]["content"] == "Root query"
            assert kwargs == {"stream": True}

            async def chunks() -> AsyncIterator[SimpleNamespace]:
                yield SimpleNamespace(
                    choices=[SimpleNamespace(delta=SimpleNamespace(content="final"))]
                )

            return chunks()

    search = object.__new__(DRIFTSearch)
    search.model = ReduceModel()
    search.context_builder = SimpleNamespace(
        reduce_system_prompt="REDUCE {context_data}",
        response_type="Multiple Paragraphs",
    )
    search.callbacks = []
    response_state = {
        "nodes": [
            {"answer": None},
            {"answer": ""},
            {"answer": "   "},
            {"answer": "real answer"},
        ],
        "edges": [],
    }

    async def collect() -> list[str]:
        return [
            chunk
            async for chunk in search._reduce_response_streaming(
                responses=response_state,
                query="Root query",
                model_params={},
            )
        ]

    assert asyncio.run(collect()) == ["final"]


def test_graphrag_3_1_drift_stream_callback_and_reduce_request_golden(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Lock deterministic depth, intermediate callbacks, and final reduce request."""

    class ContextBuilder:
        config = SimpleNamespace(
            n_depth=1,
            drift_k_followups=1,
            reduce_temperature=0.25,
            reduce_max_completion_tokens=321,
        )
        reduce_system_prompt = "REDUCE {context_data} AS {response_type}"
        response_type = "Multiple Paragraphs"

        async def build_context(
            self, _query: str
        ) -> tuple[pd.DataFrame, dict[str, int]]:
            return pd.DataFrame({"full_content": ["Report"]}), {
                "llm_calls": 1,
                "prompt_tokens": 2,
                "output_tokens": 3,
            }

    class Primer:
        async def search(self, **_kwargs: Any) -> SimpleNamespace:
            return SimpleNamespace(
                response=[
                    {
                        "intermediate_answer": "Primer answer",
                        "score": 70,
                        "follow_up_queries": ["followup"],
                    }
                ],
                llm_calls=1,
                prompt_tokens=4,
                output_tokens=5,
            )

    class Callbacks(RecordingCallbacks):
        def __init__(self) -> None:
            super().__init__()
            self.reduce_starts: list[Any] = []

        def on_reduce_response_start(self, context: Any) -> None:
            self.reduce_starts.append(context)
            self.events.append("reduce_start")

        def on_reduce_response_end(self, response: str) -> None:
            self.events.append(f"reduce_end:{response}")

    callbacks = Callbacks()

    class Local:
        async def search(self, **_kwargs: Any) -> SimpleNamespace:
            callbacks.on_llm_new_token("action-token")
            callbacks.on_context({"entities": ["entity"]})
            return SimpleNamespace(
                response='{"response":"Action answer","score":80,"follow_up_queries":[]}',
                context_data={"entities": ["entity"]},
                prompt_tokens=6,
                output_tokens=7,
            )

    class ReduceModel:
        tokenizer = ByteTokenizer()

        def __init__(self) -> None:
            self.requests: list[tuple[list[Any], dict[str, Any]]] = []

        async def completion_async(
            self,
            messages: list[Any],
            **kwargs: Any,
        ) -> AsyncIterator[SimpleNamespace]:
            self.requests.append((messages, kwargs))

            async def chunks() -> AsyncIterator[SimpleNamespace]:
                for text in ["Final ", "answer"]:
                    yield SimpleNamespace(
                        choices=[SimpleNamespace(delta=SimpleNamespace(content=text))]
                    )

            return chunks()

    model = ReduceModel()
    search = object.__new__(DRIFTSearch)
    search.model = model
    search.context_builder = ContextBuilder()
    search.tokenizer = ByteTokenizer()
    search.query_state = QueryState()
    search.primer = Primer()
    search.callbacks = [callbacks]
    search.local_search = Local()
    monkeypatch.setattr(
        "graphrag.query.structured_search.drift_search.state.random.shuffle",
        lambda actions: None,
    )

    async def collect() -> list[str]:
        return [chunk async for chunk in search.stream_search("Root query")]

    assert asyncio.run(collect()) == ["Final ", "answer"]
    assert callbacks.events == [
        "token:action-token",
        "context",
        "reduce_start",
        "token:Final ",
        "token:answer",
        "reduce_end:Final answer",
    ]
    assert len(callbacks.reduce_starts) == 1
    reduce_start = callbacks.reduce_starts[0]
    assert isinstance(reduce_start, dict)
    assert [node["query"] for node in reduce_start["nodes"]] == [
        "Root query",
        "followup",
    ]
    assert reduce_start["edges"] == [{"source": 0, "target": 1, "weight": 1.0}]
    assert reduce_start != ["Primer answer", "Action answer"]
    assert len(model.requests) == 1
    messages, kwargs = model.requests[0]
    assert messages == [
        {
            "role": "system",
            "content": (
                "REDUCE ['Primer answer', 'Action answer'] AS Multiple Paragraphs"
            ),
        },
        {"role": "user", "content": "Root query"},
    ]
    assert kwargs == {
        "stream": True,
        "temperature": 0.25,
        "max_completion_tokens": 321,
    }
    assert "max_tokens" not in kwargs


def test_graphrag_3_1_drift_call_kwargs_do_not_override_max_tokens() -> None:
    """Lock that DRIFT call-level kwargs leave provider base max_tokens intact."""
    search = object.__new__(DRIFTSearch)
    search.context_builder = SimpleNamespace(
        config=SimpleNamespace(
            local_search_text_unit_prop=0.5,
            local_search_community_prop=0.1,
            local_search_top_k_mapped_entities=10,
            local_search_top_k_relationships=10,
            local_search_temperature=0.2,
            local_search_n=1,
            local_search_top_p=0.9,
            local_search_llm_max_gen_completion_tokens=123,
            local_search_max_data_tokens=1_000,
        ),
        local_system_prompt="DRIFT Local",
        local_mixed_context=object(),
    )
    search.model = object()
    search.tokenizer = ByteTokenizer()
    search.callbacks = []

    local = search.init_local_search()

    assert local.model_params == {
        "temperature": 0.2,
        "n": 1,
        "top_p": 0.9,
        "max_completion_tokens": 123,
        "response_format_json_object": True,
    }
    assert "max_tokens" not in local.model_params
