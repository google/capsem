"""Verify cargo build --workspace succeeds and expected binaries exist."""

import subprocess
from pathlib import Path

import pytest
from helpers.bounded import bounded

PROJECT_ROOT = Path(__file__).parent.parent.parent

pytestmark = pytest.mark.recipe


def test_cargo_build_workspace():
    """cargo build --workspace succeeds."""
    result = subprocess.run(
        bounded(["cargo", "build", "--workspace"], 300),
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, f"cargo build failed:\n{result.stderr}"


def test_expected_binaries_after_build():
    """After cargo build, all expected binaries exist."""
    expected = [
        "capsem-service",
        "capsem-process",
        "capsem",

    ]
    target_dir = PROJECT_ROOT / "cache" / "target" / "cargo" / "debug"
    for name in expected:
        binary = target_dir / name
        assert binary.exists(), f"Expected binary not found: {binary}"
        assert binary.stat().st_size > 0, f"Binary is empty: {binary}"
