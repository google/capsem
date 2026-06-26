"""Gemini Ironbank release gate.

This file gives S02-021 a provider-specific executable gate while reusing the
shared black-box service/VM/mock-server harness.
"""

from __future__ import annotations

import pytest

from ironbank.model_client_assertions import assert_one_model_client
from ironbank.model_client_config import (
    HERMETIC_GEMINI_MODEL,
    LIVE_GEMINI_IMAGE_MODEL,
    LIVE_GEMINI_TEXT_MODEL,
)
from ironbank.model_client_scripts import gemini_api_script
from tests.ironbank.test_model_client_ledger_contract import ModelClientEnv

pytestmark = pytest.mark.integration


def test_gemini_streaming_and_nonstreaming_replay_ledger(
    model_client_env: ModelClientEnv,
) -> None:
    result = assert_one_model_client(
        model_client_env,
        gemini_api_script("https://generativelanguage.googleapis.com"),
    )
    assert result["provider"] == "google"
    assert result["credential_provider"] == "google"
    assert result["domain"] == "generativelanguage.googleapis.com"
    assert result["path"] == f"/v1beta/models/{HERMETIC_GEMINI_MODEL}:streamGenerateContent"
    assert result["model"] == HERMETIC_GEMINI_MODEL
    assert result["tool_call_name"] == "write_to_file"
    assert result["file_matches"] is True


def test_gemini_release_models_are_centralized() -> None:
    assert HERMETIC_GEMINI_MODEL == "gemini-3.5-flash"
    assert LIVE_GEMINI_TEXT_MODEL == "gemini-3.5-flash"
    assert LIVE_GEMINI_IMAGE_MODEL == "gemini-3.1-flash-image-preview"
