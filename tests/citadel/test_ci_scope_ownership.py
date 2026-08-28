"""Hold CI job ownership and fail-closed scope classification."""

from __future__ import annotations

from copy import deepcopy
from pathlib import Path
from typing import Any

import yaml

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "ci.yaml"

CI_SCOPE_RATIONALE = """\
Every tracked source path must name the CI jobs that own it. Missing jobs,
disabled jobs, and a classifier whose failure is discarded are equivalent to
unowned source: a rename can otherwise make required verification silently
stop running. Classification and the final aggregate job therefore fail
closed.
"""

REQUIRED_JOBS = frozenset(
    {
        "fast-gate",
        "test-linux",
        "test",
        "test-install",
        "docs-build",
        "site-build",
        "release-site-build",
        "pr-gate",
    }
)


def _workflow_problems(workflow: dict[str, Any]) -> list[str]:
    jobs = workflow.get("jobs")
    if not isinstance(jobs, dict):
        return ["workflow has no jobs mapping"]

    problems = [f"missing required CI job: {job}" for job in sorted(REQUIRED_JOBS - set(jobs))]
    for job_name in sorted(REQUIRED_JOBS & set(jobs)):
        job = jobs[job_name]
        if not isinstance(job, dict):
            problems.append(f"required CI job is not a mapping: {job_name}")
            continue
        condition = job.get("if")
        if condition is False or str(condition).strip().lower() in {
            "false",
            "${{ false }}",
            "${{false}}",
        }:
            problems.append(f"required CI job is disabled: {job_name}")

    final = jobs.get("pr-gate")
    if isinstance(final, dict):
        needs = final.get("needs", [])
        if isinstance(needs, str):
            needs = [needs]
        missing_needs = (REQUIRED_JOBS - {"pr-gate"}) - set(needs)
        problems.extend(
            f"pr-gate does not need required owner: {job}"
            for job in sorted(missing_needs)
        )

    docs_job = jobs.get("docs-build")
    steps = docs_job.get("steps", []) if isinstance(docs_job, dict) else []
    classifiers = [
        step
        for step in steps
        if isinstance(step, dict)
        and step.get("name") == "Classify pull request scope"
    ]
    if len(classifiers) != 1:
        problems.append(f"expected one CI scope classifier step, found {len(classifiers)}")
    else:
        classifier = classifiers[0]
        if classifier.get("continue-on-error") in {True, "true", "True"}:
            problems.append("CI scope classifier discards failure")
        run = classifier.get("run", "")
        if "python3 scripts/classify-ci-scope.py" not in run:
            problems.append("CI scope classifier does not call the owned script")
        if any(token in run for token in ("|| true", "; true", "set +e")):
            problems.append("CI scope classifier neutralizes a failing command")
    return problems


def _workflow() -> dict[str, Any]:
    loaded = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
    assert isinstance(loaded, dict), CI_SCOPE_RATIONALE
    return loaded


def test_missing_required_job_is_rejected() -> None:
    workflow = _workflow()
    del workflow["jobs"]["test-linux"]
    assert _workflow_problems(workflow), CI_SCOPE_RATIONALE


def test_disabled_required_job_is_rejected() -> None:
    workflow = _workflow()
    workflow["jobs"]["test"]["if"] = "${{ false }}"
    assert _workflow_problems(workflow), CI_SCOPE_RATIONALE


def test_discarded_classifier_failure_is_rejected() -> None:
    workflow = _workflow()
    classifier = next(
        step
        for step in workflow["jobs"]["docs-build"]["steps"]
        if step.get("name") == "Classify pull request scope"
    )
    classifier["continue-on-error"] = True
    assert _workflow_problems(workflow), CI_SCOPE_RATIONALE


def test_final_gate_must_need_every_owner() -> None:
    workflow = _workflow()
    mutated = deepcopy(workflow)
    mutated["jobs"]["pr-gate"]["needs"].remove("site-build")
    assert _workflow_problems(mutated), CI_SCOPE_RATIONALE


def test_current_ci_workflow_is_owned_and_enforced() -> None:
    problems = _workflow_problems(_workflow())
    assert not problems, CI_SCOPE_RATIONALE + "\n" + "\n".join(problems)
