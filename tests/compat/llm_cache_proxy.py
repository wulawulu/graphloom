"""OpenAI-compatible LiteLLM caching proxy for Query compatibility tests."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import math
import os
import sys
import threading
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Protocol
from urllib.parse import urlsplit

FORMAT_VERSION = 2
MAX_REQUEST_BYTES = 16 * 1024 * 1024
MAX_RESPONSE_BYTES = 64 * 1024 * 1024
MAX_CACHE_ENTRIES = 4096
MAX_AUTHORIZATION_BYTES = 8192
MAX_JSON_DEPTH = 64
MAX_JSON_COLLECTION_ITEMS = 100_000
MAX_JSON_KEY_BYTES = 256
MAX_JSON_INTEGER = (1 << 63) - 1
SUPPORTED_ENDPOINTS = frozenset({"/v1/chat/completions", "/v1/embeddings"})
HYDE_TEMPLATE_START = "\n\n\n                  Format it to follow the structure of the template below:\n\n\n                  "
HYDE_TEMPLATE_END = (
    '\n"\n                  Ensure that the hypothetical answer does not reference '
    "new named entities that are not present in the original query."
)


class ProxyError(Exception):
    """Bounded public error returned by the local proxy."""

    def __init__(self, status: int, message: str) -> None:
        super().__init__(message)
        self.status = status


@dataclass(frozen=True)
class ProxyResponse:
    """Replayable HTTP response material."""

    status: int
    content_type: str
    body: bytes


@dataclass(frozen=True)
class CacheEntry:
    """One durable request/response cache entry."""

    key: str
    endpoint: str
    request: dict[str, Any]
    response: ProxyResponse


@dataclass(frozen=True)
class ObservedRequest:
    """One secret-free request observation for the test-side matcher."""

    ordinal: int
    key: str
    exact_key: str
    endpoint: str
    body: dict[str, Any]
    cache_status: str
    response_content: str | None = None


@dataclass(frozen=True)
class UpstreamRoute:
    """Fixed LiteLLM provider routing for one endpoint."""

    provider: str
    api_base: str | None = None


class ProxyBackend(Protocol):
    """Provider boundary used by the caching engine."""

    def invoke(
        self,
        endpoint: str,
        payload: dict[str, Any],
        authorization: str | None,
    ) -> ProxyResponse:
        """Return one complete OpenAI-compatible response."""


class JsonlCache:
    """Append-only, versioned JSONL response cache."""

    def __init__(self, path: Path) -> None:
        self.path = path
        self._entries: dict[str, CacheEntry] = {}
        self._lock = threading.Lock()
        if path.exists():
            self._load()
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.touch(exist_ok=False)

    def get(self, key: str) -> CacheEntry | None:
        """Return an entry without changing it."""
        with self._lock:
            return self._entries.get(key)

    def put(self, entry: CacheEntry) -> CacheEntry:
        """Persist the first successful response for a key."""
        with self._lock:
            existing = self._entries.get(entry.key)
            if existing is not None:
                return existing
            if len(self._entries) >= MAX_CACHE_ENTRIES:
                raise ProxyError(507, "cache contains too many entries")
            encoded = json.dumps(
                _entry_to_json(entry),
                ensure_ascii=False,
                separators=(",", ":"),
                sort_keys=True,
                allow_nan=False,
            )
            with self.path.open("a", encoding="utf-8", newline="\n") as output:
                output.write(encoded)
                output.write("\n")
                output.flush()
                os.fsync(output.fileno())
            self._entries[entry.key] = entry
            return entry

    def _load(self) -> None:
        if self.path.is_symlink() or not self.path.is_file():
            raise ValueError(f"cache path must be a regular file: {self.path}")
        if self.path.stat().st_size > MAX_CACHE_ENTRIES * MAX_RESPONSE_BYTES:
            raise ValueError(f"cache file is too large: {self.path}")
        with self.path.open(encoding="utf-8") as source:
            for line_number, line in enumerate(source, start=1):
                if line_number > MAX_CACHE_ENTRIES:
                    raise ValueError("cache contains too many entries")
                try:
                    value = json.loads(line)
                    entry = _entry_from_json(value)
                except (
                    ValueError,
                    TypeError,
                    RecursionError,
                    json.JSONDecodeError,
                ) as error:
                    raise ValueError(
                        f"invalid cache entry at {self.path}:{line_number}"
                    ) from error
                if entry.key in self._entries:
                    raise ValueError(
                        f"duplicate cache key at {self.path}:{line_number}"
                    )
                self._entries[entry.key] = entry


@dataclass
class _InFlight:
    event: threading.Event
    error: ProxyError | None = None


class CachingProxyEngine:
    """Semantic cache with per-key single-flight and raw request observation."""

    def __init__(self, cache: JsonlCache, backend: ProxyBackend) -> None:
        self._cache = cache
        self._backend = backend
        self._state_lock = threading.Lock()
        self._in_flight: dict[str, _InFlight] = {}
        self._observations: list[ObservedRequest] = []
        self._next_ordinal = 0

    def handle(
        self,
        endpoint: str,
        payload: dict[str, Any],
        authorization: str | None,
    ) -> ProxyResponse:
        """Serve one request from cache or LiteLLM."""
        key = request_key(endpoint, payload)
        ordinal = self._reserve_ordinal()
        cached = self._cache.get(key)
        if cached is not None:
            self._observe(ordinal, key, endpoint, payload, "hit", cached.response)
            return cached.response

        with self._state_lock:
            flight = self._in_flight.get(key)
            leader = flight is None
            if flight is None:
                flight = _InFlight(threading.Event())
                self._in_flight[key] = flight

        if not leader:
            if not flight.event.wait(timeout=300):
                raise ProxyError(504, "timed out waiting for identical request")
            if flight.error is not None:
                self._observe(ordinal, key, endpoint, payload, "upstreamError", None)
                raise flight.error
            cached = self._cache.get(key)
            if cached is None:
                raise ProxyError(
                    502, "identical upstream request produced no cache entry"
                )
            self._observe(
                ordinal,
                key,
                endpoint,
                payload,
                "coalescedHit",
                cached.response,
            )
            return cached.response

        try:
            response = self._backend.invoke(endpoint, payload, authorization)
            if len(response.body) > MAX_RESPONSE_BYTES:
                raise ProxyError(502, "upstream response exceeds 64 MiB")
            stored = self._cache.put(CacheEntry(key, endpoint, payload, response))
            self._observe(ordinal, key, endpoint, payload, "miss", stored.response)
            return stored.response
        except ProxyError as error:
            flight.error = error
            self._observe(ordinal, key, endpoint, payload, "upstreamError", None)
            raise
        except Exception as error:
            detail = _safe_error_detail(error, authorization)
            public = ProxyError(
                502,
                f"LiteLLM request failed: {type(error).__name__}: {detail}",
            )
            flight.error = public
            self._observe(ordinal, key, endpoint, payload, "upstreamError", None)
            raise public from error
        finally:
            with self._state_lock:
                self._in_flight.pop(key, None)
                flight.event.set()

    def observations_since(self, offset: int = 0) -> tuple[ObservedRequest, ...]:
        """Return observations in request arrival order."""
        with self._state_lock:
            return tuple(
                sorted(self._observations[offset:], key=lambda item: item.ordinal)
            )

    def observation_count(self) -> int:
        """Return a stable observation offset."""
        with self._state_lock:
            return len(self._observations)

    def _reserve_ordinal(self) -> int:
        with self._state_lock:
            ordinal = self._next_ordinal
            self._next_ordinal += 1
            return ordinal

    def _observe(
        self,
        ordinal: int,
        key: str,
        endpoint: str,
        payload: dict[str, Any],
        cache_status: str,
        response: ProxyResponse | None,
    ) -> None:
        observation = ObservedRequest(
            ordinal=ordinal,
            key=key,
            exact_key=exact_request_key(endpoint, payload),
            endpoint=endpoint,
            body=payload,
            cache_status=cache_status,
            response_content=_completion_response_content(endpoint, response),
        )
        with self._state_lock:
            self._observations.append(observation)


def _completion_response_content(
    endpoint: str,
    response: ProxyResponse | None,
) -> str | None:
    if response is None or endpoint.endswith("/embeddings"):
        return None
    try:
        if response.content_type.startswith("text/event-stream"):
            chunks: list[str] = []
            for line in response.body.decode("utf-8").splitlines():
                if not line.startswith("data: "):
                    continue
                data = line.removeprefix("data: ")
                if data == "[DONE]":
                    continue
                payload = json.loads(data)
                content = payload["choices"][0]["delta"].get("content")
                if isinstance(content, str):
                    chunks.append(content)
            return "".join(chunks)
        payload = json.loads(response.body)
        content = payload["choices"][0]["message"].get("content")
        return content if isinstance(content, str) else None
    except (IndexError, KeyError, TypeError, UnicodeDecodeError, json.JSONDecodeError):
        return None


class LiteLlmBackend:
    """Synchronous LiteLLM adapter for completion and embedding requests."""

    def __init__(
        self,
        completion: UpstreamRoute,
        embedding: UpstreamRoute,
    ) -> None:
        self._completion = completion
        self._embedding = embedding

    def invoke(
        self,
        endpoint: str,
        payload: dict[str, Any],
        authorization: str | None,
    ) -> ProxyResponse:
        """Call the fixed provider and encode an OpenAI-compatible response."""
        import litellm

        route = (
            self._embedding if endpoint.endswith("/embeddings") else self._completion
        )
        arguments = dict(payload)
        model = arguments.get("model")
        if not isinstance(model, str) or not model:
            raise ProxyError(400, "request model must be a non-empty string")
        arguments["model"] = f"{route.provider}/{model}"
        # Provider adaptation happens after keying and observation, so the exact
        # OpenAI request remains available for compatibility comparison.
        arguments["drop_params"] = True
        arguments["timeout"] = 300
        if route.provider == "deepseek" and isinstance(
            arguments.get("response_format"), dict
        ):
            arguments["response_format"] = {"type": "json_object"}
        token = _bearer_token(authorization)
        if token is not None and route.provider != "ollama":
            arguments["api_key"] = token
        if route.api_base is not None:
            arguments["api_base"] = route.api_base

        if endpoint.endswith("/embeddings"):
            response = litellm.embedding(**arguments)
            return ProxyResponse(
                200, "application/json", _json_bytes(response.model_dump())
            )
        response = litellm.completion(**arguments)
        if payload.get("stream") is True:
            events = [
                b"data: " + _json_bytes(chunk.model_dump()) + b"\n\n"
                for chunk in response
            ]
            events.append(b"data: [DONE]\n\n")
            return ProxyResponse(200, "text/event-stream", b"".join(events))
        return ProxyResponse(
            200, "application/json", _json_bytes(response.model_dump())
        )


class CachingProxyServer:
    """Loopback HTTP lifecycle around a caching engine."""

    def __init__(self, engine: CachingProxyEngine, port: int = 0) -> None:
        handler = self._handler_type()
        self._server = ThreadingHTTPServer(("127.0.0.1", port), handler)
        self._server.daemon_threads = True
        self._server.block_on_close = False
        self._server.engine = engine  # type: ignore[attr-defined]
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)

    @property
    def api_base(self) -> str:
        """Return the bound OpenAI API base."""
        host, port = self._server.server_address
        return f"http://{host}:{port}/v1"

    def start(self) -> None:
        """Start the HTTP thread."""
        self._thread.start()

    def close(self) -> None:
        """Stop the HTTP thread."""
        self._server.shutdown()
        self._server.server_close()
        self._thread.join(timeout=5)

    @staticmethod
    def _handler_type() -> type[BaseHTTPRequestHandler]:
        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def handle(self) -> None:
                try:
                    super().handle()
                except ConnectionError:
                    return

            def do_POST(self) -> None:  # noqa: N802
                try:
                    endpoint = urlsplit(self.path).path
                    if endpoint not in SUPPORTED_ENDPOINTS:
                        raise ProxyError(404, "unsupported endpoint")
                    payload = self._read_payload()
                    authorization = self.headers.get("Authorization")
                    if (
                        authorization is not None
                        and len(authorization.encode("utf-8")) > MAX_AUTHORIZATION_BYTES
                    ):
                        raise ProxyError(400, "Authorization header is too large")
                    engine: CachingProxyEngine = self.server.engine  # type: ignore[attr-defined]
                    response = engine.handle(endpoint, payload, authorization)
                    self._send(response)
                except ProxyError as error:
                    self.close_connection = True
                    self._send(
                        ProxyResponse(
                            error.status,
                            "application/json",
                            _json_bytes({"error": {"message": str(error)}}),
                        )
                    )

            def _read_payload(self) -> dict[str, Any]:
                raw_length = self.headers.get("Content-Length")
                if (
                    raw_length is None
                    or len(raw_length) > 8
                    or not raw_length.isascii()
                    or not raw_length.isdecimal()
                ):
                    raise ProxyError(411, "valid Content-Length is required")
                length = int(raw_length)
                if length > MAX_REQUEST_BYTES:
                    raise ProxyError(413, "request body exceeds 16 MiB")
                body = self.rfile.read(length)
                try:
                    payload = json.loads(body, parse_constant=_reject_constant)
                except (UnicodeDecodeError, ValueError, RecursionError) as error:
                    raise ProxyError(400, "request body must be valid JSON") from error
                if not isinstance(payload, dict):
                    raise ProxyError(400, "request JSON root must be an object")
                _validate_json(payload)
                return payload

            def _send(self, response: ProxyResponse) -> None:
                self.send_response(response.status)
                self.send_header("Content-Type", response.content_type)
                self.send_header("Content-Length", str(len(response.body)))
                if self.close_connection:
                    self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(response.body)

            def log_message(self, _format: str, *_args: object) -> None:
                return

        return Handler


def request_key(endpoint: str, payload: dict[str, Any]) -> str:
    """Return the compatibility key used for response reuse."""
    encoded = _json_bytes(request_match_view(endpoint, payload))
    return hashlib.sha256(encoded).hexdigest()


def exact_request_key(endpoint: str, payload: dict[str, Any]) -> str:
    """Return a key over every original request-body field."""
    encoded = _json_bytes({"method": "POST", "endpoint": endpoint, "body": payload})
    return hashlib.sha256(encoded).hexdigest()


def request_match_view(endpoint: str, payload: dict[str, Any]) -> dict[str, Any]:
    """Project a request onto fields that drive compatibility matching."""
    is_embedding = endpoint.endswith("/embeddings")
    content_field = "input" if is_embedding else "messages"
    content = payload.get(content_field)
    if not is_embedding:
        content = _normalize_messages(content)
    return {
        "method": "POST",
        "endpoint": endpoint,
        "body": {
            "model": payload.get("model"),
            content_field: content,
            "stream": payload.get("stream") is True,
        },
    }


def _normalize_messages(value: Any) -> Any:
    if not isinstance(value, list):
        return value
    normalized = []
    for message in value:
        if not isinstance(message, dict):
            normalized.append(message)
            continue
        content = message.get("content")
        if isinstance(content, str):
            content = _normalize_hyde_template(content)
        normalized.append(message | {"content": content})
    return normalized


def _normalize_hyde_template(content: str) -> str:
    if not content.startswith("Create a hypothetical answer to the following query: "):
        return content
    before, separator, remainder = content.partition(HYDE_TEMPLATE_START)
    if not separator:
        return content
    _, separator, after = remainder.rpartition(HYDE_TEMPLATE_END)
    if not separator:
        return content
    return f"{before}{HYDE_TEMPLATE_START}<random-community-template>{HYDE_TEMPLATE_END}{after}"


def _json_bytes(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
        allow_nan=False,
    ).encode("utf-8")


def _entry_to_json(entry: CacheEntry) -> dict[str, Any]:
    return {
        "formatVersion": FORMAT_VERSION,
        "key": entry.key,
        "endpoint": entry.endpoint,
        "request": entry.request,
        "response": {
            "status": entry.response.status,
            "contentType": entry.response.content_type,
            "bodyBase64": base64.b64encode(entry.response.body).decode("ascii"),
        },
    }


def _entry_from_json(value: Any) -> CacheEntry:
    if not isinstance(value, dict) or set(value) != {
        "formatVersion",
        "key",
        "endpoint",
        "request",
        "response",
    }:
        raise ValueError("invalid cache entry fields")
    if value["formatVersion"] != FORMAT_VERSION:
        raise ValueError("unsupported cache format")
    key = value["key"]
    endpoint = value["endpoint"]
    request = value["request"]
    response = value["response"]
    if (
        not isinstance(key, str)
        or len(key) != 64
        or endpoint not in SUPPORTED_ENDPOINTS
        or not isinstance(request, dict)
        or not isinstance(response, dict)
        or set(response) != {"status", "contentType", "bodyBase64"}
    ):
        raise ValueError("invalid cache entry values")
    try:
        _validate_json(request)
    except ProxyError as error:
        raise ValueError("invalid cached request") from error
    if request_key(endpoint, request) != key:
        raise ValueError("cache key does not match request")
    status = response["status"]
    content_type = response["contentType"]
    if (
        not isinstance(status, int)
        or isinstance(status, bool)
        or not 100 <= status <= 599
    ):
        raise ValueError("invalid cached status")
    if not isinstance(content_type, str) or len(content_type.encode()) > 256:
        raise ValueError("invalid cached content type")
    try:
        body = base64.b64decode(response["bodyBase64"], validate=True)
    except (TypeError, ValueError) as error:
        raise ValueError("invalid cached response body") from error
    if len(body) > MAX_RESPONSE_BYTES:
        raise ValueError("cached response exceeds 64 MiB")
    return CacheEntry(key, endpoint, request, ProxyResponse(status, content_type, body))


def _bearer_token(authorization: str | None) -> str | None:
    if authorization is None:
        return None
    scheme, separator, token = authorization.partition(" ")
    if separator != " " or scheme.casefold() != "bearer" or not token:
        raise ProxyError(400, "Authorization must use Bearer authentication")
    return token


def _reject_constant(value: str) -> None:
    raise ValueError(f"non-finite JSON number: {value}")


def _validate_json(value: Any) -> None:
    stack = [(value, 1)]
    while stack:
        item, depth = stack.pop()
        if depth > MAX_JSON_DEPTH:
            raise ProxyError(400, "request JSON exceeds maximum depth")
        if isinstance(item, dict):
            if len(item) > MAX_JSON_COLLECTION_ITEMS:
                raise ProxyError(400, "request JSON object has too many fields")
            for key, child in item.items():
                if len(key.encode("utf-8")) > MAX_JSON_KEY_BYTES:
                    raise ProxyError(400, "request JSON key is too large")
                stack.append((child, depth + 1))
        elif isinstance(item, list):
            if len(item) > MAX_JSON_COLLECTION_ITEMS:
                raise ProxyError(400, "request JSON array has too many items")
            stack.extend((child, depth + 1) for child in item)
        elif isinstance(item, str):
            if len(item.encode("utf-8")) > MAX_REQUEST_BYTES:
                raise ProxyError(400, "request JSON string is too large")
        elif isinstance(item, int) and not isinstance(item, bool):
            if not -MAX_JSON_INTEGER <= item <= MAX_JSON_INTEGER:
                raise ProxyError(400, "request JSON integer is out of range")
        elif isinstance(item, float) and not math.isfinite(item):
            raise ProxyError(400, "request JSON number must be finite")


def _safe_error_detail(error: Exception, authorization: str | None) -> str:
    detail = str(error).replace("\r", " ").replace("\n", " ")
    if authorization is not None:
        detail = detail.replace(authorization, "<redacted>")
        try:
            token = _bearer_token(authorization)
        except ProxyError:
            token = None
        if token is not None:
            detail = detail.replace(token, "<redacted>")
    return detail[:512]


def argument_parser() -> argparse.ArgumentParser:
    """Build the standalone proxy CLI parser."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cassette", type=Path, required=True)
    parser.add_argument("--completion-provider", required=True)
    parser.add_argument("--completion-api-base")
    parser.add_argument("--embedding-provider", required=True)
    parser.add_argument("--embedding-api-base")
    parser.add_argument("--port", type=int, default=0)
    return parser


def main() -> int:
    """Run a standalone caching proxy until Ctrl-C."""
    arguments = argument_parser().parse_args()
    try:
        cache = JsonlCache(arguments.cassette)
        backend = LiteLlmBackend(
            UpstreamRoute(arguments.completion_provider, arguments.completion_api_base),
            UpstreamRoute(arguments.embedding_provider, arguments.embedding_api_base),
        )
        server = CachingProxyServer(
            CachingProxyEngine(cache, backend),
            arguments.port,
        )
        server.start()
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    print(f"api_base: {server.api_base}")
    print(f"cassette: {arguments.cassette}")
    try:
        threading.Event().wait()
    except KeyboardInterrupt:
        pass
    finally:
        server.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
