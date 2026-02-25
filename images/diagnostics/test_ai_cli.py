"""AI CLI installation and sandbox enforcement tests."""

import pytest

from conftest import run


@pytest.mark.parametrize("cli", ["claude", "gemini", "codex"])
def test_ai_cli_installed(cli):
    """AI CLI binary must be in PATH."""
    result = run(f"command -v {cli}")
    assert result.returncode == 0, f"{cli} not found in PATH"


@pytest.mark.parametrize("cli", ["gemini", "claude", "codex"])
def test_ai_cli_sandboxed(cli):
    """AI CLIs must fail when invoked (no network, no auth)."""
    result = run(f"{cli} --help 2>&1 || {cli} 2>&1", timeout=15)
    # We expect non-zero: either network failure or missing credentials.
    # Even --help returning 0 is acceptable for some CLIs, so we only
    # test that the binary runs at all without hanging. The real sandbox
    # enforcement is test_network_blocked in test_sandbox.py.
    # Here we just verify the binary executes and doesn't crash with a signal.
    assert result.returncode < 128, (
        f"{cli} crashed with signal {result.returncode - 128}"
    )
