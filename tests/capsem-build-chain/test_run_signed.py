"""Build runner contract tests."""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]


def test_run_signed_serializes_codesign_without_flock() -> None:
    script = (PROJECT_ROOT / "build_system" / "packaging" / "macos" / "run_signed.sh").read_text()

    assert "SIGN_LOCK_DIR=" in script
    assert "acquire_sign_lock" in script
    assert "release_sign_lock" in script
    assert "mkdir \"$SIGN_LOCK_DIR\"" in script
    assert "flock" not in script


def test_run_signed_materializes_its_cache_leaves(tmp_path: Path) -> None:
    package_dir = tmp_path / "build_system" / "packaging" / "macos"
    package_dir.mkdir(parents=True)
    source = PROJECT_ROOT / "build_system" / "packaging" / "macos"
    shutil.copy(source / "run_signed.sh", package_dir)

    result = subprocess.run(
        ("bash", str(package_dir / "run_signed.sh")),
        check=False,
        capture_output=True,
        text=True,
    )

    assert result.returncode == 1
    assert "codesign requires macOS" in result.stderr
    assert (tmp_path / "cache" / "target").is_dir()
    assert "codesign requires macOS" in (
        tmp_path / "cache" / "containers" / "logs" / "build.log"
    ).read_text()
