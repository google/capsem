"""AI CLI installation and sandbox enforcement tests."""

import pytest

from conftest import run


@pytest.mark.parametrize("cli", ["claude", "gemini", "codex"])
def test_ai_cli_installed(cli):
    """AI CLI binary must be in PATH."""
    result = run(f"command -v {cli}")
    assert result.returncode == 0, f"{cli} not found in PATH"


@pytest.mark.parametrize("cli", ["gemini", "claude", "codex"])
def test_ai_cli_help(cli):
    """AI CLI --help must execute without runtime errors."""
    result = run(f"{cli} --help 2>&1", timeout=15)
    output = result.stdout
    # Must not crash with a JS/Node runtime error
    for error in ["SyntaxError", "TypeError", "ReferenceError", "Cannot find module"]:
        assert error not in output, f"{cli} --help has runtime error: {error}"
    assert result.returncode == 0, (
        f"{cli} --help failed (rc={result.returncode}): {output[:300]}"
    )
