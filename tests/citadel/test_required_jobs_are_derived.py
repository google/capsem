"""Citadel guard: the set of CI jobs that must pass is derived, not restated.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one guards the check that decides whether anything may merge.
"""

from __future__ import annotations

import re
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "ci.yaml"
SCRIPT = ROOT / "scripts" / "require-ci-jobs.sh"

#: The job that aggregates the others. It cannot require itself, so it is the
#: one job excluded from the derivation rather than an entry in a list.
AGGREGATOR = "pr-gate"

#: `${{ needs.<job>.result }}` -- how `pr-gate` binds a job's outcome to the
#: variable the script reads. This is the authoritative mapping, because the
#: names differ: the job is `test`, the variable is `TEST_MACOS_RESULT`.
RESULT_BINDING = re.compile(r"needs\.(?P<job>[\w-]+)\.result")

#: The names the script insists on before it will judge anything.
REQUIRED_IN_SCRIPT = re.compile(r"^\s*for\s+required\s+in\s+(?P<names>.+?)(?:;|\s+do)", re.S | re.M)

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
    return set(document["jobs"]) - {AGGREGATOR}


def gate_step(document: dict) -> dict:
    """The `pr-gate` step that runs the script, with its env block."""
    for step in document["jobs"][AGGREGATOR]["steps"]:
        if SCRIPT.name in str(step.get("run", "")):
            return step
    raise AssertionError(f"{AGGREGATOR} no longer runs {SCRIPT.name}")


def bound_jobs(document: dict) -> dict[str, str]:
    """Variable name to the job whose result it carries."""
    found = {}
    for name, expression in (gate_step(document).get("env") or {}).items():
        match = RESULT_BINDING.search(str(expression))
        if match:
            found[name] = match.group("job")
    return found


def required_variables(text: str) -> set[str]:
    match = REQUIRED_IN_SCRIPT.search(text)
    assert match, f"{SCRIPT.name} no longer names the variables it requires"
    return {word for word in match.group("names").split() if word.isupper()}


def mismatches(document: dict, script: str) -> dict[str, list[str]]:
    """Every disagreement between the four spellings, named.

    One function rather than four assertions so the same derivation can be run
    against a deliberately broken workflow. Proving a guard fires belongs in
    the repository, not in a shell session that nobody can re-run.
    """
    jobs = declared_jobs(document)
    bound = bound_jobs(document)
    required = required_variables(script)
    passed = set(gate_step(document).get("env") or {})
    return {
        "unwaited": sorted(jobs - set(document["jobs"][AGGREGATOR]["needs"])),
        "unbound": sorted(jobs - set(bound.values())),
        "unrequired": sorted(set(bound) - required),
        "unpassed": sorted(required - passed),
    }


def real() -> tuple[dict, str]:
    return workflow(), SCRIPT.read_text(encoding="utf-8")


#: What each disagreement means, for a reader who hits it once a year.
MEANING = {
    "unwaited": "jobs pr-gate does not wait for",
    "unbound": "jobs whose result is never bound to a variable",
    "unrequired": "results passed to the gate that it never requires",
    "unpassed": "variables the script requires that nothing passes",
}


def test_the_four_spellings_agree() -> None:
    found = {
        MEANING[kind]: entries for kind, entries in mismatches(*real()).items() if entries
    }
    assert not found, DERIVED_RATIONALE + f"\n{found}"


def test_an_unwired_job_is_caught() -> None:
    """A job that exists, runs, can fail, and blocks nothing.

    The hole this guard was written for. Verified by hand first; encoded here
    because a hand-verification proves the guard worked once, on a tree nobody
    can reconstruct.
    """
    document, script = real()
    document["jobs"]["security-audit"] = {"runs-on": "ubuntu-latest", "steps": []}
    found = mismatches(document, script)

    assert found["unwaited"] == ["security-audit"]
    assert found["unbound"] == ["security-audit"]


def test_a_renamed_result_variable_is_caught_both_ways() -> None:
    """The subtler hole: wired and waited for, but read by nobody.

    Branch protection stays green over a check whose result reaches a variable
    the script never looks at.
    """
    document, script = real()
    env = gate_step(document)["env"]
    variable = next(name for name, value in env.items() if "test-install" in str(value))
    env[f"{variable}_RENAMED"] = env.pop(variable)
    found = mismatches(document, script)

    assert found["unrequired"] == [f"{variable}_RENAMED"]
    assert found["unpassed"] == [variable]


def test_a_job_removed_from_the_gate_is_caught() -> None:
    """Dropping a job from `needs:` alone silently stops requiring it."""
    document, script = real()
    dropped = document["jobs"][AGGREGATOR]["needs"].pop()
    assert mismatches(document, script)["unwaited"] == [dropped]


def test_the_guard_reads_what_it_claims_to() -> None:
    """Break it here, so a refactor that blinds it fails rather than passes.

    Every derivation above compares two sets, and two empty sets compare equal.
    Each extractor is asserted non-empty for that reason.
    """
    document, script = real()
    assert len(declared_jobs(document)) > 3, "read too few jobs to trust this guard"
    assert bound_jobs(document), "no result bindings extracted"
    assert required_variables(script), "no required variables extracted"

    assert RESULT_BINDING.search("${{ needs.test-linux.result }}")
    assert not RESULT_BINDING.search("${{ needs.docs-build.outputs.web_only }}"), (
        "an output is not a result; binding one would excuse a job from the gate"
    )
