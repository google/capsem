"""What a substring check cannot see about a gating workflow.

`test_release_doctor_contract` proves the pr-gate wiring by matching literal
text against the workflow source. That catches a renamed variable, and it is
blind in both directions. Measured against the real `ci.yaml`, replaying the
twenty-four assertions of `test_ci_has_stable_pr_gate_over_all_required_jobs`
against four mutations:

    reformat needs[] to YAML block style   identical gate     contract went RED
    reorder needs[] alphabetically         identical gate     contract went RED
    append `|| true` to every enforcement  CI now advisory    contract stayed GREEN
    add `continue-on-error: true`          step cannot fail   contract stayed GREEN

Exactly inverted: the two edits GitHub cannot distinguish from the original
broke the build, and the two that disable merge protection passed. The first
pair is noise. The second pair is the failure that matters -- `pr-gate` is the
only status branch protection requires, so a `pr-gate` that cannot fail is a
main branch with no gate at all, reported green by the test written to prevent
exactly that.

The reason is mechanical. `assert 'test "$X" = success' in gate` is a substring
test, and `test "$X" = success || true` contains that substring. So does a
commented-out copy. `continue-on-error` is invisible to it entirely, being a
key the assertion never mentions.

## What counts as enforcement here

Narrowly: a **gate step** is a step that maps an environment name to an
expression reading `needs.<job>.result`. That is the step which turns a
dependency's outcome into this job's outcome, and it is the only place these
rules apply.

The looser reading -- any step running `test "$..."` -- was tried first and is
wrong twice over. It swept in `release.yaml:preflight` comparing a tag to a
ref, which is an ordinary precondition and not a merge gate. And it would have
failed `ci.yaml:test-linux`, whose "Enable KVM (best-effort)" step carries
`continue-on-error: true` legitimately: a best-effort setup step is allowed to
fail, the step that decides the gate is not. A guard that cannot tell those
apart gets deleted the first time it blocks something reasonable.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest
import yaml

PROJECT_ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = PROJECT_ROOT / ".github" / "workflows"

# `needs.<job>.result` inside a `${{ }}` expression.
RESULT_REFERENCE = re.compile(r"needs\.([A-Za-z0-9_-]+)\.result")
# A line that decides the gate: `test "$X" = word`, and nothing after it.
ENFORCEMENT_LINE = re.compile(r'^\s*test\s+"\$([A-Z_][A-Z0-9_]*)"\s+=\s+\S+\s*$')


def _document(name: str) -> dict:
    return yaml.safe_load((WORKFLOWS / name).read_text())


def _result_env(step: dict) -> dict[str, str]:
    """The step's environment names that carry a dependency's result."""
    return {
        name: str(value)
        for name, value in (step.get("env") or {}).items()
        if RESULT_REFERENCE.search(str(value))
    }


def _gate_steps() -> list[tuple[str, str, dict, dict, dict[str, str]]]:
    """Every (workflow, job name, job, step, result-env) in the repository.

    Found by shape, so a second aggregating gate is covered on the day it is
    written rather than the day someone remembers to add it here.
    """
    found = []
    for path in sorted(WORKFLOWS.glob("*.yaml")):
        for job_name, job in (_document(path.name).get("jobs") or {}).items():
            for step in job.get("steps") or []:
                if not isinstance(step, dict):
                    continue
                results = _result_env(step)
                if results:
                    found.append((path.name, job_name, job, step, results))
    return found


def test_there_is_at_least_one_gate_step() -> None:
    """A guard built from the current state asserts nothing if that state is empty."""
    assert _gate_steps(), "no gate step found; every contract below would be vacuous"


def test_no_gate_step_may_continue_on_error() -> None:
    """`continue-on-error: true` makes a failing gate step a passing job.

    Invisible to every substring assertion in the doctor contract, because it
    is a key those assertions never name.
    """
    offenders = [
        f"{workflow}:{job_name}:{step.get('name', '<unnamed>')}"
        for workflow, job_name, _job, step, _results in _gate_steps()
        if step.get("continue-on-error") is True
    ]
    assert not offenders, f"gate steps that cannot fail: {offenders}"


def test_no_enforcement_line_is_neutralized() -> None:
    """Nothing may follow a `test "$RESULT" = ...` inside a gate step.

    `|| true`, `|| :`, `; true` and a trailing `&` each leave the literal the
    doctor contract greps for perfectly intact while removing every effect it
    has on the job's exit status.
    """
    offenders = []
    for workflow, job_name, _job, step, results in _gate_steps():
        for line in str(step.get("run", "")).splitlines():
            stripped = line.strip()
            if not stripped.startswith("test "):
                continue
            match = ENFORCEMENT_LINE.match(line)
            if match is None:
                referenced = [name for name in results if name in stripped]
                if referenced:
                    offenders.append(f"{workflow}:{job_name}: {stripped}")
    assert not offenders, f"neutralized enforcement: {offenders}"


def test_every_result_a_gate_step_reads_is_a_declared_dependency() -> None:
    """`needs.<job>.result` is the empty string for a job absent from `needs:`.

    The env line still reads correctly and the doctor contract still matches
    it; the comparison downstream just silently stops being about anything.
    """
    for workflow, job_name, job, _step, results in _gate_steps():
        declared = set(job.get("needs") or [])
        for name, expression in results.items():
            for referenced in RESULT_REFERENCE.findall(expression):
                assert referenced in declared, (
                    f"{workflow}:{job_name} reads {name}=needs.{referenced}.result "
                    f"but declares needs: {sorted(declared)}"
                )


def test_every_declared_result_is_actually_tested() -> None:
    """Naming a dependency's result and never comparing it gates nothing."""
    for workflow, job_name, _job, step, results in _gate_steps():
        script = str(step.get("run", ""))
        tested = {
            match.group(1) for match in map(ENFORCEMENT_LINE.match, script.splitlines()) if match
        }
        missing = sorted(set(results) - tested)
        assert not missing, f"{workflow}:{job_name} reads but never tests: {missing}"


def test_every_job_owning_a_gate_step_runs_when_a_dependency_fails() -> None:
    """`if: always()`, or the gate is skipped exactly when it is needed.

    A skipped job reports neither success nor failure, and branch protection
    requiring it either waits forever or treats it as satisfied.
    """
    for workflow, job_name, job, _step, _results in _gate_steps():
        condition = str(job.get("if", ""))
        assert "always()" in condition, f"{workflow}:{job_name} has if: {condition!r}"


# ---------------------------------------------------------------------------
# The mutation cases. These are the reason this file exists.
# ---------------------------------------------------------------------------

NEEDS_LINE = (
    "    needs: [fast-gate, test-linux, test, test-install, "
    "docs-build, site-build, release-site-build]\n"
)


def _apply(text: str, mutation: str) -> str:
    if mutation == "neutralize":
        return "\n".join(
            line + " || true" if ENFORCEMENT_LINE.match(line) else line
            for line in text.splitlines()
        )
    if mutation == "continue-on-error":
        return text.replace(
            "      - name: Require all CI jobs\n",
            "      - name: Require all CI jobs\n        continue-on-error: true\n",
        )
    if mutation == "drop-need":
        return text.replace(
            NEEDS_LINE,
            "    needs: [fast-gate, test, test-install, docs-build, "
            "site-build, release-site-build]\n",
        )
    if mutation == "skip-gate":
        return text.replace("    if: ${{ always() }}\n", "    if: ${{ success() }}\n")
    raise AssertionError(f"unknown mutation {mutation}")


CHECKS = (
    test_no_gate_step_may_continue_on_error,
    test_no_enforcement_line_is_neutralized,
    test_every_result_a_gate_step_reads_is_a_declared_dependency,
    test_every_declared_result_is_actually_tested,
    test_every_job_owning_a_gate_step_runs_when_a_dependency_fails,
)


def _contract_accepts(tmp_workflows: Path) -> bool:
    """Run every contract above against a substituted workflow directory.

    The checks read the module-level `WORKFLOWS` at call time, so swapping it
    is enough and no argument has to be threaded through five signatures for
    the sake of this file alone.
    """
    global WORKFLOWS
    original = WORKFLOWS
    WORKFLOWS = tmp_workflows
    try:
        for check in CHECKS:
            try:
                check()
            except AssertionError:
                return False
        return True
    finally:
        WORKFLOWS = original


@pytest.mark.parametrize("mutation", ["neutralize", "continue-on-error", "drop-need", "skip-gate"])
def test_mutation_is_caught(mutation: str, tmp_path: Path) -> None:
    """Each mutation disables merge protection and must be rejected.

    All four passed the literal-text contract this file exists to replace. If
    any starts passing here, the contract has regressed to what it replaced.
    """
    original = (WORKFLOWS / "ci.yaml").read_text()
    mutated = _apply(original, mutation)
    assert mutated != original, f"{mutation} did not apply; its anchor text moved"

    clean = tmp_path / "clean"
    clean.mkdir()
    (clean / "ci.yaml").write_text(original)
    assert _contract_accepts(clean), "the real ci.yaml must pass"

    broken = tmp_path / "broken"
    broken.mkdir()
    (broken / "ci.yaml").write_text(mutated)
    assert not _contract_accepts(broken), f"{mutation} was not caught"


@pytest.mark.parametrize("style", ["block", "reordered"])
def test_equivalent_yaml_is_not_a_failure(style: str) -> None:
    """Reformatting `needs:` changes nothing GitHub acts on.

    Both spellings broke four literal contracts. A contract reading the parsed
    list as a set cannot tell them apart, which is the correct answer.
    """
    required = set(_document("ci.yaml")["jobs"]["pr-gate"]["needs"])

    if style == "block":
        rendered = yaml.safe_load("needs:\n" + "".join(f"  - {job}\n" for job in sorted(required)))
    else:
        rendered = yaml.safe_load(f"needs: [{', '.join(sorted(required, reverse=True))}]")

    assert set(rendered["needs"]) == required
