"""Verify cargo build produces all expected daemon binaries."""

import subprocess
from pathlib import Path

import pytest

pytestmark = pytest.mark.build_chain

PROJECT_ROOT = Path(__file__).parent.parent.parent


def test_all_binaries_exist(built_binaries):
    """All 4 daemon crate binaries exist in target/debug/."""
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


def test_workspace_has_no_warnings():
    """All workspace crates must compile with zero warnings.

    Compiler warnings have historically masked broken code (unused imports,
    dead fields, unconstructed response types) that caused runtime failures
    while unit tests passed. Workspace lints deny warnings via Cargo.toml.
    """
    result = subprocess.run(
        ["cargo", "check", "--workspace"],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=300,
    )
    assert result.returncode == 0, (
        "Workspace crates have compiler warnings (treated as errors):\n"
        f"{result.stderr}"
    )
