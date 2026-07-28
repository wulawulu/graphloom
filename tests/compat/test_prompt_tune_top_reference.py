"""Offline tests for deterministic prompt-tune Top record/replay."""

from __future__ import annotations

import os
from pathlib import Path

from prompt_tune_top_reference import (
    ReplayEntry,
    ReplayServer,
    message_bytes,
    sha256_bytes,
    verify_fixtures,
)


def _request(messages: list[dict[str, str]], ordinal: int) -> dict[str, object]:
    encoded = message_bytes(messages)
    return {
        "messages": messages,
        "messages_sha256": sha256_bytes(encoded),
        "operation": "entity_relationship",
        "producer_local_ordinal": ordinal,
    }


def _response(ordinal: int, content: str) -> dict[str, object]:
    return {
        "operation": "entity_relationship",
        "producer_local_ordinal": ordinal,
        "response": content,
    }


def test_should_verify_committed_prompt_tune_top_byte_evidence() -> None:
    verify_fixtures(Path(os.environ["GRAPHLOOM_BIN"]))


def test_should_match_duplicate_requests_without_arrival_order_identity() -> None:
    messages = [
        {"role": "system", "content": "persona"},
        {"role": "user", "content": "accumulated request"},
    ]
    server = ReplayServer(
        "test",
        [
            ReplayEntry(_request(messages, 0), _response(0, "same")),
            ReplayEntry(_request(messages, 1), _response(1, "same")),
        ],
    )
    payload = {"messages": messages, "model": "prompt-tune-compat"}
    try:
        first_status, first = server.replay(payload)
        second_status, second = server.replay(payload)
        server.assert_exhausted()
    finally:
        server.close()

    assert first_status == second_status == 200
    assert first["choices"][0]["message"]["content"] == "same"
    assert second["choices"][0]["message"]["content"] == "same"


def test_should_reject_unknown_prompt_tune_replay_request() -> None:
    messages = [{"role": "user", "content": "known"}]
    server = ReplayServer(
        "test",
        [ReplayEntry(_request(messages, 0), _response(0, "response"))],
    )
    try:
        status, response = server.replay(
            {
                "messages": [{"role": "user", "content": "unknown"}],
                "model": "prompt-tune-compat",
            }
        )
    finally:
        server.close()

    assert status == 400
    assert "unconsumed exact request match" in response["error"]["message"]
