"""Verify cargo build produces all expected daemon binaries."""

import subprocess
import tomllib
from pathlib import Path

import pytest
from helpers.bounded import bounded

pytestmark = pytest.mark.build_chain

PROJECT_ROOT = Path(__file__).parent.parent.parent


def test_all_binaries_exist(built_binaries):
    """All 4 daemon crate binaries exist in cache/target/cargo/debug/."""
    for name, path in built_binaries.items():
        assert path.exists(), f"{name} not found at {path}"


def test_binaries_are_executable(built_binaries):
    """Built binaries have executable permissions."""
    import stat

    for name, path in built_binaries.items():
        if not path.exists():
            continue
        mode = path.stat().st_mode
        assert mode & stat.S_IXUSR, f"{name} is not executable"


def test_binaries_nonzero_size(built_binaries):
    """Built binaries are not empty files."""
    for name, path in built_binaries.items():
        if not path.exists():
            continue
        assert path.stat().st_size > 0, f"{name} is empty"


@pytest.mark.parametrize("flag", ["--version", "--help"])
def test_process_diagnostics_without_worker_grants(built_binaries, flag):
    """Package diagnostics work with ordinary stdin, before worker admission."""
    result = subprocess.run(
        bounded([str(built_binaries["capsem-process"]), flag], 10),
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    if flag == "--version":
        version = tomllib.loads((PROJECT_ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
        assert result.stdout.strip() == f"capsem-process {version}"
    else:
        assert "Usage:" in result.stdout
        assert "--parent-pid" in result.stdout


def test_process_start_requires_worker_grants(built_binaries, tmp_path):
    """Moving diagnostic parsing must not admit ordinary standalone startup."""
    session = tmp_path / "session"
    result = subprocess.run(
        bounded([
            str(built_binaries["capsem-process"]),
            "--id", "unadmitted",
            "--assets-dir", str(tmp_path),
            "--rootfs", str(tmp_path / "rootfs.erofs"),
            "--expected-kernel-hash", "0" * 64,
            "--expected-initrd-hash", "0" * 64,
            "--expected-rootfs-hash", "0" * 64,
            "--session-dir", str(session),
            "--active-policy", str(tmp_path / "policy.toml"),
            "--uds-path", str(tmp_path / "ipc.sock"),
        ], 10),
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode != 0
    assert "adopt inherited upstream grant channel" in result.stderr
    assert not session.exists()
    assert not (tmp_path / "ipc.sock").exists()


def test_workspace_has_no_warnings():
    """All workspace crates must compile with zero warnings.

    Compiler warnings have historically masked broken code (unused imports,
    dead fields, unconstructed response types) that caused runtime failures
    while unit tests passed. Workspace lints deny warnings via Cargo.toml.
    """
    result = subprocess.run(
        bounded(["cargo", "check", "--workspace"], 300),
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, (
        "Workspace crates have compiler warnings (treated as errors):\n"
        f"{result.stderr}"
    )
