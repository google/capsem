"""Claude/Anthropic Ironbank release gate.

This is the provider-specific S02-022 entrypoint for Claude API, SDK, and CLI
replay proof. It delegates to the shared harness so the same ledger contract is
asserted everywhere.
"""

from __future__ import annotations

import pytest

from ironbank.model_client_assertions import assert_one_model_client
from ironbank.model_client_config import HERMETIC_ANTHROPIC_MODEL, LIVE_CLAUDE_MODEL
from ironbank.model_client_scripts import (
    claude_api_script,
    claude_ollama_launch_script,
    claude_sdk_script,
    claude_streaming_api_script,
)
from tests.ironbank.test_model_client_ledger_contract import ModelClientEnv

pytestmark = pytest.mark.integration


def test_claude_anthropic_http_replay_ledger(model_client_env: ModelClientEnv) -> None:
    result = assert_one_model_client(
        model_client_env,
        claude_api_script("https://api.anthropic.com"),
    )
    assert result["provider"] == "anthropic"
    assert result["credential_provider"] == "anthropic"
    assert result["domain"] == "api.anthropic.com"
    assert result["path"] == "/v1/messages"
    assert result["model"] == HERMETIC_ANTHROPIC_MODEL


def test_claude_streaming_replay_ledger(model_client_env: ModelClientEnv) -> None:
    result = assert_one_model_client(
        model_client_env,
        claude_streaming_api_script("https://api.anthropic.com"),
    )
    assert result["provider"] == "anthropic"
    assert result["credential_provider"] == "anthropic"
    assert result["domain"] == "api.anthropic.com"
    assert result["path"] == "/v1/messages"
    assert result["model"] == HERMETIC_ANTHROPIC_MODEL


def test_claude_sdk_replay_ledger(model_client_env: ModelClientEnv) -> None:
    result = assert_one_model_client(
        model_client_env,
        claude_sdk_script("https://api.anthropic.com"),
    )
    assert result["provider"] == "anthropic"
    assert result["credential_provider"] == "anthropic"
    assert result["domain"] == "api.anthropic.com"
    assert result["path"] == "/v1/messages"
    assert result["model"] == HERMETIC_ANTHROPIC_MODEL


def test_claude_cli_ollama_launch_replay_ledger(model_client_env: ModelClientEnv) -> None:
    result = assert_one_model_client(
        model_client_env,
        claude_ollama_launch_script(model_client_env.mock_base_url),
    )
    assert result["provider"] == "ollama"
    assert result["credential_provider"] == "ollama"
    assert result["domain"] == "127.0.0.1"
    assert result["path"] == "/v1/messages"
    assert result["tool_call_name"] == "Bash"


def test_claude_release_model_is_centralized() -> None:
    assert HERMETIC_ANTHROPIC_MODEL == "claude-sonnet-4-6"
    assert LIVE_CLAUDE_MODEL == "claude-sonnet-4-6"
