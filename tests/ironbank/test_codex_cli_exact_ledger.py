"""Ironbank proof for the real Codex CLI model/tool/file ledger path.

This is the dedicated S02-017 gate. The shared model-client harness owns the
service, VM, mock-server, DB, route, and log plumbing; this file keeps the real
Codex CLI proof discoverable as a release item.
"""

from __future__ import annotations

import pytest
from ironbank.model_client_assertions import assert_one_model_client
from ironbank.model_client_config import HERMETIC_OPENAI_COMPAT_MODEL, WORKLOAD_OLLAMA_HOST
from ironbank.model_client_scripts import codex_cli_script, codex_ollama_launch_script

pytestmark = pytest.mark.integration


def test_codex_cli_exec_pays_full_ledger_debt(model_client_env) -> None:
    result = assert_one_model_client(
        model_client_env,
        codex_cli_script(model_client_env.mock_base_url),
    )
    assert result["provider"] == "ollama"
    assert result["credential_provider"] == "openai"
    assert result["domain"] == WORKLOAD_OLLAMA_HOST
    assert result["path"] == "/v1/responses"
    assert result["model"] == HERMETIC_OPENAI_COMPAT_MODEL
    assert result["tool_call_name"] == "exec_command"
    assert result["call_args"]["cmd"].startswith("printf '%s\\n' ")
    assert result["target"].startswith("/workspace/codex-cli-")
    assert result["file_text"] == result["nonce"] + "\n"


def test_codex_ollama_launch_pays_full_ledger_debt(model_client_env) -> None:
    result = assert_one_model_client(
        model_client_env,
        codex_ollama_launch_script(model_client_env.mock_base_url),
    )
    assert result["provider"] == "ollama"
    assert result["credential_provider"] == "ollama"
    assert result["domain"] == WORKLOAD_OLLAMA_HOST
    assert result["path"] == "/v1/responses"
    assert result["model"] == HERMETIC_OPENAI_COMPAT_MODEL
    assert result["tool_call_name"] == "exec_command"
    assert result["call_args"]["cmd"].startswith("printf '%s\\n' ")
    assert result["target"].startswith("/workspace/codex-ollama-launch-")
    assert result["file_text"] == result["nonce"] + "\n"
