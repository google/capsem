"""Verify just doctor recipe runs without errors."""

import subprocess

import pytest

from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent.parent

pytestmark = pytest.mark.recipe


def test_just_doctor():
    """just doctor runs and exits cleanly."""
    result = subprocess.run(
        ["just", "doctor"],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=60,
    )
    # Doctor may warn but should not crash
    assert result.returncode == 0, (
        f"just doctor failed (exit {result.returncode}):\n{result.stderr}"
    )
