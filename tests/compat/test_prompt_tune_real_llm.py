"""Offline safety tests for the opt-in real-LLM acceptance runner."""

from __future__ import annotations

from pathlib import Path

import pytest
import yaml

from prompt_tune_real_llm import (
    build_live_settings,
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


def test_should_refuse_implicit_run_overwrite(tmp_path: Path) -> None:
    first = prepare_run_directory(tmp_path, "typed-top", clean=False)
    assert first.is_dir()

    with pytest.raises(FileExistsError, match="already exists"):
        prepare_run_directory(tmp_path, "typed-top", clean=False)


def test_should_reject_unsafe_run_name(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="run name"):
        prepare_run_directory(tmp_path, "../outside", clean=False)
