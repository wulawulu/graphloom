"""GraphRAG 3.1.0 Query compatibility goldens.

The fixtures in this module exercise the fixed upstream package directly. Rust
tests use the same identifiers, ordering, and exact context snapshot.
"""

import asyncio
from collections.abc import AsyncIterator
from pathlib import Path
from types import SimpleNamespace
from typing import Any

from graphrag.data_model.community_report import CommunityReport
from graphrag.data_model.covariate import Covariate
from graphrag.data_model.entity import Entity
from graphrag.data_model.relationship import Relationship
from graphrag.data_model.text_unit import TextUnit
from graphrag.prompts.query.local_search_system_prompt import (
    LOCAL_SEARCH_SYSTEM_PROMPT,
)
from graphrag.query.context_builder.conversation_history import (
    ConversationHistory,
    ConversationRole,
)
from graphrag.query.context_builder.local_context import build_relationship_context
from graphrag.query.input.retrieval.entities import get_entity_by_id
from graphrag.query.structured_search.local_search.mixed_context import (
    LocalSearchMixedContext,
)
from graphrag.query.structured_search.local_search.search import LocalSearch


LOCAL_CONTEXT = (
    Path(__file__).parent / "fixtures" / "query" / "local_context.txt"
).read_text(encoding="utf-8")


class ByteTokenizer:
    """Deterministic tokenizer used on both sides of the golden."""

    def encode(self, text: str) -> list[int]:
        return list(text.encode())

    def num_tokens(self, text: str) -> int:
        return len(text.encode())


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


class RecordingCallbacks:
    """Capture the GraphRAG streaming callback lifecycle."""

    def __init__(self) -> None:
        self.events: list[str] = []

    def on_context(self, _context: Any) -> None:
        self.events.append("context")

    def on_llm_new_token(self, token: str) -> None:
        self.events.append(f"token:{token}")


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
