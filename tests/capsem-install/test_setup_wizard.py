"""Setup wizard tests for WB2.

Tests verify `capsem setup` in non-interactive mode: completes without
prompts, persists state, respects --force, and writes user.toml.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from .conftest import (
    CAPSEM_DIR,
    run_capsem,
)

SETUP_STATE = CAPSEM_DIR / "setup-state.json"
USER_TOML = CAPSEM_DIR / "user.toml"


@pytest.fixture
def clean_setup_state():
    """Remove setup state before and after test."""
    SETUP_STATE.unlink(missing_ok=True)
    USER_TOML.unlink(missing_ok=True)
    yield
    SETUP_STATE.unlink(missing_ok=True)


@pytest.mark.live_system
class TestSetupWizard:
    """capsem setup non-interactive mode."""

    def test_non_interactive_setup(self, installed_layout, clean_state, clean_setup_state):
        """Non-interactive setup completes without prompts, state file written."""
        result = run_capsem("setup", "--non-interactive", timeout=30)
        assert result.returncode == 0, (
            f"setup failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
        )

        assert SETUP_STATE.exists(), "setup-state.json should be written"
        state = json.loads(SETUP_STATE.read_text())
        assert state.get("schema_version") == 2
        assert "welcome" in state.get("completed_steps", [])
        assert "security_preset" in state.get("completed_steps", [])
        # install_completed tracks CLI install finish (separate from GUI wizard).
        assert state.get("install_completed") is True, (
            "successful non-interactive setup must flip install_completed"
        )
        # GUI wizard must NOT be auto-completed -- that's the app's job.
        assert state.get("onboarding_completed") is False

    def test_setup_rerun_skips_completed(self, installed_layout, clean_state, clean_setup_state):
        """Second run skips done steps."""
        # First run
        r1 = run_capsem("setup", "--non-interactive", timeout=30)
        assert r1.returncode == 0, f"first run failed: {r1.stderr}"

        # Second run -- should skip completed steps
        r2 = run_capsem("setup", "--non-interactive", timeout=30)
        assert r2.returncode == 0, (
            f"second run failed:\nstdout: {r2.stdout}\nstderr: {r2.stderr}"
        )

        state = json.loads(SETUP_STATE.read_text())
        # Steps should still be marked done
        assert "welcome" in state.get("completed_steps", [])

    def test_setup_force_reruns_all(self, installed_layout, clean_state, clean_setup_state):
        """--force re-runs all steps even if previously completed."""
        # First run
        r1 = run_capsem("setup", "--non-interactive", timeout=30)
        assert r1.returncode == 0

        # Force re-run
        r2 = run_capsem("setup", "--non-interactive", "--force", timeout=30)
        assert r2.returncode == 0, (
            f"force rerun failed:\nstdout: {r2.stdout}\nstderr: {r2.stderr}"
        )

        state = json.loads(SETUP_STATE.read_text())
        assert "welcome" in state.get("completed_steps", [])
        assert "security_preset" in state.get("completed_steps", [])

    def test_force_onboarding_resets_only_wizard_flags(self, installed_layout, clean_state, clean_setup_state):
        """--force-onboarding resets GUI wizard flags but keeps install state."""
        # Run setup to completion so install_completed flips true.
        r1 = run_capsem("setup", "--non-interactive", timeout=30)
        assert r1.returncode == 0

        # Simulate a user who has completed the GUI wizard.
        state = json.loads(SETUP_STATE.read_text())
        state["onboarding_completed"] = True
        state["onboarding_version"] = 1
        SETUP_STATE.write_text(json.dumps(state))

        # Now reset onboarding.
        r2 = run_capsem("setup", "--force-onboarding", timeout=10)
        assert r2.returncode == 0, (
            f"force-onboarding failed:\nstdout: {r2.stdout}\nstderr: {r2.stderr}"
        )

        after = json.loads(SETUP_STATE.read_text())
        assert after.get("onboarding_completed") is False, "wizard flag must reset"
        assert after.get("onboarding_version") == 0, "wizard version must reset"
        # Install state must survive.
        assert after.get("install_completed") is True, "install_completed must persist"
        assert "summary" in after.get("completed_steps", []), "completed steps must persist"

    def test_setup_writes_user_toml(self, installed_layout, clean_state, clean_setup_state):
        """Security preset writes user.toml."""
        result = run_capsem("setup", "--non-interactive", "--preset", "medium", timeout=30)
        assert result.returncode == 0, (
            f"setup failed:\nstdout: {result.stdout}\nstderr: {result.stderr}"
        )

        # user.toml should exist after applying a preset
        assert USER_TOML.exists(), "user.toml should be written by apply_preset"

        state = json.loads(SETUP_STATE.read_text())
        assert state.get("security_preset") == "medium"
