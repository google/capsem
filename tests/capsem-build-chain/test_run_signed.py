"""Build runner contract tests."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]


def test_run_signed_serializes_codesign_without_flock() -> None:
    script = (PROJECT_ROOT / "build_system" / "packaging" / "macos" / "run_signed.sh").read_text()

    assert "SIGN_LOCK_DIR=" in script
    assert "acquire_sign_lock" in script
    assert "release_sign_lock" in script
    assert "mkdir \"$SIGN_LOCK_DIR\"" in script
    assert "flock" not in script


@pytest.mark.parametrize("platform", ["Linux", "Darwin"])
def test_run_signed_materializes_its_cache_leaves(tmp_path: Path, platform: str) -> None:
    package_dir = tmp_path / "build_system" / "packaging" / "macos"
    package_dir.mkdir(parents=True)
    source = PROJECT_ROOT / "build_system" / "packaging" / "macos"
    shutil.copy(source / "run_signed.sh", package_dir)

    # Exercise both host branches without codesigning a real developer binary.
    # The copied runner deliberately has no entitlements beside it.
    result = subprocess.run(
        (
            "bash", "-c",
            'uname() { echo "$TEST_PLATFORM"; }; export -f uname; exec bash "$@"',
            "bash", str(package_dir / "run_signed.sh"), sys.executable,
        ),
        env={**os.environ, "TEST_PLATFORM": platform},
        check=False,
        capture_output=True,
        text=True,
    )

    assert result.returncode == 1
    expected = "codesign requires macOS" if platform == "Linux" else f"not found at {package_dir}/"
    assert expected in result.stderr
    assert (tmp_path / "cache" / "target").is_dir()
    assert expected in (
        tmp_path / "cache" / "containers" / "logs" / "build.log"
    ).read_text()
