"""capsem_run: one-shot command execution in a fresh ephemeral VM."""

import pytest

from helpers.mcp import parse_content

pytestmark = pytest.mark.mcp


def test_run_stdout(mcp_session):
    """capsem_run returns stdout from the command."""
    res = mcp_session.call_tool("capsem_run", {"command": "echo hello-run"})
    payload = parse_content(res)
    assert "hello-run" in payload["stdout"]
    assert payload["exit_code"] == 0


def test_run_exit_code(mcp_session):
    """Non-zero exit codes propagate back as the exit_code field."""
    res = mcp_session.call_tool("capsem_run", {"command": "false"})
    payload = parse_content(res)
    # `false` exits with code 1 on Linux.
    assert payload["exit_code"] != 0, f"expected non-zero exit, got {payload}"


def test_run_env(mcp_session):
    """env={...} arguments are injected into the guest."""
    res = mcp_session.call_tool("capsem_run", {
        "command": "echo RUN_ENV=$RUN_ENV_KEY",
        "env": {"RUN_ENV_KEY": "run-env-value"},
    })
    payload = parse_content(res)
    assert "RUN_ENV=run-env-value" in payload["stdout"], (
        f"env not injected: {payload['stdout']!r}"
    )
    assert payload["exit_code"] == 0
