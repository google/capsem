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
    assert "dtolnay/rust-toolchain@stable" in install_job
    assert "Normalize cargo proxy" in install_job
    assert "bash scripts/build-assets.sh --assets-dir assets --arch arm64" in install_job
    assert install_job.index("pnpm/action-setup@v5") < install_job.index("just test-install")
    assert install_job.index("actions/setup-node@v5") < install_job.index("just test-install")
    assert install_job.index("uv sync") < install_job.index("just test-install")
    assert install_job.index("b3sum minisign") < install_job.index("just test-install")
    assert install_job.index("bash scripts/build-assets.sh --assets-dir assets --arch arm64") < install_job.index("just test-install")


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
    assert "--cov-fail-under=89" in section
    assert "--cov-fail-under=90" not in section


def test_pr_non_vm_integration_lane_has_no_generated_asset_prereqs():
    """Clean PR runners must not execute suites that require built assets/signing."""
    workflow = (REPO_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    section = workflow.split(
        "      - name: Python integration tests (non-VM suites)\n",
        1,
    )[1]
    section = section.split(
        "\n      # Verify all integration test suites import cleanly",
        1,
    )[0]
    collect_section = workflow.split(
        "      - name: Verify all integration test imports\n",
        1,
    )[1]
    collect_section = collect_section.split("\n      # Schema drift check", 1)[0]

    assert "tests/capsem-rootfs-artifacts/" in section
    assert "tests/capsem-bootstrap/" not in section
    assert "tests/capsem-codesign/" not in section
    assert "tests/capsem-*/ --collect-only" in collect_section


def test_pr_linux_ci_compiles_kvm_without_exercising_hosted_kvm():
    """Hosted ARM KVM can hang; PR CI must stay compile/no-run."""
    workflow = (REPO_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    linux_job = workflow.split("  test-linux:\n", 1)[1]
    linux_job = linux_job.split("\n  # ---------------------------------------------------------------------------", 1)[0]
    kvm_sys = (
        REPO_ROOT
        / "crates"
        / "capsem-core"
        / "src"
        / "hypervisor"
        / "kvm"
        / "sys.rs"
    ).read_text()

    assert 'CAPSEM_SKIP_KVM_TESTS: "1"' in linux_job
    assert "release pipeline owns real-KVM coverage" in linux_job
    assert "Compile tests (KVM backend, no live KVM)" in linux_job
    assert "timeout-minutes: 15" in linux_job
    assert "cargo test --no-run --all-targets" in linux_job
    assert "cargo llvm-cov nextest" not in linux_job
    assert "codecov-linux.json" not in linux_job
    assert "-p capsem-core" in linux_job
    assert 'std::env::var_os("CAPSEM_SKIP_KVM_TESTS")' in kvm_sys


def test_pr_linux_kvm_diagnostics_do_not_emit_red_success_annotations():
    """Diagnostic-only KVM setup must not rely on continue-on-error."""
    workflow = (REPO_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    linux_job = workflow.split("  test-linux:\n", 1)[1]
    linux_job = linux_job.split("\n  # ---------------------------------------------------------------------------", 1)[0]

    assert "continue-on-error: true" not in linux_job
    assert "Enable KVM (best-effort)" not in linux_job
    assert "Collect KVM diagnostics" in linux_job


def test_ci_rust_integration_coverage_is_release_blocking():
    """Rust integration coverage must fail CI when the tests fail."""
    workflow = (REPO_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    section = workflow.split("      - name: Integration tests with coverage\n", 1)[1]
    section = section.split("\n      # Frontend tests with coverage", 1)[0]

    assert "cargo llvm-cov nextest" in section
    assert "|| true" not in section


def test_ci_coverage_summary_report_errors_are_not_hidden_by_tee():
    """The coverage summary command must be compatible and pipefail-protected."""
    workflow = (REPO_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    section = workflow.split("      - name: Unit tests with coverage\n", 1)[1]
    section = section.split("\n      # Integration tests", 1)[0]

    assert "set -o pipefail" in section
    assert "cargo llvm-cov report --summary-only" in section
    assert "cargo llvm-cov report --no-cfg-coverage" not in section


def test_ci_uses_supported_codecov_test_results_upload():
    """Codecov test analytics should use codecov-action, not deprecated action."""
    workflow = (REPO_ROOT / ".github" / "workflows" / "ci.yaml").read_text()
    section = workflow.split("      - name: Upload test results to Codecov\n", 1)[1]
    section = section.split("\n      # T5: preserve every test artifact", 1)[0]

    assert "codecov/test-results-action" not in workflow
    assert "uses: codecov/codecov-action@v5" in section
    assert "report_type: test_results" in section


def test_workflows_opt_into_node24_action_runtime():
    """Avoid late Node 20 action-runtime surprises across all workflows."""
    for workflow in sorted((REPO_ROOT / ".github" / "workflows").glob("*.yaml")):
        text = workflow.read_text()
        assert "FORCE_JAVASCRIPT_ACTIONS_TO_NODE24: true" in text, workflow
