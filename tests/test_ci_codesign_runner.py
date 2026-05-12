"""Regression checks for the macOS cargo runner used by CI."""

from __future__ import annotations

from pathlib import Path


REPO_ROOT = Path(__file__).parent.parent


def test_run_signed_serializes_codesign_and_skips_valid_signatures():
    """Nextest discovery can invoke the macOS runner concurrently."""
    script = (REPO_ROOT / "scripts" / "run_signed.sh").read_text()

    assert "SIGN_LOCK_DIR" in script
    assert "acquire_codesign_lock" in script
    assert "release_codesign_lock" in script
    assert 'mkdir "$SIGN_LOCK_DIR"' in script
    assert "signed_with_entitlements" in script
    assert 'codesign --verify "$binary"' in script
    assert "com.apple.security.virtualization" in script


def test_ci_failure_artifacts_include_codesign_runner_log():
    """When runner signing fails, CI must preserve target/build.log."""
    workflow = (REPO_ROOT / ".github" / "workflows" / "ci.yaml").read_text()

    assert "target/build.log" in workflow
