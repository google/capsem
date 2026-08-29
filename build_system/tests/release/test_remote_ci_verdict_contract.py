"""The live readiness check must prove the same CI verdict that merges code."""

from __future__ import annotations

from collections.abc import Callable
from pathlib import Path

import pytest
from capsem_builder.release.tools import check_remote_release_readiness as CHECKER
from capsem_builder.release.tools import remote_ci_gate as REMOTE_GATE

PROJECT_ROOT = Path(__file__).resolve().parents[3]
WORKFLOW = (PROJECT_ROOT / ".github/workflows/ci.yaml").read_text(encoding="utf-8")
VERDICT = (PROJECT_ROOT / REMOTE_GATE.GATE_SCRIPT_PATH).read_text(encoding="utf-8")


def gate_block(workflow: str = WORKFLOW) -> str:
    return REMOTE_GATE.workflow_job_block(workflow, "pr-gate")


def test_current_workflow_and_dispatched_script_form_one_fail_closed_contract() -> None:
    assert REMOTE_GATE.pr_gate_contract_failures(gate_block(), VERDICT) == []


@pytest.mark.parametrize(
    "mutate",
    [
        lambda text: text.replace("if: ${{ always() }}", "if: ${{ success() }}", 1),
        lambda text: text.replace(
            "run: bash build_system/scripts/ci/require-ci-jobs.sh",
            "run: bash build_system/scripts/ci/require-ci-jobs.sh || true",
            1,
        ),
        lambda text: text.replace(
            "name: Require all CI jobs",
            "continue-on-error: true\n        name: Require all CI jobs",
            1,
        ),
        lambda text: text.replace("scope, fast-gate, ", "fast-gate, ", 1),
    ],
    ids=("skip-on-failure", "neutralized-dispatch", "optional-step", "removed-need"),
)
def test_remote_workflow_mutations_are_rejected(mutate: Callable[[str], str]) -> None:
    broken = mutate(WORKFLOW)
    assert broken != WORKFLOW
    assert REMOTE_GATE.pr_gate_contract_failures(gate_block(broken), VERDICT)


@pytest.mark.parametrize(
    "mutate",
    [
        lambda text: text.replace("set -euo pipefail", "set +e", 1),
        lambda text: text.replace(
            'test "$FAST_GATE_RESULT" = success',
            'test "$FAST_GATE_RESULT" = success || true',
            1,
        ),
        lambda text: text.replace(
            'test "$FAST_GATE_RESULT" = success',
            'test "$FAST_GATE_RESULT" = success &',
            1,
        ),
        lambda text: text.replace(
            'test "$FAST_GATE_RESULT" = success',
            'test "$FAST_GATE_RESULT" = success | cat',
            1,
        ),
        lambda text: text.replace('test "$FAST_GATE_RESULT" = success\n', "", 1),
    ],
    ids=("no-fail-fast", "neutralized", "backgrounded", "piped", "discarded"),
)
def test_remote_verdict_script_mutations_are_observed_behaviorally(
    mutate: Callable[[str], str]
) -> None:
    broken = mutate(VERDICT)
    assert broken != VERDICT
    assert REMOTE_GATE.gate_script_contract_failures(broken)


def test_every_non_success_selected_result_and_non_skipped_unselected_result_fails() -> None:
    assert REMOTE_GATE.gate_script_contract_failures(VERDICT) == []


def test_behavioral_matrix_observes_every_job_and_result_state() -> None:
    permissive = VERDICT.replace("set -euo pipefail", "set +e", 1) + "\ntrue\n"
    failures = REMOTE_GATE.gate_script_contract_failures(permissive)
    for job, _env_name in REMOTE_GATE.REQUIRED_PR_GATE_RESULT_CHECKS:
        for result in REMOTE_GATE.NON_SUCCESS_RESULTS:
            assert f"verdict script accepted selected {job}={result}" in failures
    for job, _env_name, _selector in REMOTE_GATE.INDEPENDENT_RESULT_CHECKS:
        for result in ("success", "failure", "cancelled"):
            assert f"verdict script accepted unselected {job}={result}" in failures


def test_remote_check_reads_workflow_and_verdict_script_from_one_exact_revision(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    revision = "a" * 40
    calls: list[list[str]] = []
    replies = iter(
        [
            CHECKER.TextResult(0, "registered", ""),
            CHECKER.TextResult(0, revision + "\n", ""),
            CHECKER.TextResult(0, WORKFLOW, ""),
            CHECKER.TextResult(0, VERDICT, ""),
        ]
    )

    def run_text(argv: list[str]) -> CHECKER.TextResult:
        calls.append(argv)
        return next(replies)

    monkeypatch.setattr(CHECKER, "run_text", run_text)
    result = CHECKER.check_remote_pr_gate("google/capsem", "main")

    assert result.ok, result.detail
    assert calls[0][:4] == ["gh", "workflow", "view", "ci.yaml"]
    assert calls[1] == ["gh", "api", "repos/google/capsem/commits/main", "--jq", ".sha"]
    assert all(f"ref={revision}" in call[-1] for call in calls[2:])
