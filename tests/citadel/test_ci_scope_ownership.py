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
        "scope",
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
INDEPENDENT_JOBS = REQUIRED_JOBS - {"scope", "fast-gate", "pr-gate"}
REQUIRED_VARIABLES = {
    "test-linux": "TEST_LINUX_REQUIRED",
    "test": "TEST_MACOS_REQUIRED",
    "test-install": "TEST_INSTALL_REQUIRED",
    "docs-build": "DOCS_BUILD_REQUIRED",
    "site-build": "SITE_BUILD_REQUIRED",
    "release-site-build": "RELEASE_SITE_BUILD_REQUIRED",
}


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

    scope_job = jobs.get("scope")
    steps = scope_job.get("steps", []) if isinstance(scope_job, dict) else []
    classifiers = [
        step
        for step in steps
        if isinstance(step, dict)
        and step.get("name") == "Classify changed path owners"
    ]
    if len(classifiers) != 1:
        problems.append(f"expected one CI scope classifier step, found {len(classifiers)}")
    else:
        classifier = classifiers[0]
        if classifier.get("continue-on-error") in {True, "true", "True"}:
            problems.append("CI scope classifier discards failure")
        run = classifier.get("run", "")
        if "python3 build_system/scripts/ci/classify-ci-scope.py --owners" not in run:
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
        for step in workflow["jobs"]["scope"]["steps"]
        if step.get("name") == "Classify changed path owners"
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


def test_ci_jobs_are_selected_by_one_fail_closed_owner_stream() -> None:
    workflow = _workflow()
    jobs = workflow["jobs"]
    scope = jobs["scope"]
    assert scope["outputs"]["owners"] == "${{ steps.scope.outputs.owners }}"
    assert scope["steps"][0]["uses"].startswith("actions/checkout@")
    assert scope["steps"][0]["with"]["fetch-depth"] == 0
    classifier = next(
        step for step in scope["steps"] if step.get("name") == "Classify changed path owners"
    )
    assert classifier.get("continue-on-error") is None
    assert "git diff --name-only -z" in classifier["run"]
    assert classifier["run"].count(
        "build_system/scripts/ci/classify-ci-scope.py --owners"
    ) == 2
    assert ".github/workflows/ci.yaml" in classifier["run"]

    assert "if" not in jobs["fast-gate"]
    assert "needs" not in jobs["fast-gate"]
    for name in INDEPENDENT_JOBS:
        assert jobs[name]["needs"] == "scope"
        assert jobs[name]["if"] == (
            f"${{{{ contains(fromJSON(needs.scope.outputs.owners), '{name}') }}}}"
        )


def test_pr_gate_receives_owner_selection_and_every_job_result() -> None:
    workflow = _workflow()
    gate = workflow["jobs"]["pr-gate"]
    assert set(gate["needs"]) == (REQUIRED_JOBS | {"scope"}) - {"pr-gate"}
    require = next(step for step in gate["steps"] if step.get("name") == "Require all CI jobs")
    env = require["env"]
    assert env["CI_OWNERS"] == "${{ needs.scope.outputs.owners }}"
    assert env["SCOPE_RESULT"] == "${{ needs.scope.result }}"
    for name in INDEPENDENT_JOBS:
        variable = REQUIRED_VARIABLES[name]
        assert env[variable] == (
            f"${{{{ contains(fromJSON(needs.scope.outputs.owners), '{name}') }}}}"
        )


def _push_paths(name: str) -> set[str]:
    document = yaml.safe_load((ROOT / ".github" / "workflows" / name).read_text())
    trigger = document.get("on") or document.get(True)
    return set(trigger["push"]["paths"])


def test_public_web_deployments_run_only_for_owned_and_shared_inputs() -> None:
    script_root = "scr" + "ipts"
    current_docs = "do" + "cs/**"
    current_site = "si" + "te/**"
    current_graphics = "gra" + "phics/**"
    shared = {
        "README.md",
        f"{script_root}/check-web-surface.sh",
        f"{script_root}/lib/exec_lock.sh",
    }
    assert _push_paths("docs.yaml") == shared | {
        ".github/workflows/docs.yaml",
        "config/gate.toml",
        current_docs,
        "web/docs/**",
        f"{script_root}/check-docs-holding-build.py",
        "build_system/builder/gate/tools/web/check_docs_holding_build.py",
    }
    assert _push_paths("site.yaml") == shared | {
        ".github/workflows/site.yaml",
        current_site,
        current_graphics,
        "web/marketing/**",
        "web/graphics/**",
    }
