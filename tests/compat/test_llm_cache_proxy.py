"""Tests for the Query LiteLLM caching proxy."""

from __future__ import annotations

import http.client
import json
import sys
import threading
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import pytest

from llm_cache_proxy import (
    CacheEntry,
    CachingProxyEngine,
    CachingProxyServer,
    JsonlCache,
    LiteLlmBackend,
    ProxyResponse,
    UpstreamRoute,
    request_key,
)


class CountingBackend:
    """Thread-safe fake backend with observable concurrency."""

    def __init__(self, delay: float = 0.0) -> None:
        self.delay = delay
        self.calls = 0
        self.active = 0
        self.max_active = 0
        self._lock = threading.Lock()

    def invoke(
        self,
        endpoint: str,
        payload: dict[str, Any],
        authorization: str | None,
    ) -> ProxyResponse:
        assert authorization in {None, "Bearer secret-not-persisted"}
        with self._lock:
            self.calls += 1
            self.active += 1
            self.max_active = max(self.max_active, self.active)
        time.sleep(self.delay)
        body = json.dumps(
            {"endpoint": endpoint, "model": payload.get("model")},
            sort_keys=True,
        ).encode()
        with self._lock:
            self.active -= 1
        return ProxyResponse(200, "application/json", body)


def _payload(model: str = "test") -> dict[str, Any]:
    return {
        "model": model,
        "messages": [{"role": "user", "content": "exact prompt"}],
    }


def _hyde_payload(template: str, query: str = "question") -> dict[str, Any]:
    content = (
        f"Create a hypothetical answer to the following query: {query}\n\n\n"
        "                  Format it to follow the structure of the template below:\n\n\n"
        f'                  {template}\n"\n'
        "                  Ensure that the hypothetical answer does not reference new named "
        "entities that are not present in the original query."
    )
    return {
        "model": "test",
        "messages": [{"role": "user", "content": content}],
    }


def _post(api_base: str, payload: dict[str, Any]) -> bytes:
    request = urllib.request.Request(
        f"{api_base}/chat/completions",
        data=json.dumps(payload).encode(),
        headers={
            "Authorization": "Bearer secret-not-persisted",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=5) as response:
        return response.read()


def test_should_cache_and_reload_exact_response_without_secret(tmp_path: Path) -> None:
    path = tmp_path / "cassette.jsonl"
    backend = CountingBackend()
    engine = CachingProxyEngine(JsonlCache(path), backend)

    first = engine.handle(
        "/v1/chat/completions",
        _payload(),
        "Bearer secret-not-persisted",
    )
    second = engine.handle(
        "/v1/chat/completions",
        _payload(),
        "Bearer secret-not-persisted",
    )
    reloaded = JsonlCache(path).get(request_key("/v1/chat/completions", _payload()))

    assert first == second
    assert backend.calls == 1
    assert [item.cache_status for item in engine.observations_since()] == [
        "miss",
        "hit",
    ]
    assert reloaded is not None
    assert reloaded.response == first
    assert "secret-not-persisted" not in path.read_text()


def test_should_reuse_response_when_only_transport_fields_differ(
    tmp_path: Path,
) -> None:
    backend = CountingBackend()
    engine = CachingProxyEngine(JsonlCache(tmp_path / "cassette.jsonl"), backend)
    first = _payload() | {"stream": False, "temperature": 0.0}
    second = _payload() | {
        "temperature": 1.0,
        "response_format": {"type": "json_object"},
    }

    first_response = engine.handle("/v1/chat/completions", first, None)
    second_response = engine.handle("/v1/chat/completions", second, None)

    assert first_response == second_response
    assert backend.calls == 1
    observations = engine.observations_since()
    assert [item.cache_status for item in observations] == ["miss", "hit"]
    assert observations[0].key == observations[1].key
    assert observations[0].exact_key != observations[1].exact_key


def test_should_normalize_only_random_hyde_template_content() -> None:
    first = _hyde_payload("first random report")
    second = _hyde_payload("second random report")
    changed_query = _hyde_payload("first random report", "different question")

    assert request_key("/v1/chat/completions", first) == request_key(
        "/v1/chat/completions", second
    )
    assert request_key("/v1/chat/completions", first) != request_key(
        "/v1/chat/completions", changed_query
    )


def test_should_single_flight_same_key_without_blocking_different_keys(
    tmp_path: Path,
) -> None:
    backend = CountingBackend(delay=0.1)
    engine = CachingProxyEngine(JsonlCache(tmp_path / "cassette.jsonl"), backend)

    with ThreadPoolExecutor(max_workers=3) as executor:
        futures = [
            executor.submit(
                engine.handle,
                "/v1/chat/completions",
                _payload("same"),
                None,
            ),
            executor.submit(
                engine.handle,
                "/v1/chat/completions",
                _payload("same"),
                None,
            ),
            executor.submit(
                engine.handle,
                "/v1/chat/completions",
                _payload("different"),
                None,
            ),
        ]
        responses = [future.result(timeout=2) for future in futures]

    assert backend.calls == 2
    assert backend.max_active == 2
    assert responses[0] == responses[1]
    assert {item.cache_status for item in engine.observations_since()} == {
        "miss",
        "coalescedHit",
    }


def test_should_serve_openai_http_and_reject_unknown_endpoint(tmp_path: Path) -> None:
    backend = CountingBackend()
    engine = CachingProxyEngine(JsonlCache(tmp_path / "cassette.jsonl"), backend)
    server = CachingProxyServer(engine)
    server.start()
    try:
        assert _post(server.api_base, _payload()) == _post(server.api_base, _payload())
        request = urllib.request.Request(
            f"{server.api_base}/unknown",
            data=b"{}",
            method="POST",
        )
        with pytest.raises(urllib.error.HTTPError) as raised:
            urllib.request.urlopen(request, timeout=5)
        assert raised.value.code == 404
    finally:
        server.close()

    assert backend.calls == 1


def test_should_keep_http_connection_alive_across_requests(tmp_path: Path) -> None:
    backend = CountingBackend()
    engine = CachingProxyEngine(JsonlCache(tmp_path / "cassette.jsonl"), backend)
    server = CachingProxyServer(engine)
    server.start()
    connection = http.client.HTTPConnection(
        server.api_base.removeprefix("http://").removesuffix("/v1"), timeout=5
    )
    try:
        for index in range(20):
            body = json.dumps(_payload(f"model-{index}"))
            connection.request(
                "POST",
                "/v1/chat/completions",
                body,
                {"Content-Type": "application/json"},
            )
            response = connection.getresponse()
            assert response.status == 200
            response.read()
            assert not response.will_close
    finally:
        connection.close()
        server.close()

    assert backend.calls == 20


def test_should_reject_excessively_nested_json(tmp_path: Path) -> None:
    backend = CountingBackend()
    engine = CachingProxyEngine(JsonlCache(tmp_path / "cassette.jsonl"), backend)
    server = CachingProxyServer(engine)
    nested: dict[str, Any] = {}
    current = nested
    for _ in range(65):
        child: dict[str, Any] = {}
        current["child"] = child
        current = child
    server.start()
    try:
        with pytest.raises(urllib.error.HTTPError) as raised:
            _post(server.api_base, nested)
        assert raised.value.code == 400
    finally:
        server.close()

    assert backend.calls == 0


def test_should_replay_cached_sse_bytes(tmp_path: Path) -> None:
    path = tmp_path / "cassette.jsonl"
    payload = _payload() | {"stream": True}
    body = b'data: {"choices":[]}\n\ndata: [DONE]\n\n'
    cache = JsonlCache(path)
    key = request_key("/v1/chat/completions", payload)
    cache.put(
        CacheEntry(
            key,
            "/v1/chat/completions",
            payload,
            ProxyResponse(200, "text/event-stream", body),
        )
    )
    backend = CountingBackend()
    engine = CachingProxyEngine(JsonlCache(path), backend)

    response = engine.handle("/v1/chat/completions", payload, None)

    assert response.body == body
    assert response.content_type == "text/event-stream"
    assert backend.calls == 0


def test_should_reject_corrupt_or_mismatched_cache_entry(tmp_path: Path) -> None:
    path = tmp_path / "cassette.jsonl"
    path.write_text(
        json.dumps(
            {
                "formatVersion": 1,
                "key": "0" * 64,
                "endpoint": "/v1/chat/completions",
                "request": _payload(),
                "response": {
                    "status": 200,
                    "contentType": "application/json",
                    "bodyBase64": "e30=",
                },
            }
        )
    )

    with pytest.raises(ValueError, match="invalid cache entry"):
        JsonlCache(path)


def test_should_adapt_deepseek_after_preserving_original_request(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    captured: dict[str, Any] = {}

    def completion(**arguments: Any) -> SimpleNamespace:
        captured.update(arguments)
        return SimpleNamespace(model_dump=lambda: {"choices": []})

    monkeypatch.setitem(sys.modules, "litellm", SimpleNamespace(completion=completion))
    backend = LiteLlmBackend(
        UpstreamRoute("deepseek", None),
        UpstreamRoute("ollama", "http://127.0.0.1:11434"),
    )
    payload = _payload() | {"response_format": {"type": "json_object"}}

    response = backend.invoke("/v1/chat/completions", payload, "Bearer secret")

    assert response.status == 200
    assert payload["response_format"] == {"type": "json_object"}
    assert captured["response_format"] == {"type": "json_object"}
    assert captured["model"] == "deepseek/test"
    assert captured["drop_params"] is True
    assert captured["timeout"] == 300


def test_should_not_forward_deepseek_token_to_local_ollama(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    captured: dict[str, Any] = {}

    def embedding(**arguments: Any) -> SimpleNamespace:
        captured.update(arguments)
        return SimpleNamespace(model_dump=lambda: {"data": []})

    monkeypatch.setitem(sys.modules, "litellm", SimpleNamespace(embedding=embedding))
    backend = LiteLlmBackend(
        UpstreamRoute("deepseek", None),
        UpstreamRoute("ollama", "http://127.0.0.1:11434"),
    )

    response = backend.invoke(
        "/v1/embeddings",
        {"model": "embed", "input": ["text"]},
        "Bearer deepseek-secret",
    )

    assert response.status == 200
    assert "api_key" not in captured
    assert captured["api_base"] == "http://127.0.0.1:11434"
