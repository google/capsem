"""Regression tests for removing the legacy setup command."""

from __future__ import annotations

from .conftest import CAPSEM_DIR, run_capsem

SETUP_STATE = CAPSEM_DIR / "setup-state.json"
USER_TOML = CAPSEM_DIR / "settings.toml"


def test_setup_command_is_removed(installed_layout, clean_state):
    """`capsem setup` must not parse after the install/setup rebuild."""
    SETUP_STATE.unlink(missing_ok=True)
    USER_TOML.unlink(missing_ok=True)

    result = run_capsem("setup", "--non-interactive", timeout=10)

    assert result.returncode != 0
    combined = f"{result.stdout}\n{result.stderr}".lower()
    assert "unrecognized" in combined or "invalid" in combined
    assert not SETUP_STATE.exists(), "removed setup command must not write setup-state.json"
    assert not USER_TOML.exists(), "removed setup command must not write settings.toml"
