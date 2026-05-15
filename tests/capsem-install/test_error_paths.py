"""Error path tests for failure scenarios.

Verifies that failure modes produce actionable error messages (not stack
traces or silent failures) and that the system degrades gracefully.
"""

from __future__ import annotations

import json
import os
import platform
import stat
import subprocess
import sys
from pathlib import Path

import pytest

from .conftest import (
    CAPSEM_DIR,
    INSTALL_DIR,
    RUN_DIR,
    ASSETS_DIR,
    run_capsem,
)

REPO_ROOT = Path(__file__).resolve().parents[2]
CAPTURE_STATUS_SCRIPT = REPO_ROOT / "scripts" / "capture-install-status.py"


def _capture_installed_status(out_dir: Path) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env["CAPSEM_HOME"] = str(CAPSEM_DIR)
    env["CAPSEM_RUN_DIR"] = str(RUN_DIR)
    return subprocess.run(
        [
            sys.executable,
            str(CAPTURE_STATUS_SCRIPT),
            "--capsem-bin",
            str(INSTALL_DIR / "capsem"),
            "--out-dir",
            str(out_dir),
        ],
        capture_output=True,
        text=True,
        timeout=15,
        env=env,
        check=False,
    )


def _host_manifest_arch() -> str:
    machine = platform.machine()
    if machine in {"aarch64", "arm64"}:
        return "arm64"
    return machine


def _manifest_asset_path(logical_name: str) -> Path:
    manifest = json.loads((ASSETS_DIR / "manifest.json").read_text())
    release = manifest["assets"]["current"]
    arch = _host_manifest_arch()
    entry = manifest["assets"]["releases"][release]["arches"][arch][logical_name]
    stem, dot, suffix = logical_name.partition(".")
    hashed = f"{stem}-{entry['hash'][:16]}"
    if dot:
        hashed += f".{suffix}"
    return ASSETS_DIR / arch / hashed


def _write_completed_setup_state() -> None:
    (CAPSEM_DIR / "setup-state.json").write_text(
        json.dumps(
            {
                "schema_version": 2,
                "completed_steps": ["summary"],
                "security_preset": "medium",
                "providers_done": True,
                "repositories_done": True,
                "service_installed": True,
                "vm_verified": False,
                "corp_config_source": None,
                "install_completed": True,
                "onboarding_completed": False,
                "onboarding_version": 0,
            }
        )
    )


class TestErrorPaths:
    """Failure scenarios with actionable error messages."""

    def test_bad_service_binary(self, installed_layout, clean_state):
        """Broken capsem-service gives error, not hang."""
        service_bin = INSTALL_DIR / "capsem-service"
        original = service_bin.read_bytes()
        try:
            # unlink-then-write: writing over the mapped binary of a still-
            # running service process raises ETXTBSY on Linux. Unlinking
            # the path breaks the inode association; a subsequent write
            # creates a fresh inode so any lingering exec handle on the
            # old inode doesn't block us. The `finally` does the same
            # restore so a flaky cleanup can't wedge the installed prefix.
            service_bin.unlink()
            service_bin.write_text("#!/bin/sh\nexit 1\n")
            service_bin.chmod(stat.S_IRWXU | stat.S_IRGRP | stat.S_IXGRP)

            result = run_capsem("list", timeout=15)
            assert result.returncode != 0
            combined = (result.stdout + result.stderr).lower()
            assert "error" in combined or "failed" in combined, (
                f"expected error message: {result.stdout}{result.stderr}"
            )
        finally:
            service_bin.unlink(missing_ok=True)
            service_bin.write_bytes(original)
            service_bin.chmod(stat.S_IRWXU | stat.S_IRGRP | stat.S_IXGRP)

    @pytest.mark.live_system
    def test_missing_assets_dir(self, installed_layout, clean_state):
        """Missing assets directory gives clear error."""
        backup = ASSETS_DIR.parent / "assets_backup"
        moved = False
        try:
            if ASSETS_DIR.exists():
                ASSETS_DIR.rename(backup)
                moved = True

            result = run_capsem("list", timeout=15)
            assert result.returncode != 0
            combined = (result.stdout + result.stderr).lower()
            assert "assets" in combined or "error" in combined
        finally:
            if moved and backup.exists():
                backup.rename(ASSETS_DIR)

    def test_corrupt_setup_state(self, installed_layout, clean_state):
        """Corrupt setup-state.json doesn't crash setup."""
        CAPSEM_DIR.mkdir(parents=True, exist_ok=True)
        state_file = CAPSEM_DIR / "setup-state.json"
        state_file.write_text("{{{invalid json")

        result = run_capsem("setup", "--non-interactive", timeout=15)
        # Should succeed (treat corrupt state as fresh)
        assert result.returncode == 0, (
            f"setup should handle corrupt state:\n{result.stdout}{result.stderr}"
        )

    def test_wrong_permissions_on_capsem_dir(self, installed_layout, clean_state):
        """Read-only ~/.capsem gives clear write error."""
        CAPSEM_DIR.mkdir(parents=True, exist_ok=True)
        original_mode = CAPSEM_DIR.stat().st_mode
        try:
            CAPSEM_DIR.chmod(stat.S_IRUSR | stat.S_IXUSR)  # read-only

            result = run_capsem("setup", "--non-interactive", timeout=15)
            # Should fail with permission error
            combined = (result.stdout + result.stderr).lower()
            if result.returncode != 0:
                assert "permission" in combined or "denied" in combined or "error" in combined
        finally:
            CAPSEM_DIR.chmod(original_mode)

    def test_stale_socket(self, installed_layout, clean_state):
        """Stale socket file doesn't prevent auto-launch."""
        RUN_DIR.mkdir(parents=True, exist_ok=True)
        stale = RUN_DIR / "service.sock"
        # Create a regular file pretending to be a socket
        stale.write_text("")

        result = run_capsem("list", timeout=15)
        # Should either connect (auto-launch cleans up) or give clear error
        combined = (result.stdout + result.stderr).lower()
        assert result.returncode == 0 or "error" in combined or "failed" in combined

        # Clean up
        stale.unlink(missing_ok=True)

    def test_version_works_without_service(self, installed_layout, clean_state):
        """capsem version works even when service is down."""
        result = run_capsem("version", timeout=5)
        assert result.returncode == 0
        assert "capsem" in result.stdout
        assert "build" in result.stdout

    def test_status_json_reports_typed_install_blockers(self, installed_layout, clean_state):
        """capsem status --json reports machine-readable blockers."""
        state_file = CAPSEM_DIR / "setup-state.json"
        state_file.unlink(missing_ok=True)

        result = run_capsem("status", "--json", timeout=10)
        assert result.stdout, f"status --json should print a report: {result.stderr}"
        report = json.loads(result.stdout)

        assert result.returncode != 0
        assert report["schema"] == "capsem.status.v1"
        assert report["ok"] is False
        assert report["state"] == "blocked"
        assert report["service"]["running"] is False
        codes = {issue["code"] for issue in report["issues"]}
        assert "service_not_running" in codes
        assert "setup_state_missing" in codes
        assert report["checks"]["service_endpoint"]["state"] == "blocked"
        assert report["checks"]["setup"]["state"] == "blocked"
        assert report["checks"]["gateway"]["state"] == "skipped"

    def test_status_json_reports_missing_helper_binary(self, installed_layout, clean_state):
        """capsem status --json reports missing helper binaries by code."""
        service_bin = INSTALL_DIR / "capsem-service"
        backup = INSTALL_DIR / "capsem-service.bak"
        service_bin.rename(backup)
        try:
            result = run_capsem("status", "--json", timeout=10)
            assert result.stdout, f"status --json should print a report: {result.stderr}"
            report = json.loads(result.stdout)
            missing = [
                issue
                for issue in report["issues"]
                if issue["code"] == "host_binary_missing"
            ]
            assert missing, report
            assert missing[0]["details"]["name"] == "capsem-service"
            assert missing[0]["details"]["path"].endswith("/capsem-service")
            assert report["checks"]["host"]["state"] == "blocked"
            assert "host_binary_missing" in report["checks"]["host"]["issue_codes"]
        finally:
            backup.rename(service_bin)

    def test_status_capture_records_partial_install_missing_helper(
        self, installed_layout, clean_state, tmp_path
    ):
        """S2 evidence capture preserves typed status for a partial install."""
        service_bin = INSTALL_DIR / "capsem-service"
        backup = INSTALL_DIR / "capsem-service.bak"
        service_bin.rename(backup)
        try:
            result = _capture_installed_status(tmp_path / "bundle")
            assert result.returncode != 0
            assert result.stdout.strip() == str(tmp_path / "bundle")

            metadata = json.loads(
                (tmp_path / "bundle" / "capture.meta.json").read_text(encoding="utf-8")
            )
            assert "host_binary_missing" in metadata["status_issue_codes"]
            assert metadata["status_checks"]["host"]["state"] == "blocked"

            report = json.loads(
                (tmp_path / "bundle" / "status.json").read_text(encoding="utf-8")
            )
            missing = [
                issue
                for issue in report["issues"]
                if issue["code"] == "host_binary_missing"
                and issue["details"]["name"] == "capsem-service"
            ]
            assert missing, report
            assert missing[0]["details"]["path"].endswith("/capsem-service")
        finally:
            backup.rename(service_bin)

    def test_status_capture_records_missing_tray_helper(
        self, installed_layout, clean_state, tmp_path
    ):
        """S2 evidence capture preserves tray helper gaps in partial installs."""
        tray_bin = INSTALL_DIR / "capsem-tray"
        backup = INSTALL_DIR / "capsem-tray.bak"
        tray_bin.rename(backup)
        try:
            result = _capture_installed_status(tmp_path / "bundle")
            assert result.returncode != 0

            bundle = tmp_path / "bundle"
            metadata = json.loads((bundle / "capture.meta.json").read_text(encoding="utf-8"))
            assert "host_binary_missing" in metadata["status_issue_codes"]

            report = json.loads((bundle / "status.json").read_text(encoding="utf-8"))
            missing = [
                issue
                for issue in report["issues"]
                if issue["code"] == "host_binary_missing"
                and issue["details"]["name"] == "capsem-tray"
            ]
            assert missing, report

            layout = json.loads((bundle / "install-layout.json").read_text(encoding="utf-8"))
            assert layout["binaries"]["capsem-tray"]["kind"] == "missing"
            assert metadata["status_checks"]["host"]["state"] == "blocked"
        finally:
            backup.rename(tray_bin)

    def test_status_capture_records_dead_service(self, installed_layout, clean_state, tmp_path):
        """S2 evidence capture preserves typed status when the daemon is down."""
        _write_completed_setup_state()

        result = _capture_installed_status(tmp_path / "bundle")
        assert result.returncode != 0

        bundle = tmp_path / "bundle"
        metadata = json.loads((bundle / "capture.meta.json").read_text(encoding="utf-8"))
        assert "service_not_running" in metadata["status_issue_codes"]
        assert "debug" in metadata["commands"]

        report = json.loads((bundle / "status.json").read_text(encoding="utf-8"))
        codes = {issue["code"] for issue in report["issues"]}
        assert "service_not_running" in codes
        assert report["checks"]["service_endpoint"]["state"] == "blocked"
        assert report["checks"]["gateway"]["state"] == "skipped"

        run_state = json.loads((bundle / "run-state.json").read_text(encoding="utf-8"))
        entries = {entry["path"]: entry for entry in run_state["entries"]}
        assert entries["service.sock"]["kind"] in {"missing", "other"}
        assert entries["gateway.port"]["kind"] == "missing"

    def test_status_json_reports_missing_mcp_helper_binary(self, installed_layout, clean_state):
        """capsem status --json covers MCP helper binaries in the install layout."""
        helper_bin = INSTALL_DIR / "capsem-mcp-builtin"
        backup = INSTALL_DIR / "capsem-mcp-builtin.bak"
        helper_bin.rename(backup)
        try:
            result = run_capsem("status", "--json", timeout=10)
            assert result.stdout, f"status --json should print a report: {result.stderr}"
            report = json.loads(result.stdout)
            missing = [
                issue
                for issue in report["issues"]
                if issue["code"] == "host_binary_missing"
                and issue["details"]["name"] == "capsem-mcp-builtin"
            ]
            assert missing, report
            assert missing[0]["details"]["path"].endswith("/capsem-mcp-builtin")
            assert report["checks"]["host"]["state"] == "blocked"
        finally:
            backup.rename(helper_bin)

    def test_status_json_reports_stale_process_helper_binary(self, installed_layout, clean_state):
        """capsem status --json reports helper version skew by stable code."""
        helper_bin = INSTALL_DIR / "capsem-process"
        backup = INSTALL_DIR / "capsem-process.bak"
        helper_bin.rename(backup)
        helper_bin.write_text("#!/bin/sh\nprintf 'capsem-process 0.0.0\\n'\n")
        helper_bin.chmod(stat.S_IRWXU | stat.S_IRGRP | stat.S_IXGRP)
        try:
            result = run_capsem("status", "--json", timeout=10)
            assert result.stdout, f"status --json should print a report: {result.stderr}"
            report = json.loads(result.stdout)
            mismatches = [
                issue
                for issue in report["issues"]
                if issue["code"] == "host_binary_version_mismatch"
                and issue["details"]["name"] == "capsem-process"
            ]
            assert result.returncode != 0
            assert mismatches, report
            assert mismatches[0]["details"]["actual_version"] == "0.0.0"
            assert report["checks"]["host"]["state"] == "blocked"
        finally:
            helper_bin.unlink(missing_ok=True)
            backup.rename(helper_bin)

    def test_status_json_reports_corrupt_setup_state(self, installed_layout, clean_state):
        """capsem status --json reports corrupt setup state by code."""
        state_file = CAPSEM_DIR / "setup-state.json"
        state_file.write_text("{not json")

        result = run_capsem("status", "--json", timeout=10)
        assert result.stdout, f"status --json should print a report: {result.stderr}"
        report = json.loads(result.stdout)
        invalid = [
            issue
            for issue in report["issues"]
            if issue["code"] == "setup_state_invalid"
        ]
        assert invalid, report
        assert invalid[0]["details"]["path"].endswith("setup-state.json")
        assert report["checks"]["setup"]["state"] == "blocked"

    def test_status_json_reports_missing_asset_manifest(self, installed_layout, clean_state):
        """capsem status --json reports a missing manifest by stable code."""
        manifest = ASSETS_DIR / "manifest.json"
        backup = ASSETS_DIR / "manifest.json.bak"
        manifest.rename(backup)
        try:
            result = run_capsem("status", "--json", timeout=10)
            assert result.stdout, f"status --json should print a report: {result.stderr}"
            report = json.loads(result.stdout)
            codes = {issue["code"] for issue in report["issues"]}
            assert result.returncode != 0
            assert "manifest_missing" in codes
            assert report["checks"]["assets"]["state"] == "blocked"
        finally:
            backup.rename(manifest)

    def test_status_json_reports_missing_rootfs_asset(self, installed_layout, clean_state):
        """capsem status --json reports the missing canonical rootfs asset."""
        rootfs = _manifest_asset_path("rootfs.squashfs")
        backup = rootfs.with_suffix(rootfs.suffix + ".bak")
        rootfs.rename(backup)
        try:
            result = run_capsem("status", "--json", timeout=10)
            assert result.stdout, f"status --json should print a report: {result.stderr}"
            report = json.loads(result.stdout)
            missing = [
                issue
                for issue in report["issues"]
                if issue["code"] == "rootfs_asset_missing"
            ]
            assert result.returncode != 0
            assert missing, report
            assert missing[0]["details"]["path"].endswith(rootfs.name)
            assert report["checks"]["assets"]["state"] == "blocked"
        finally:
            backup.rename(rootfs)

    def test_status_json_accepts_completed_setup_state(self, installed_layout, clean_state):
        """capsem status --json does not invent setup blockers for completed setup."""
        _write_completed_setup_state()

        result = run_capsem("status", "--json", timeout=10)
        assert result.stdout, f"status --json should print a report: {result.stderr}"
        report = json.loads(result.stdout)
        codes = {issue["code"] for issue in report["issues"]}
        assert "setup_state_missing" not in codes
        assert "setup_state_invalid" not in codes
        assert "setup_incomplete" not in codes
        assert report["checks"]["setup"]["state"] == "ok"

    @pytest.mark.live_system
    def test_service_status_works_without_install(self, installed_layout, clean_state):
        """capsem status reports health even when not installed."""
        result = run_capsem("status", timeout=10)
        assert "Installed:" in result.stdout

    def test_completions_work_without_service(self, installed_layout):
        """capsem completions works without service running."""
        result = run_capsem("completions", "bash", timeout=5)
        assert result.returncode == 0
        assert "capsem" in result.stdout
