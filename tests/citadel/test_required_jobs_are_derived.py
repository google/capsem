"""Citadel guard: the set of CI jobs that must pass is derived, not restated.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one guards the check that decides whether anything may merge.
"""

from __future__ import annotations

from enum import StrEnum
from pathlib import Path

import yaml
from helpers.workflow_contract import canonical_shell_commands, referenced_need_results

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "ci.yaml"
SCRIPT = ROOT / "scripts" / "require-ci-jobs.sh"

#: The job that aggregates the others. It cannot require itself, so it is the
#: one job excluded from the derivation rather than an entry in a list.
AGGREGATOR = "pr-gate"


class Key(StrEnum):
    """The closed workflow vocabulary this guard reads."""

    JOBS = "jobs"
    STEPS = "steps"
    RUN = "run"
    ENV = "env"
    NEEDS = "needs"


class Gap(StrEnum):
    """The four ways a declared CI job can fall out of branch protection."""

    UNWAITED = "unwaited"
    UNBOUND = "unbound"
    UNREQUIRED = "unrequired"
    UNPASSED = "unpassed"


DERIVED_RATIONALE = """\
Four places have to agree about which CI jobs must pass, and nothing compared
them:

  `jobs:`                  the jobs that exist
  `pr-gate.needs:`         the jobs the gate waits for
  `pr-gate.env:`           each job's result, bound to a variable
  `require-ci-jobs.sh`     the variables the gate refuses to run without

A job added to the first and forgotten in any of the others is a check that
runs, can fail, and cannot block a merge. Branch protection stays green because
the one required status -- `pr-gate` -- was never told to look. Nothing fails,
which is precisely the problem: the failure mode is silence, and it lasts until
somebody happens to read four files together.

None of this is a list anybody should maintain. The set is derivable: every job
except the aggregator, which cannot require itself. So it is derived here, and
the four spellings are held to it.

`require-ci-jobs.sh` already carries the other half of this rule -- that each
comparison is a bare command whose exit status reaches the shell, because
`test X = success || true` turns a failing gate green. That guards how the
check runs. This guards *what it checks*.

See .github/workflows/ci.yaml and scripts/require-ci-jobs.sh.
"""


def workflow() -> dict:
    return yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))


def declared_jobs(document: dict) -> set[str]:
    """Every job in the workflow except the aggregator."""
    return set(document[Key.JOBS]) - {AGGREGATOR}


def gate_step(document: dict) -> dict:
    """The `pr-gate` step that runs the script, with its env block."""
    for step in document[Key.JOBS][AGGREGATOR][Key.STEPS]:
        if SCRIPT.name in str(step.get(Key.RUN, "")):
            return step
    raise AssertionError(f"{AGGREGATOR} no longer runs {SCRIPT.name}")


def bound_jobs(document: dict) -> dict[str, str]:
    """Variable name to the job whose result it carries."""
    found = {}
    for name, expression in (gate_step(document).get(Key.ENV) or {}).items():
        referenced = referenced_need_results(expression)
        assert len(referenced) <= 1, f"{name} combines multiple job results: {sorted(referenced)}"
        if referenced:
            found[name] = next(iter(referenced))
    return found


def required_variables(text: str) -> set[str]:
    """Variables in the script's required loop, parsed as shell words.

    Quoting and line continuation are presentation. The shared tokenizer
    removes both, so semantically identical shell stays green while a removed
    variable still changes the set.
    """
    loops = [
        command
        for command in canonical_shell_commands(text)
        if command[:3] == ("for", "required", "in")
    ]
    assert len(loops) == 1, f"{SCRIPT.name} must have one 'for required in' loop"
    loop = loops[0]
    try:
        end = loop.index("do", 3)
    except ValueError as error:
        raise AssertionError(f"{SCRIPT.name} required loop has no do") from error
    return {word for word in loop[3:end] if word != ";"}


def mismatches(document: dict, script: str) -> dict[Gap, list[str]]:
    """Every disagreement between the four spellings, named.

    One function rather than four assertions so the same derivation can be run
    against a deliberately broken workflow. Proving a guard fires belongs in
    the repository, not in a shell session that nobody can re-run.
    """
    jobs = declared_jobs(document)
    bound = bound_jobs(document)
    required = required_variables(script)
    passed = set(gate_step(document).get(Key.ENV) or {})
    return {
        Gap.UNWAITED: sorted(jobs - set(document[Key.JOBS][AGGREGATOR][Key.NEEDS])),
        Gap.UNBOUND: sorted(jobs - set(bound.values())),
        Gap.UNREQUIRED: sorted(set(bound) - required),
        Gap.UNPASSED: sorted(required - passed),
    }


def real() -> tuple[dict, str]:
    return workflow(), SCRIPT.read_text(encoding="utf-8")


#: What each disagreement means, for a reader who hits it once a year.
MEANING = {
    Gap.UNWAITED: "jobs pr-gate does not wait for",
    Gap.UNBOUND: "jobs whose result is never bound to a variable",
    Gap.UNREQUIRED: "results passed to the gate that it never requires",
    Gap.UNPASSED: "variables the script requires that nothing passes",
}


def test_the_four_spellings_agree() -> None:
    found = {MEANING[kind]: entries for kind, entries in mismatches(*real()).items() if entries}
    assert not found, DERIVED_RATIONALE + f"\n{found}"


def test_an_unwired_job_is_caught() -> None:
    """A job that exists, runs, can fail, and blocks nothing.

    The hole this guard was written for. Verified by hand first; encoded here
    because a hand-verification proves the guard worked once, on a tree nobody
    can reconstruct.
    """
    document, script = real()
    document[Key.JOBS]["security-audit"] = {"runs-on": "ubuntu-latest", "steps": []}
    found = mismatches(document, script)

    assert found[Gap.UNWAITED] == ["security-audit"]
    assert found[Gap.UNBOUND] == ["security-audit"]


def test_a_renamed_result_variable_is_caught_both_ways() -> None:
    """The subtler hole: wired and waited for, but read by nobody.

    Branch protection stays green over a check whose result reaches a variable
    the script never looks at.
    """
    document, script = real()
    env = gate_step(document)[Key.ENV]
    variable = next(name for name, value in env.items() if "test-install" in str(value))
    env[f"{variable}_RENAMED"] = env.pop(variable)
    found = mismatches(document, script)

    assert found[Gap.UNREQUIRED] == [f"{variable}_RENAMED"]
    assert found[Gap.UNPASSED] == [variable]


def test_a_job_removed_from_the_gate_is_caught() -> None:
    """Dropping a job from `needs:` alone silently stops requiring it."""
    document, script = real()
    dropped = document[Key.JOBS][AGGREGATOR][Key.NEEDS].pop()
    assert mismatches(document, script)[Gap.UNWAITED] == [dropped]


def optional_required_jobs(document: dict) -> list[str]:
    """Required jobs whose own failure GitHub is allowed to ignore."""
    return sorted(
        name
        for name in declared_jobs(document)
        if document[Key.JOBS][name].get("continue-on-error")
        not in {None, False, "false", "${{ false }}"}
    )


def test_no_required_job_is_optional() -> None:
    document, _script = real()
    assert not optional_required_jobs(document), DERIVED_RATIONALE


def test_a_job_level_continue_on_error_is_caught() -> None:
    document, _script = real()
    document[Key.JOBS]["test-install"]["continue-on-error"] = True
    assert optional_required_jobs(document) == ["test-install"]


def test_shell_presentation_does_not_change_the_required_set() -> None:
    """Quoting a loop word is not a change to what the shell requires."""
    _document, script = real()
    quoted = script.replace("FAST_GATE_RESULT", "'FAST_GATE_RESULT'", 1)

    assert required_variables(quoted) == required_variables(script)


def test_the_guard_reads_what_it_claims_to() -> None:
    """Break it here, so a refactor that blinds it fails rather than passes.

    Every derivation above compares two sets, and two empty sets compare equal.
    Each extractor is asserted non-empty for that reason.
    """
    document, script = real()
    assert len(declared_jobs(document)) > 3, "read too few jobs to trust this guard"
    assert bound_jobs(document), "no result bindings extracted"
    assert required_variables(script), "no required variables extracted"

    assert referenced_need_results("${{ needs.test-linux.result }}") == {"test-linux"}
    assert not referenced_need_results("${{ needs.docs-build.outputs.web_only }}"), (
        "an output is not a result; binding one would excuse a job from the gate"
    )
