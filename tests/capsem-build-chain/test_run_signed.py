"""Build runner contract tests."""

from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]


def test_run_signed_serializes_codesign_without_flock() -> None:
    script = (PROJECT_ROOT / "scripts" / "run_signed.sh").read_text()

    assert "SIGN_LOCK_DIR=" in script
    assert "acquire_sign_lock" in script
    assert "release_sign_lock" in script
    assert "mkdir \"$SIGN_LOCK_DIR\"" in script
    assert "flock" not in script
