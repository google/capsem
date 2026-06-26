"""Ironbank proof for the real Claude CLI path.

This file is the dedicated S02-008 gate. The shared model-client harness owns
the service, VM, mock-server, DB, route, and log plumbing; this test keeps the
Claude CLI proof discoverable as its own release ledger item.
"""

from __future__ import annotations

import pytest

from ironbank.model_client_assertions import assert_one_model_client
from ironbank.model_client_scripts import claude_api_script, claude_ollama_launch_script

pytestmark = pytest.mark.integration


def test_claude_cli_ollama_launch_pays_full_ledger_debt(
    model_client_env,
) -> None:
    result = assert_one_model_client(
        model_client_env,
        claude_ollama_launch_script(model_client_env.mock_base_url),
    )
    assert result["provider"] == "ollama"
    assert result["credential_provider"] == "ollama"
    assert result["domain"] == "127.0.0.1"
    assert result["path"] == "/v1/messages"
    assert result["tool_call_name"] == "Bash"
    assert result["call_args"]["command"].startswith("printf '%s\\n' ")
    assert result["target"].startswith("/root/claude-ollama-launch-")
    assert result["file_text"] == result["nonce"] + "\n"


def test_claude_anthropic_protocol_brokers_api_key(
    model_client_env,
) -> None:
    result = assert_one_model_client(
        model_client_env,
        claude_api_script("https://api.anthropic.com"),
    )
    assert result["provider"] == "anthropic"
    assert result["credential_provider"] == "anthropic"
    assert result["domain"] == "api.anthropic.com"
    assert result["path"] == "/v1/messages"
    assert result["tool_call_name"] == "exec_command"
    assert result["target"].startswith("/root/claude-api-")
    assert result["file_text"] == result["nonce"] + "\n"
