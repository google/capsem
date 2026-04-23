"""Tests for capsem.builder.doctor -- composable build prerequisite checks.

TDD: tests written first (RED), then doctor.py makes them pass (GREEN).
All subprocess/shutil calls are mocked -- no real tools needed to run tests.
"""

from __future__ import annotations

import subprocess
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

from capsem.builder.doctor import (
    MAX_CLOCK_SKEW_SECONDS,
    CheckResult,
    check_b3sum,
    check_container_clock,
    check_container_resources,
    check_container_runtime,
    check_cross_target,
    check_guest_config,
    check_rust_toolchain,
    check_source_files,
    format_results,
    run_all_checks,
)


# ---------------------------------------------------------------------------
# CheckResult model
# ---------------------------------------------------------------------------


class TestCheckResult:
    def test_passed_result(self):
        r = CheckResult(name="test", passed=True, detail="ok")
        assert r.passed is True
        assert r.fix is None

    def test_failed_result_with_fix(self):
        r = CheckResult(name="test", passed=False, detail="missing", fix="install it")
        assert r.passed is False
        assert r.fix == "install it"

    def test_str_passed(self):
        r = CheckResult(name="cargo", passed=True, detail="cargo 1.82.0")
        s = str(r)
        assert "PASS" in s
        assert "cargo" in s

    def test_str_failed(self):
        r = CheckResult(name="docker", passed=False, detail="not found", fix="brew install docker")
        s = str(r)
        assert "FAIL" in s
        assert "fix:" in s.lower() or "brew install" in s


# ---------------------------------------------------------------------------
# Container runtime check
# ---------------------------------------------------------------------------


class TestCheckContainerRuntime:
    @patch("capsem.builder.doctor.shutil.which")
    @patch("capsem.builder.doctor.subprocess.run")
    def test_docker_found(self, mock_run, mock_which):
        mock_which.side_effect = lambda name: "/usr/local/bin/docker" if name == "docker" else None
        mock_run.return_value = MagicMock(
            stdout="Docker version 27.1.1\n", returncode=0
        )
        result = check_container_runtime()
        assert result.passed is True
        assert "docker" in result.detail.lower()

    @patch("capsem.builder.doctor.shutil.which")
    def test_not_found(self, mock_which):
        mock_which.return_value = None
        result = check_container_runtime()
        assert result.passed is False
        assert result.fix is not None
        assert "docker" in result.fix.lower()


# ---------------------------------------------------------------------------
# Rust toolchain check
# ---------------------------------------------------------------------------


class TestCheckRustToolchain:
    @patch("capsem.builder.doctor.shutil.which")
    @patch("capsem.builder.doctor.subprocess.run")
    def test_all_present(self, mock_run, mock_which):
        mock_which.side_effect = lambda name: f"/usr/local/bin/{name}"
        mock_run.return_value = MagicMock(stdout="rustup 1.27.1\n", returncode=0)
        result = check_rust_toolchain()
        assert result.passed is True

    @patch("capsem.builder.doctor.shutil.which")
    def test_rustup_missing(self, mock_which):
        mock_which.side_effect = lambda name: None if name == "rustup" else f"/usr/local/bin/{name}"
        result = check_rust_toolchain()
        assert result.passed is False
        assert "rustup" in result.detail.lower()
        assert result.fix is not None

    @patch("capsem.builder.doctor.shutil.which")
    def test_cargo_missing(self, mock_which):
        mock_which.side_effect = lambda name: None if name == "cargo" else f"/usr/local/bin/{name}"
        result = check_rust_toolchain()
        assert result.passed is False
        assert "cargo" in result.detail.lower()


# ---------------------------------------------------------------------------
# Cross-compilation target check
# ---------------------------------------------------------------------------


class TestCheckCrossTarget:
    @patch("capsem.builder.doctor.subprocess.run")
    def test_target_installed(self, mock_run):
        mock_run.return_value = MagicMock(
            stdout="aarch64-unknown-linux-musl\nx86_64-unknown-linux-musl\n",
            returncode=0,
        )
        result = check_cross_target("aarch64-unknown-linux-musl")
        assert result.passed is True

    @patch("capsem.builder.doctor.subprocess.run")
    def test_target_not_installed(self, mock_run):
        mock_run.return_value = MagicMock(
            stdout="aarch64-apple-darwin\n",
            returncode=0,
        )
        result = check_cross_target("x86_64-unknown-linux-musl")
        assert result.passed is False
        assert "rustup target add" in result.fix

    @patch("capsem.builder.doctor.subprocess.run")
    def test_rustup_not_available(self, mock_run):
        mock_run.side_effect = FileNotFoundError("rustup not found")
        result = check_cross_target("aarch64-unknown-linux-musl")
        assert result.passed is False


# ---------------------------------------------------------------------------
# b3sum check
# ---------------------------------------------------------------------------


class TestCheckB3sum:
    @patch("capsem.builder.doctor.shutil.which")
    @patch("capsem.builder.doctor.subprocess.run")
    def test_found(self, mock_run, mock_which):
        mock_which.return_value = "/usr/local/bin/b3sum"
        mock_run.return_value = MagicMock(stdout="b3sum 1.5.4\n", returncode=0)
        result = check_b3sum()
        assert result.passed is True

    @patch("capsem.builder.doctor.shutil.which")
    def test_missing(self, mock_which):
        mock_which.return_value = None
        result = check_b3sum()
        assert result.passed is False
        assert "cargo install b3sum" in result.fix


# ---------------------------------------------------------------------------
# Container clock check
# ---------------------------------------------------------------------------


class TestCheckContainerClock:
    @patch("capsem.builder.doctor.sys")
    def test_returns_none_on_linux(self, mock_sys):
        mock_sys.platform = "linux"
        assert check_container_clock() is None

    @patch("capsem.builder.doctor.shutil.which")
    @patch("capsem.builder.doctor.subprocess.run")
    @patch("capsem.builder.doctor.sys")
    def test_passes_when_clock_in_sync(self, mock_sys, mock_run, mock_which):
        mock_sys.platform = "darwin"
        mock_which.side_effect = lambda name: "/usr/local/bin/docker" if name == "docker" else None
        import time
        mock_run.return_value = MagicMock(
            stdout=str(int(time.time())), returncode=0,
        )
        result = check_container_clock()
        assert result is not None
        assert result.passed is True
        assert "ok" in result.detail

    @patch("capsem.builder.doctor.shutil.which")
    @patch("capsem.builder.doctor.subprocess.run")
    @patch("capsem.builder.doctor.sys")
    def test_fails_when_clock_behind(self, mock_sys, mock_run, mock_which):
        mock_sys.platform = "darwin"
        mock_which.side_effect = lambda name: "/usr/local/bin/docker" if name == "docker" else None
        import time
        mock_run.return_value = MagicMock(
            stdout=str(int(time.time()) - 300), returncode=0,
        )
        result = check_container_clock()
        assert result is not None
        assert result.passed is False
        assert "behind" in result.detail

    @patch("capsem.builder.doctor.shutil.which")
    @patch("capsem.builder.doctor.subprocess.run")
    @patch("capsem.builder.doctor.sys")
    def test_fails_when_clock_ahead(self, mock_sys, mock_run, mock_which):
        mock_sys.platform = "darwin"
        mock_which.side_effect = lambda name: "/usr/local/bin/docker" if name == "docker" else None
        import time
        mock_run.return_value = MagicMock(
            stdout=str(int(time.time()) + 300), returncode=0,
        )
        result = check_container_clock()
        assert result is not None
        assert result.passed is False
        assert "ahead" in result.detail

    @patch("capsem.builder.doctor.shutil.which")
    @patch("capsem.builder.doctor.sys")
    def test_returns_none_when_no_docker(self, mock_sys, mock_which):
        mock_sys.platform = "darwin"
        mock_which.return_value = None
        assert check_container_clock() is None

    @patch("capsem.builder.doctor.shutil.which")
    @patch("capsem.builder.doctor.subprocess.run")
    @patch("capsem.builder.doctor.sys")
    def test_returns_none_on_command_failure(self, mock_sys, mock_run, mock_which):
        mock_sys.platform = "darwin"
        mock_which.side_effect = lambda name: "/usr/local/bin/docker" if name == "docker" else None
        mock_run.return_value = MagicMock(stdout="", returncode=1)
        assert check_container_clock() is None


# ---------------------------------------------------------------------------
# Guest config check
# ---------------------------------------------------------------------------


class TestCheckGuestConfig:
    def test_valid_config(self, tmp_path):
        config = tmp_path / "config"
        config.mkdir()
        (config / "build.toml").write_text(
            '[build]\ncompression = "zstd"\ncompression_level = 15\n'
            "[build.architectures.arm64]\n"
            'base_image = "debian:bookworm-slim"\n'
            'docker_platform = "linux/arm64"\n'
            'rust_target = "aarch64-unknown-linux-musl"\n'
            'kernel_branch = "6.6"\n'
            'kernel_image = "arch/arm64/boot/Image"\n'
            'defconfig = "kernel/defconfig.arm64"\n'
            "node_major = 24\n"
        )
        result = check_guest_config(tmp_path)
        assert result.passed is True
        assert "1 architecture" in result.detail

    def test_missing_build_toml(self, tmp_path):
        config = tmp_path / "config"
        config.mkdir()
        result = check_guest_config(tmp_path)
        assert result.passed is False
        assert "build.toml" in result.detail

    def test_no_config_dir(self, tmp_path):
        result = check_guest_config(tmp_path)
        assert result.passed is False

    def test_invalid_toml(self, tmp_path):
        config = tmp_path / "config"
        config.mkdir()
        (config / "build.toml").write_text("invalid [[ toml")
        result = check_guest_config(tmp_path)
        assert result.passed is False


# ---------------------------------------------------------------------------
# Source files check
# ---------------------------------------------------------------------------


def _create_all_source_files(tmp_path, *, skip=None):
    """Create all required source files for check_source_files(), optionally
    skipping one by name so tests can verify detection of that missing file."""
    from capsem.builder.docker import (
        ROOTFS_SCRIPTS,
        ROOTFS_SCRIPT_DIRS,
        ROOTFS_SUPPORT_FILES,
    )
    artifacts = tmp_path / "guest" / "artifacts"
    artifacts.mkdir(parents=True, exist_ok=True)
    config = tmp_path / "config"
    config.mkdir(exist_ok=True)
    # Individual files
    all_files = ["capsem-init"] + list(ROOTFS_SUPPORT_FILES) + list(ROOTFS_SCRIPTS)
    for name in all_files:
        if name != skip:
            (artifacts / name).write_text("stub")
    # Directories
    for name in ROOTFS_SCRIPT_DIRS:
        if name != skip:
            (artifacts / name).mkdir(exist_ok=True)
    # capsem_bench needs __main__.py
    bench_pkg = artifacts / "capsem_bench"
    if bench_pkg.is_dir():
        (bench_pkg / "__main__.py").write_text("stub")
    # CA cert
    if skip != "capsem-ca.crt":
        (config / "capsem-ca.crt").write_text("stub cert")


class TestCheckSourceFiles:
    def test_all_present(self, tmp_path):
        artifacts = tmp_path / "guest" / "artifacts"
        artifacts.mkdir(parents=True)
        config = tmp_path / "config"
        config.mkdir()
        # Create all required files
        for name in [
            "capsem-init", "capsem-bashrc", "banner.txt", "tips.txt",
            "capsem-doctor", "capsem-bench", "snapshots",
        ]:
            (artifacts / name).write_text("stub")
        (artifacts / "diagnostics").mkdir()
        bench_pkg = artifacts / "capsem_bench"
        bench_pkg.mkdir()
        (bench_pkg / "__main__.py").write_text("stub")
        (config / "capsem-ca.crt").write_text("stub cert")
        result = check_source_files(tmp_path)
        assert result.passed is True

    def test_missing_capsem_init(self, tmp_path):
        artifacts = tmp_path / "guest" / "artifacts"
        artifacts.mkdir(parents=True)
        config = tmp_path / "config"
        config.mkdir()
        for name in [
            "capsem-bashrc", "banner.txt", "tips.txt",
            "capsem-doctor", "capsem-bench", "snapshots",
        ]:
            (artifacts / name).write_text("stub")
        (artifacts / "diagnostics").mkdir()
        bench_pkg = artifacts / "capsem_bench"
        bench_pkg.mkdir()
        (bench_pkg / "__main__.py").write_text("stub")
        (config / "capsem-ca.crt").write_text("stub cert")
        result = check_source_files(tmp_path)
        assert result.passed is False
        assert "capsem-init" in result.detail

    def test_missing_snapshots(self, tmp_path):
        artifacts = tmp_path / "guest" / "artifacts"
        artifacts.mkdir(parents=True)
        config = tmp_path / "config"
        config.mkdir()
        for name in [
            "capsem-init", "capsem-bashrc", "banner.txt", "tips.txt",
            "capsem-doctor", "capsem-bench",
        ]:
            (artifacts / name).write_text("stub")
        (artifacts / "diagnostics").mkdir()
        bench_pkg = artifacts / "capsem_bench"
        bench_pkg.mkdir()
        (bench_pkg / "__main__.py").write_text("stub")
        (config / "capsem-ca.crt").write_text("stub cert")
        result = check_source_files(tmp_path)
        assert result.passed is False
        assert "snapshots" in result.detail

    def test_missing_diagnostics_dir(self, tmp_path):
        artifacts = tmp_path / "guest" / "artifacts"
        artifacts.mkdir(parents=True)
        config = tmp_path / "config"
        config.mkdir()
        for name in [
            "capsem-init", "capsem-bashrc", "banner.txt", "tips.txt",
            "capsem-doctor", "capsem-bench", "snapshots",
        ]:
            (artifacts / name).write_text("stub")
        # No diagnostics/ dir
        bench_pkg = artifacts / "capsem_bench"
        bench_pkg.mkdir()
        (bench_pkg / "__main__.py").write_text("stub")
        (config / "capsem-ca.crt").write_text("stub cert")
        result = check_source_files(tmp_path)
        assert result.passed is False
        assert "diagnostics" in result.detail

    def test_missing_bench_pkg_dir(self, tmp_path):
        artifacts = tmp_path / "guest" / "artifacts"
        artifacts.mkdir(parents=True)
        config = tmp_path / "config"
        config.mkdir()
        for name in [
            "capsem-init", "capsem-bashrc", "banner.txt", "tips.txt",
            "capsem-doctor", "capsem-bench", "snapshots",
        ]:
            (artifacts / name).write_text("stub")
        (artifacts / "diagnostics").mkdir()
        # No capsem_bench/ dir
        (config / "capsem-ca.crt").write_text("stub cert")
        result = check_source_files(tmp_path)
        assert result.passed is False
        assert "capsem_bench" in result.detail

    def test_all_missing_reports_all_names(self, tmp_path):
        artifacts = tmp_path / "guest" / "artifacts"
        artifacts.mkdir(parents=True)
        # Nothing created -- all required files missing
        result = check_source_files(tmp_path)
        assert result.passed is False
        for name in ["capsem-init", "capsem-bashrc", "banner.txt", "tips.txt",
                      "capsem-doctor", "capsem-bench", "snapshots",
                      "diagnostics", "capsem_bench", "capsem-ca.crt"]:
            assert name in result.detail, f"{name} not reported as missing"

    def test_missing_ca_cert(self, tmp_path):
        artifacts = tmp_path / "guest" / "artifacts"
        artifacts.mkdir(parents=True)
        for name in [
            "capsem-init", "capsem-bashrc", "banner.txt", "tips.txt",
            "capsem-doctor", "capsem-bench", "snapshots",
        ]:
            (artifacts / name).write_text("stub")
        (artifacts / "diagnostics").mkdir()
        bench_pkg = artifacts / "capsem_bench"
        bench_pkg.mkdir()
        (bench_pkg / "__main__.py").write_text("stub")
        # No config/capsem-ca.crt
        result = check_source_files(tmp_path)
        assert result.passed is False
        assert "capsem-ca.crt" in result.detail

    def test_missing_snapshots(self, tmp_path):
        _create_all_source_files(tmp_path, skip="snapshots")
        result = check_source_files(tmp_path)
        assert result.passed is False
        assert "snapshots" in result.detail


# ---------------------------------------------------------------------------
# run_all_checks
# ---------------------------------------------------------------------------


class TestRunAllChecks:
    @patch("capsem.builder.doctor.check_container_runtime")
    @patch("capsem.builder.doctor.check_container_resources")
    @patch("capsem.builder.doctor.check_container_clock")
    @patch("capsem.builder.doctor.check_rust_toolchain")
    @patch("capsem.builder.doctor.check_cross_target")
    @patch("capsem.builder.doctor.check_b3sum")
    @patch("capsem.builder.doctor.check_guest_config")
    @patch("capsem.builder.doctor.check_source_files")
    def test_composes_all(
        self, mock_src, mock_guest, mock_b3, mock_cross, mock_rust,
        mock_clock, mock_resources, mock_runtime,
    ):
        for mock in [mock_src, mock_guest, mock_b3, mock_rust, mock_runtime]:
            mock.return_value = CheckResult(name="x", passed=True, detail="ok")
        mock_cross.return_value = CheckResult(name="x", passed=True, detail="ok")
        mock_resources.return_value = None
        mock_clock.return_value = None
        results = run_all_checks(Path("guest"), Path("."))
        # At minimum: runtime + rust + arm64 target + x86_64 target + b3sum + config + source
        assert len(results) >= 7
        assert all(r.passed for r in results)

    @patch("capsem.builder.doctor.check_container_runtime")
    @patch("capsem.builder.doctor.check_container_resources")
    @patch("capsem.builder.doctor.check_container_clock")
    @patch("capsem.builder.doctor.check_rust_toolchain")
    @patch("capsem.builder.doctor.check_cross_target")
    @patch("capsem.builder.doctor.check_b3sum")
    @patch("capsem.builder.doctor.check_guest_config")
    @patch("capsem.builder.doctor.check_source_files")
    def test_counts_failures(
        self, mock_src, mock_guest, mock_b3, mock_cross, mock_rust,
        mock_clock, mock_resources, mock_runtime,
    ):
        mock_runtime.return_value = CheckResult(
            name="container-runtime", passed=False, detail="missing", fix="install"
        )
        for mock in [mock_src, mock_guest, mock_b3, mock_rust]:
            mock.return_value = CheckResult(name="x", passed=True, detail="ok")
        mock_cross.return_value = CheckResult(name="x", passed=True, detail="ok")
        mock_resources.return_value = None
        mock_clock.return_value = None
        results = run_all_checks(Path("guest"), Path("."))
        failures = [r for r in results if not r.passed]
        assert len(failures) >= 1


# ---------------------------------------------------------------------------
# Output formatting
# ---------------------------------------------------------------------------


class TestFormatResults:
    def test_all_pass(self):
        results = [
            CheckResult(name="docker", passed=True, detail="docker 27.1.1"),
            CheckResult(name="cargo", passed=True, detail="cargo 1.82.0"),
        ]
        output = format_results(results)
        assert "PASS" in output
        assert "2 passed" in output
        assert "0 failed" in output

    def test_with_failures(self):
        results = [
            CheckResult(name="docker", passed=True, detail="docker 27.1.1"),
            CheckResult(
                name="b3sum", passed=False, detail="not found",
                fix="cargo install b3sum",
            ),
        ]
        output = format_results(results)
        assert "FAIL" in output
        assert "1 passed" in output
        assert "1 failed" in output
        assert "cargo install b3sum" in output
