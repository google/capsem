"""Setup wizard tests for WB2.

Tests verify `capsem setup` in non-interactive mode: completes without
prompts, persists state, respects --force, and writes user.toml.
"""

from __future__ import annotations

import json
import stat
from pathlib import Path
import tomllib

import pytest

from .conftest import (
    CAPSEM_DIR,
    ASSETS_DIR,
    INSTALL_DIR,
    RUN_DIR,
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


@pytest.fixture
def setup_isolation_env(monkeypatch):
    """Force setup's test-isolation mode in packaging/non-live harnesses.

    This keeps setup from writing persistent LaunchAgent/systemd units while
    still exercising the real setup summary/service-truth path.
    """
    monkeypatch.setenv("CAPSEM_HOME", str(CAPSEM_DIR))
    monkeypatch.setenv("CAPSEM_RUN_DIR", str(RUN_DIR))
    monkeypatch.setenv("CAPSEM_ASSETS_DIR", str(ASSETS_DIR))


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


class TestSetupWizardHarness:
    """Packaging-safe S5 setup harness proofs."""

    def test_setup_rerun_is_idempotent_under_isolation(
        self,
        installed_layout,
        clean_state,
        clean_setup_state,
        setup_isolation_env,
    ):
        """Second non-interactive run keeps setup state stable."""
        first = run_capsem("setup", "--non-interactive", "--accept-detected", timeout=30)
        assert first.returncode == 0, (
            f"first setup run failed:\nstdout: {first.stdout}\nstderr: {first.stderr}"
        )
        state1 = json.loads(SETUP_STATE.read_text())

        second = run_capsem("setup", "--non-interactive", "--accept-detected", timeout=30)
        assert second.returncode == 0, (
            f"second setup run failed:\nstdout: {second.stdout}\nstderr: {second.stderr}"
        )
        state2 = json.loads(SETUP_STATE.read_text())

        assert state1 == state2, "setup rerun should not mutate settled state"
        assert state2.get("install_completed") is True
        assert state2.get("service_installed") is False
        assert state2.get("providers_done") is True
        assert state2.get("repositories_done") is True
        assert "summary" in state2.get("completed_steps", [])

    def test_setup_provider_settings_fallback_with_no_detected_keys(
        self,
        installed_layout,
        clean_state,
        clean_setup_state,
        setup_isolation_env,
        monkeypatch,
        tmp_path,
    ):
        """Setup succeeds and falls back cleanly when provider detection is empty."""
        # Make host detection deterministic: no git/ssh/provider files under HOME
        # and no API key env vars.
        monkeypatch.setenv("HOME", str(tmp_path / "empty-home"))
        (tmp_path / "empty-home").mkdir(parents=True, exist_ok=True)
        for key in (
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "GOOGLE_API_KEY",
            "GEMINI_API_KEY",
            "GITHUB_TOKEN",
        ):
            monkeypatch.delenv(key, raising=False)

        result = run_capsem("setup", "--non-interactive", "--accept-detected", timeout=30)
        assert result.returncode == 0, (
            f"setup should succeed with empty provider detection:\nstdout: {result.stdout}\nstderr: {result.stderr}"
        )
        assert "No API keys detected. Configure later with `capsem setup --force`." in result.stdout

        # Security preset still writes a valid settings file even with no
        # provider credentials detected.
        parsed = tomllib.loads(USER_TOML.read_text())
        assert isinstance(parsed, dict), "user.toml must be valid TOML after setup"

        state = json.loads(SETUP_STATE.read_text())
        assert state.get("providers_done") is True
        assert state.get("install_completed") is True
        assert "summary" in state.get("completed_steps", [])

    def test_setup_reports_pending_when_service_never_becomes_live(
        self,
        installed_layout,
        clean_state,
        clean_setup_state,
        setup_isolation_env,
    ):
        """Setup completes config but reports pending VM readiness on dead service."""
        service_bin = INSTALL_DIR / "capsem-service"
        original = service_bin.read_bytes()
        try:
            # Break direct auto-launch in isolation mode so /list never comes up.
            service_bin.unlink()
            service_bin.write_text("#!/bin/sh\nexit 1\n")
            service_bin.chmod(stat.S_IRWXU | stat.S_IRGRP | stat.S_IXGRP)

            result = run_capsem("setup", "--non-interactive", "--accept-detected", timeout=30)
            assert result.returncode == 0, (
                f"setup should complete with pending readiness when service is unavailable:\n"
                f"stdout: {result.stdout}\nstderr: {result.stderr}"
            )
            combined = f"{result.stdout}\n{result.stderr}"
            assert "Service asset status unavailable" in combined
            assert "VM readiness is not verified" in combined

            state = json.loads(SETUP_STATE.read_text())
            assert state.get("vm_verified") is False
            assert state.get("install_completed") is True
            assert "summary" in state.get("completed_steps", [])
        finally:
            service_bin.unlink(missing_ok=True)
            service_bin.write_bytes(original)
            service_bin.chmod(stat.S_IRWXU | stat.S_IRGRP | stat.S_IXGRP)
