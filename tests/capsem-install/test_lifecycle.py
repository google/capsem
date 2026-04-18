"""Full lifecycle integration test.

Exercises every WB in sequence: install -> setup -> list -> service status
-> update check -> uninstall. Catches cross-WB socket-path mismatches,
state-file conflicts, and service-restart races.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from .conftest import (
    CAPSEM_DIR,
    INSTALL_DIR,
    RUN_DIR,
    run_capsem,
)


@pytest.mark.live_system
class TestLifecycle:
    """Full user journey in a single test."""

    def test_full_lifecycle(self, installed_layout, clean_state, systemd_available):
        """install -> setup -> list -> service status -> update -> uninstall."""

        # 1. Fresh install (handled by installed_layout fixture)
        assert INSTALL_DIR.exists()

        # 2. Setup wizard (non-interactive)
        r = run_capsem("setup", "--non-interactive", "--preset", "medium", "--accept-detected", timeout=30)
        assert r.returncode == 0, f"setup failed: {r.stderr}"
        assert (CAPSEM_DIR / "setup-state.json").exists()

        # 3. First command triggers auto-launch
        r = run_capsem("list", timeout=15)
        assert r.returncode == 0, f"list failed: {r.stderr}"

        # 4. Service management
        r = run_capsem("service", "install", timeout=15)
        assert r.returncode == 0, f"service install failed: {r.stderr}"

        r = run_capsem("service", "status", timeout=10)
        assert r.returncode == 0, f"service status failed: {r.stderr}"
        assert "Installed: true" in r.stdout

        # 5. Update check (installed layout, not dev build)
        r = run_capsem("update", "--yes", timeout=30)
        # May fail due to no network, but should not crash
        assert "Development build" not in r.stdout

        # 6. Service uninstall before full uninstall
        r = run_capsem("service", "uninstall", timeout=15)
        assert r.returncode == 0, f"service uninstall failed: {r.stderr}"

        # 7. Full uninstall
        r = run_capsem("uninstall", "--yes", timeout=15)
        assert r.returncode == 0, f"uninstall failed: {r.stderr}"
        assert not (CAPSEM_DIR / "bin" / "capsem").exists(), "binary should be removed"
