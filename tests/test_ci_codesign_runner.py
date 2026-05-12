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


def test_pr_install_e2e_sets_up_asset_build_prerequisites():
    """PR install E2E builds missing VM assets from a clean checkout."""
    workflow = (REPO_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    install_job = workflow.split("  test-install:\n", 1)[1]

    assert "pnpm/action-setup@v5" in install_job
    assert "actions/setup-node@v5" in install_job
    assert "node-version: 24" in install_job
    assert "cache-dependency-path: frontend/pnpm-lock.yaml" in install_job
    assert "astral-sh/setup-uv@v5" in install_job
    assert "uv sync" in install_job
    assert "b3sum minisign" in install_job
    assert install_job.index("pnpm/action-setup@v5") < install_job.index("just test-install")
    assert install_job.index("actions/setup-node@v5") < install_job.index("just test-install")
    assert install_job.index("uv sync") < install_job.index("just test-install")
    assert install_job.index("b3sum minisign") < install_job.index("just test-install")
