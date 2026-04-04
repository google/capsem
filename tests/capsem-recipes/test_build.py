"""Verify cargo build --workspace succeeds and expected binaries exist."""

import subprocess

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent

pytestmark = pytest.mark.recipe


def test_cargo_build_workspace():
    """cargo build --workspace succeeds."""
    result = subprocess.run(
        ["cargo", "build", "--workspace"],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=300,
    )
    assert result.returncode == 0, f"cargo build failed:\n{result.stderr}"


def test_expected_binaries_after_build():
    """After cargo build, all expected binaries exist."""
    expected = [
        "capsem-service",
        "capsem-process",
        "capsem",
        "capsem-mcp",
    ]
    target_dir = PROJECT_ROOT / "target" / "debug"
    for name in expected:
        binary = target_dir / name
        assert binary.exists(), f"Expected binary not found: {binary}"
        assert binary.stat().st_size > 0, f"Binary is empty: {binary}"
