"""Offline safety tests for the opt-in real-LLM acceptance runner."""

from __future__ import annotations

import asyncio
from pathlib import Path
from types import SimpleNamespace

import pytest
import yaml

from prompt_tune_real_llm import (
    LiveRecordingModel,
    build_live_settings,
    configure_chunking,
    configure_input_pattern,
    prepare_graphrag_project,
    prepare_run_directory,
    redact,
)


def _settings(path: Path, api_key: str = "${TEST_REAL_LLM_KEY}") -> Path:
    path.write_text(
        yaml.safe_dump(
            {
                "completion_models": {
                    "default_completion_model": {
                        "api_key": api_key,
                        "auth_method": "api_key",
                        "model": "test-model",
                        "model_provider": "openai",
                        "temperature": 0,
                    }
                },
                "extract_graph": {"completion_model_id": "default_completion_model"},
            },
            sort_keys=False,
        ),
        encoding="utf-8",
        newline="\n",
    )
    return path


def test_should_redact_nested_credentials() -> None:
    value = {
        "api_key": "secret",
        "nested": [{"access_token": "secret", "temperature": 0}],
    }

    assert redact(value) == {
        "api_key": "<redacted>",
        "nested": [{"access_token": "<redacted>", "temperature": 0}],
    }


def test_should_build_sanitized_settings_without_exposing_literal_key(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("PROMPT_TUNE_REAL_LLM_API_KEY", raising=False)
    settings = build_live_settings(_settings(tmp_path / "settings.yaml", "secret"))

    completion = settings.project_settings["completion_models"][
        "default_completion_model"
    ]
    assert completion["api_key"] == "${PROMPT_TUNE_REAL_LLM_API_KEY}"
    assert settings.provider == "openai"
    assert settings.model == "test-model"
    assert "api_key" not in settings.model_parameters


def test_should_require_referenced_credential(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("TEST_REAL_LLM_KEY", raising=False)

    with pytest.raises(ValueError, match="credential environment variable is unset"):
        build_live_settings(_settings(tmp_path / "settings.yaml"))


def test_should_reject_disabled_loopback_endpoint(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("TEST_REAL_LLM_KEY", "secret")
    path = _settings(tmp_path / "settings.yaml")
    settings = yaml.safe_load(path.read_text("utf-8"))
    completion_model = settings["completion_models"]["default_completion_model"]
    completion_model["api_base"] = "http://127.0.0.1:9/v1"
    path.write_text(
        yaml.safe_dump(settings, sort_keys=False),
        encoding="utf-8",
        newline="\n",
    )

    with pytest.raises(ValueError, match="disabled loopback port 9"):
        build_live_settings(path)


def test_should_apply_explicit_live_chunking(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("TEST_REAL_LLM_KEY", "secret")

    settings = configure_chunking(
        build_live_settings(_settings(tmp_path / "settings.yaml")),
        chunk_size=1200,
        overlap=100,
        encoding_model="o200k_base",
    )

    assert settings.project_settings["chunking"] == {
        "type": "tokens",
        "size": 1200,
        "overlap": 100,
        "encoding_model": "o200k_base",
    }


def test_should_copy_and_sanitize_source_embedding_model(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("TEST_REAL_LLM_KEY", "secret")
    monkeypatch.delenv(
        "PROMPT_TUNE_REAL_LLM_EMBEDDING_API_KEY",
        raising=False,
    )
    path = _settings(tmp_path / "settings.yaml")
    source = yaml.safe_load(path.read_text("utf-8"))
    source["embedding_models"] = {
        "source_embedding": {
            "api_key": "embedding-secret",
            "auth_method": "api_key",
            "api_base": "http://localhost:11434",
            "model": "bge-m3",
            "model_provider": "ollama",
        }
    }
    source["embed_text"] = {"embedding_model_id": "source_embedding"}
    path.write_text(
        yaml.safe_dump(source, sort_keys=False),
        encoding="utf-8",
        newline="\n",
    )

    settings = build_live_settings(path)

    embedding = settings.project_settings["embedding_models"]["default_embedding_model"]
    assert embedding["api_key"] == "${PROMPT_TUNE_REAL_LLM_EMBEDDING_API_KEY}"
    assert settings.embedding_provider == "ollama"
    assert settings.embedding_model == "bge-m3"


def test_should_validate_live_input_pattern(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("TEST_REAL_LLM_KEY", "secret")
    settings = build_live_settings(_settings(tmp_path / "settings.yaml"))

    configured = configure_input_pattern(settings, r"first[.]txt")

    assert configured.project_settings["input"]["file_pattern"] == r"first[.]txt"
    with pytest.raises(ValueError, match="invalid input file pattern"):
        configure_input_pattern(settings, "[")


def test_should_copy_selected_live_inputs(tmp_path: Path) -> None:
    input_directory = tmp_path / "source"
    input_directory.mkdir()
    (input_directory / "sample.txt").write_text(
        "fixture input", encoding="utf-8", newline="\n"
    )
    project = tmp_path / "project"

    prepare_graphrag_project(project, {"input": {"type": "text"}}, input_directory)

    assert (project / "input" / "sample.txt").read_text("utf-8") == "fixture input"


def test_should_single_flight_identical_concurrent_live_requests() -> None:
    class CountingModel:
        def __init__(self) -> None:
            self.call_count = 0

        async def completion_async(self, **_kwargs: object) -> SimpleNamespace:
            self.call_count += 1
            await asyncio.sleep(0)
            return SimpleNamespace(content=f"response-{self.call_count}")

    async def run() -> tuple[list[SimpleNamespace], CountingModel]:
        delegate = CountingModel()
        recorder = LiveRecordingModel(delegate, ["selected"])
        messages = [
            {
                "role": "user",
                "content": "assigning a descriptive domain",
            }
        ]
        responses = await asyncio.gather(
            recorder.completion_async(messages=messages),
            recorder.completion_async(messages=messages),
        )
        return responses, delegate

    responses, delegate = asyncio.run(run())

    assert delegate.call_count == 1
    assert [response.content for response in responses] == [
        "response-1",
        "response-1",
    ]


def test_should_refuse_implicit_run_overwrite(tmp_path: Path) -> None:
    first = prepare_run_directory(tmp_path, "typed-top", clean=False)
    assert first.is_dir()

    with pytest.raises(FileExistsError, match="already exists"):
        prepare_run_directory(tmp_path, "typed-top", clean=False)


def test_should_reject_unsafe_run_name(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="run name"):
        prepare_run_directory(tmp_path, "../outside", clean=False)
