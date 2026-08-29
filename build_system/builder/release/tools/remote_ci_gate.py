"""Fail-closed behavioral contract for the remote branch-protection gate."""

from __future__ import annotations

import os
import re
import subprocess

WORKFLOW_PATH = ".github/workflows/ci.yaml"
GATE_SCRIPT_PATH = "build_system/scripts/ci/require-ci-jobs.sh"
REQUIRED_PR_GATE_JOBS = (
    "scope",
    "fast-gate",
    "test-linux",
    "test",
    "test-install",
    "docs-build",
    "site-build",
    "release-site-build",
)
REQUIRED_PR_GATE_RESULT_CHECKS = (
    ("scope", "SCOPE_RESULT"),
    ("fast-gate", "FAST_GATE_RESULT"),
    ("test-linux", "TEST_LINUX_RESULT"),
    ("test", "TEST_MACOS_RESULT"),
    ("test-install", "TEST_INSTALL_RESULT"),
    ("docs-build", "DOCS_BUILD_RESULT"),
    ("site-build", "SITE_BUILD_RESULT"),
    ("release-site-build", "RELEASE_SITE_BUILD_RESULT"),
)
INDEPENDENT_RESULT_CHECKS = (
    ("test-linux", "TEST_LINUX_RESULT", "TEST_LINUX_REQUIRED"),
    ("test", "TEST_MACOS_RESULT", "TEST_MACOS_REQUIRED"),
    ("test-install", "TEST_INSTALL_RESULT", "TEST_INSTALL_REQUIRED"),
    ("docs-build", "DOCS_BUILD_RESULT", "DOCS_BUILD_REQUIRED"),
    ("site-build", "SITE_BUILD_RESULT", "SITE_BUILD_REQUIRED"),
    ("release-site-build", "RELEASE_SITE_BUILD_RESULT", "RELEASE_SITE_BUILD_REQUIRED"),
)
NON_SUCCESS_RESULTS = ("failure", "cancelled", "skipped")


def workflow_job_block(workflow: str, name: str) -> str:
    lines = workflow.splitlines()
    start = next((i for i, line in enumerate(lines) if line == f"  {name}:"), None)
    if start is None:
        return ""
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line.startswith("  ") and not line.startswith("    ") and line.endswith(":"):
            end = i
            break
    return "\n".join(lines[start:end])


def workflow_job_needs(job_block: str) -> set[str]:
    inline = re.search(r"(?m)^\s+needs:\s*\[([^\]]+)\]\s*$", job_block)
    if inline:
        return {part.strip() for part in inline.group(1).split(",") if part.strip()}

    needs: set[str] = set()
    lines = job_block.splitlines()
    for i, line in enumerate(lines):
        if re.match(r"^\s+needs:\s*$", line):
            for item in lines[i + 1 :]:
                if not item.startswith("      - "):
                    break
                needs.add(item.removeprefix("      - ").strip())
            break
    return needs


def pr_gate_contract_failures(job_block: str, gate_script: str) -> list[str]:
    failures: list[str] = []
    missing = sorted(set(REQUIRED_PR_GATE_JOBS) - workflow_job_needs(job_block))
    if missing:
        failures.append("does not aggregate required jobs: " + ", ".join(missing))
    if not re.search(r"(?m)^\s+if:\s*\$\{\{\s*always\(\)\s*\}}\s*$", job_block):
        failures.append("pr-gate does not run with if: ${{ always() }}")
    if not re.search(
        rf"(?m)^\s+run:\s+bash\s+{re.escape(GATE_SCRIPT_PATH)}\s*$", job_block
    ):
        failures.append(f"pr-gate does not dispatch bare {GATE_SCRIPT_PATH}")
    if re.search(
        r"(?m)^\s+(?:-\s+)?continue-on-error:\s*(?:true|\$\{\{\s*true\s*\}\})\s*$",
        job_block,
    ):
        failures.append("pr-gate verdict step is optional")
    for job, env_name in REQUIRED_PR_GATE_RESULT_CHECKS:
        if f"needs.{job}.result" not in job_block:
            failures.append(f"pr-gate does not bind {job} result to {env_name}")
    failures.extend(gate_script_contract_failures(gate_script))
    return failures


def gate_script_contract_failures(script: str) -> list[str]:
    failures: list[str] = []
    if not re.search(r"(?m)^set -euo pipefail\s*$", script):
        failures.append("verdict script does not enable exact fail-fast shell policy")
    for job, env_name in REQUIRED_PR_GATE_RESULT_CHECKS:
        if not _has_exact_test(script, env_name, "success"):
            failures.append(f"verdict script lacks bare success assertion for {job}")
    for job, env_name, _selector in INDEPENDENT_RESULT_CHECKS:
        if not _has_exact_test(script, env_name, "skipped"):
            failures.append(f"verdict script lacks bare skipped assertion for {job}")

    selected = _selected_environment()
    _expect_result(script, selected, True, "all-selected success baseline", failures)
    for job, env_name in REQUIRED_PR_GATE_RESULT_CHECKS:
        for result in NON_SUCCESS_RESULTS:
            changed = {**selected, env_name: result}
            _expect_result(script, changed, False, f"selected {job}={result}", failures)
    for job, env_name, selector in INDEPENDENT_RESULT_CHECKS:
        unselected = {**selected, env_name: "skipped", selector: "false"}
        _expect_result(script, unselected, True, f"unselected {job}=skipped", failures)
        for result in ("success", "failure", "cancelled"):
            changed = {**unselected, env_name: result}
            _expect_result(script, changed, False, f"unselected {job}={result}", failures)
    return failures


def _has_exact_test(script: str, variable: str, expected: str) -> bool:
    return (
        re.search(
            rf'(?m)^\s*test\s+"\${re.escape(variable)}"\s+=\s+{expected}\s*$',
            script,
        )
        is not None
    )


def _selected_environment() -> dict[str, str]:
    environment = {
        "CI_OWNERS": "all",
        **{env_name: "success" for _job, env_name in REQUIRED_PR_GATE_RESULT_CHECKS},
    }
    environment.update(
        {selector: "true" for _job, _env_name, selector in INDEPENDENT_RESULT_CHECKS}
    )
    return environment


def _expect_result(
    script: str,
    environment: dict[str, str],
    expected_success: bool,
    case: str,
    failures: list[str],
) -> None:
    try:
        completed = subprocess.run(
            ("bash", "-s"),
            input=script,
            env={**os.environ, **environment},
            check=False,
            capture_output=True,
            text=True,
            timeout=5,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        failures.append(f"cannot execute verdict script for {case}: {error}")
        return
    succeeded = completed.returncode == 0
    if succeeded != expected_success:
        outcome = "accepted" if succeeded else "rejected"
        failures.append(f"verdict script {outcome} {case}")
