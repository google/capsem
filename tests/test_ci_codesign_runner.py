"""Regression checks for the macOS cargo runner used by CI."""

from __future__ import annotations

import re
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


def test_ci_rust_coverage_floor_matches_just_test_gate():
    """CI should not drift from the local full-test coverage floor."""
    workflow = (REPO_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    justfile = (REPO_ROOT / "justfile").read_text()

    just_match = re.search(
        r"cargo llvm-cov --workspace --no-cfg-coverage --fail-under-lines (\d+)",
        justfile,
    )
    assert just_match, "just test Rust coverage floor missing"

    ci_thresholds = set(
        re.findall(r"cargo llvm-cov nextest [^\n]*--fail-under-lines (\d+)", workflow)
    )
    assert ci_thresholds, "CI Rust coverage floors missing"
    assert ci_thresholds == {just_match.group(1)}


def test_ci_python_schema_step_does_not_collect_vm_suites():
    """The schema/coverage step must not accidentally boot VM integration suites."""
    workflow = (REPO_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    section = workflow.split("      - name: Python schema tests with coverage\n", 1)[1]
    section = section.split("\n      # Python integration tests that need no VM", 1)[0]

    assert "tests/test_*.py" in section
    assert "python -m pytest tests/ --cov" not in section
    assert "tests/capsem-" not in section
