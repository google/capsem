"""Citadel guard: a gating workflow step must be unable to pass while failing.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. Why this one exists, and the measurements behind it, are in
`WORKFLOW_ENFORCEMENT_RATIONALE` below -- stated there rather than here so a
violation prints it instead of a bare assertion.

## What counts as enforcement here

Narrowly: a **gate step** is a step mapping an environment name to an
expression reading `needs.<job>.result`. That is the step which turns a
dependency's outcome into this job's outcome, and it is the only place these
rules apply.

The looser reading -- any step running `test "$..."` -- was tried first and is
wrong twice over. It swept in `release.yaml:preflight` comparing a tag to a
ref, an ordinary precondition rather than a merge gate. And it would have
failed `ci.yaml:test-linux`, whose "Enable KVM (best-effort)" step carries
`continue-on-error: true` legitimately: a best-effort setup step is allowed to
fail, the step that decides the gate is not. A guard that cannot tell those
apart gets deleted the first time it blocks something reasonable.
"""

from __future__ import annotations

from enum import StrEnum
from pathlib import Path

import pytest
import yaml
from helpers.workflow_contract import (
    canonical_shell_commands,
    disables_fail_fast,
    masks_failure,
    referenced_need_results,
)

WORKFLOW_ENFORCEMENT_RATIONALE = """\
A gating workflow step must be provably unable to pass while failing.

`pr-gate` is the only status branch protection requires, so a `pr-gate` that
cannot fail is a main branch with no gate at all.

The contract that used to prove this matched literal text against the workflow
source, and measurement showed it inverted in both directions. Replaying its
twenty-four assertions against four mutations of the real ci.yaml:

    reformat needs[] to YAML block style   identical gate     went RED
    reorder needs[] alphabetically         identical gate     went RED
    append `|| true` to every enforcement  CI now advisory    stayed GREEN
    add `continue-on-error: true`          step cannot fail   stayed GREEN

The two edits GitHub cannot distinguish from the original broke the build; the
two that disable merge protection passed. `assert 'test "$X" = success' in gate`
is a substring test, and `test "$X" = success || true` contains that substring.
`continue-on-error` is a key the assertion never mentions, so it is invisible.

Ask the parsed document, never the source text, and judge fail-open spellings
with `workflow_contract.masks_failure`, which keeps shell operators as tokens.
A second opinion here is a second thing to keep in step with the first.

The mutations below are executable cases rather than prose: if any starts
passing, this contract has regressed to the thing it replaced.

See skills/dev-ci/SKILL.md and tests/helpers/workflow_contract.py.
"""

PROJECT_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = PROJECT_ROOT / ".github" / "workflows"
WORKFLOW_GLOB = "*.yaml"

#: The workflow this file's fixtures mutate. The contracts themselves sweep
#: every workflow; only the mutation cases need to name one.
CI_WORKFLOW = "ci.yaml"
#: The aggregating job branch protection requires. Named once, and only the
#: fixtures use it -- the contracts find gate steps by shape.
GATE_JOB = "pr-gate"

#: The shell builtin an enforcement comparison is spelled with.
ENFORCEMENT_COMMAND = "test"
#: What a `neutralize` mutation appends. `masks_failure` recognizes the whole
#: family; this is simply the one the fixture writes.
FAIL_OPEN_SUFFIX = " || true"


class Key(StrEnum):
    """The GitHub workflow keys this contract reads.

    A closed vocabulary of what *this* file depends on, not an attempt to model
    GitHub's schema. `StrEnum` members are `str`, so mapping access is
    unchanged and a typo becomes an AttributeError at import instead of a
    `.get` that quietly returns `None` and makes a contract vacuous.
    """

    JOBS = "jobs"
    STEPS = "steps"
    NEEDS = "needs"
    ENV = "env"
    RUN = "run"
    NAME = "name"
    IF = "if"
    CONTINUE_ON_ERROR = "continue-on-error"


class Condition(StrEnum):
    """The two job conditions this contract distinguishes."""

    ALWAYS = "always()"
    """Required on a gate job: a gate skipped when a dependency fails reports
    neither success nor failure, which is the one case it exists for."""

    SUCCESS = "${{ success() }}"
    """What the `skip-gate` mutation substitutes, and what must be rejected."""


def _document(name: str) -> dict:
    return yaml.safe_load((WORKFLOWS / name).read_text())


def _result_env(step: dict) -> dict[str, frozenset[str]]:
    """Environment names in `step` that carry a dependency's result.

    The reading of a `${{ }}` belongs to `workflow_contract`, which already
    owns that syntax for masking; this only decides which env entries count.
    """
    found = {}
    for name, value in (step.get(Key.ENV) or {}).items():
        referenced = referenced_need_results(value)
        if referenced:
            found[name] = referenced
    return found


def _gate_steps() -> list[tuple[str, str, dict, dict, dict[str, frozenset[str]]]]:
    """Every (workflow, job name, job, step, result-env) in the repository.

    Found by shape, so a second aggregating gate is covered on the day it is
    written rather than the day someone remembers to add it here.
    """
    found = []
    for path in sorted(WORKFLOWS.glob(WORKFLOW_GLOB)):
        for job_name, job in (_document(path.name).get(Key.JOBS) or {}).items():
            for step in job.get(Key.STEPS) or []:
                if not isinstance(step, dict):
                    continue
                results = _result_env(step)
                if results:
                    found.append((path.name, job_name, job, step, results))
    return found


def _decides_the_gate(command: tuple[str, ...], results: dict[str, frozenset[str]]) -> bool:
    """Whether one shell command is a comparison that decides the gate.

    An unrelated `test` in the same script -- a directory probe, a version
    check -- is not this contract's business, so the command must both be a
    `test` and mention one of the environment names carrying a dependency's
    result.
    """
    if command[:1] != (ENFORCEMENT_COMMAND,):
        return False
    return any(name in token for token in command for name in results)


def test_there_is_at_least_one_gate_step() -> None:
    """A guard built from the current state asserts nothing if that state is empty."""
    assert _gate_steps(), "no gate step found; every contract below would be vacuous"


def test_no_gate_step_may_continue_on_error() -> None:
    """`continue-on-error: true` makes a failing gate step a passing job.

    Invisible to every substring assertion in the doctor contract, because it
    is a key those assertions never name.
    """
    offenders = [
        f"{workflow}:{job_name}:{step.get(Key.NAME, '<unnamed>')}"
        for workflow, job_name, _job, step, _results in _gate_steps()
        if step.get(Key.CONTINUE_ON_ERROR) is True
    ]
    assert not offenders, (
        WORKFLOW_ENFORCEMENT_RATIONALE + f"\ngate steps that cannot fail: {offenders}"
    )


def test_no_enforcement_line_is_neutralized() -> None:
    """Nothing may make a `test "$RESULT" = ...` unable to fail its step.

    `|| true`, `|| :`, `; true` and `set +e` all leave the literal the doctor
    contract greps for perfectly intact while removing every effect it has on
    the job's exit status.

    The judgement is `workflow_contract.masks_failure`, not a regex of this
    file's own: that lexer keeps shell operators as tokens and already
    normalizes comments, quoting, repeated whitespace and line continuations,
    so a fail-open suffix cannot hide behind presentation. A second opinion
    here would be a second thing to keep in step with the first.
    """
    offenders = []
    for workflow, job_name, _job, step, results in _gate_steps():
        for command in canonical_shell_commands(str(step.get(Key.RUN, ""))):
            if disables_fail_fast(command):
                offenders.append(f"{workflow}:{job_name}: {' '.join(command)}")
                continue
            if not _decides_the_gate(command, results):
                continue
            if masks_failure(command):
                offenders.append(f"{workflow}:{job_name}: {' '.join(command)}")
    assert not offenders, (
        WORKFLOW_ENFORCEMENT_RATIONALE + f"\nneutralized enforcement: {offenders}"
    )


def test_every_result_a_gate_step_reads_is_a_declared_dependency() -> None:
    """`needs.<job>.result` is the empty string for a job absent from `needs:`.

    The env line still reads correctly and the doctor contract still matches
    it; the comparison downstream just silently stops being about anything.
    """
    for workflow, job_name, job, _step, results in _gate_steps():
        declared = set(job.get(Key.NEEDS) or [])
        for name, referenced_jobs in results.items():
            for referenced in sorted(referenced_jobs):
                assert referenced in declared, (
                    WORKFLOW_ENFORCEMENT_RATIONALE
                    + f"\n{workflow}:{job_name} reads {name}=needs.{referenced}.result "
                    + f"but declares needs: {sorted(declared)}"
                )


def test_every_declared_result_is_actually_tested() -> None:
    """Naming a dependency's result and never comparing it gates nothing."""
    for workflow, job_name, _job, step, results in _gate_steps():
        commands = canonical_shell_commands(str(step.get(Key.RUN, "")))
        tested = {
            name
            for command in commands
            if command[:1] == ("test",)
            for name in results
            if any(name in token for token in command)
        }
        missing = sorted(set(results) - tested)
        assert not missing, (
            WORKFLOW_ENFORCEMENT_RATIONALE
            + f"\n{workflow}:{job_name} reads but never tests: {missing}"
        )


def test_every_job_owning_a_gate_step_runs_when_a_dependency_fails() -> None:
    """`if: always()`, or the gate is skipped exactly when it is needed.

    A skipped job reports neither success nor failure, and branch protection
    requiring it either waits forever or treats it as satisfied.
    """
    for workflow, job_name, job, _step, _results in _gate_steps():
        condition = str(job.get(Key.IF, ""))
        assert Condition.ALWAYS in condition, (
            WORKFLOW_ENFORCEMENT_RATIONALE
            + f"\n{workflow}:{job_name} has if: {condition!r}"
        )


# ---------------------------------------------------------------------------
# The mutation cases. These are the reason this file exists.
# ---------------------------------------------------------------------------


class Mutation(StrEnum):
    """The closed set of edits that disable merge protection.

    Each one passed the literal-text contract this file replaces. They are a
    vocabulary rather than four strings because `_apply` dispatches on them
    exhaustively: a member added here without a `case` fails loudly instead of
    falling through to a silently unmutated document.
    """

    NEUTRALIZE = "neutralize"
    """`|| true` after each enforcement comparison. The literal a substring
    contract greps for survives intact; the exit status does not."""

    CONTINUE_ON_ERROR = "continue-on-error"
    """A key no substring assertion mentions, so text matching cannot see it
    at all. Makes the deciding step unable to fail its job."""

    DROP_NEED = "drop-need"
    """Removes a dependency the step still reads, so `needs.<job>.result`
    becomes the empty string and the comparison stops being about anything."""

    SKIP_GATE = "skip-gate"
    """`success()` instead of `always()`. The gate is skipped exactly when a
    dependency failed -- the one case it exists for."""


class Equivalent(StrEnum):
    """The closed set of edits GitHub cannot distinguish from the original.

    The half the literal contract got backwards: each of these turned four
    contracts red while changing nothing that runs.
    """

    REORDERED_NEEDS = "reordered-needs"
    BLOCK_STYLE_NEEDS = "block-style-needs"
    FULL_REFORMAT = "full-reformat"


def _apply(document: dict, mutation: Mutation) -> dict:
    """Break merge protection by editing the parsed document.

    These were text substitutions against literal anchors -- an exact
    `needs: [...]` line, an exact `- name:` line -- which made the fixtures
    themselves brittle in precisely the way this file exists to prevent:
    reformatting `needs:` into block style left the anchors unmatched, the
    `did not apply` guard fired, and the suite went red over an edit that
    changes nothing GitHub acts on. Measured, not theorised.

    So every mutation is now derived: the gate job and step are found by shape,
    and the dropped dependency is whichever one the step actually reads.
    """
    jobs = document[Key.JOBS]
    located = [
        (job_name, job, index, step)
        for job_name, job in jobs.items()
        for index, step in enumerate(job.get(Key.STEPS) or [])
        if isinstance(step, dict) and _result_env(step)
    ]
    assert located, "no gate step to mutate"
    job_name, job, index, step = located[0]
    results = _result_env(step)

    match mutation:
        case Mutation.NEUTRALIZE:
            job[Key.STEPS][index] = {
                **step,
                Key.RUN: "\n".join(
                    line + FAIL_OPEN_SUFFIX
                    if line.strip().startswith(f"{ENFORCEMENT_COMMAND} ")
                    and any(name in line for name in results)
                    else line
                    for line in str(step[Key.RUN]).splitlines()
                )
                + "\n",
            }
        case Mutation.CONTINUE_ON_ERROR:
            job[Key.STEPS][index] = {**step, Key.CONTINUE_ON_ERROR: True}
        case Mutation.DROP_NEED:
            # Whichever dependency the step genuinely reads, so its `.result`
            # silently becomes the empty string.
            read = sorted(frozenset().union(*results.values()))
            assert read, "gate step reads no dependency result"
            job[Key.NEEDS] = [name for name in job[Key.NEEDS] if name != read[0]]
        case Mutation.SKIP_GATE:
            job[Key.IF] = Condition.SUCCESS
        case _:
            raise AssertionError(f"unhandled mutation: {mutation}")

    jobs[job_name] = job
    return document


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


class _VocabularyDumper(yaml.SafeDumper):
    """Emit `Key` and `Condition` as the strings they already are.

    `safe_dump` dispatches on exact type, so a `StrEnum` used as a mapping key
    raises `RepresenterError` even though the member *is* a `str` and every
    read through `.get(Key.ENV)` works untouched. Registering the representer
    once here is what lets the mutations write through the same vocabulary they
    read through, instead of one half of the file reverting to raw literals.
    """


_VocabularyDumper.add_multi_representer(
    StrEnum, lambda dumper, value: dumper.represent_str(str(value))
)


def _rendered(document: dict, directory: Path) -> Path:
    directory.mkdir(parents=True, exist_ok=True)
    (directory / CI_WORKFLOW).write_text(
        yaml.dump(document, Dumper=_VocabularyDumper, sort_keys=False)
    )
    return directory


@pytest.mark.parametrize("mutation", tuple(Mutation))
def test_mutation_is_caught(mutation: Mutation, tmp_path: Path) -> None:
    """Each mutation disables merge protection and must be rejected.

    All four passed the literal-text contract this file exists to replace. If
    any starts passing here, the contract has regressed to what it replaced.

    Both trees are re-emitted from the parsed document, so the clean one is
    complete reformat of the real workflow -- different quoting, block style
    throughout, no comments. Proving it still passes is the false-positive half
    of this contract, and it costs nothing to assert here.
    """
    original = _document(CI_WORKFLOW)
    mutated = _apply(_document(CI_WORKFLOW), mutation)
    assert mutated != original, f"{mutation} changed nothing"

    assert _contract_accepts(_rendered(original, tmp_path / "clean")), (
        "a reformatted but semantically identical ci.yaml must pass"
    )
    assert not _contract_accepts(_rendered(mutated, tmp_path / "broken")), (
        f"{mutation} was not caught"
    )


@pytest.mark.parametrize("style", tuple(Equivalent))
def test_equivalent_yaml_is_not_a_failure(style: Equivalent, tmp_path: Path) -> None:
    """An edit GitHub cannot distinguish from the original must stay green.

    This is the half the literal-text contract got backwards, and it is worth
    a real assertion rather than a round-trip of a fragment. Reordering
    `needs:` or writing it in block style each turned four contracts red while
    changing nothing that runs; `full-reformat` re-emits the entire workflow,
    which drops every comment and rewrites all quoting and flow style.

    A false red here is not harmless. It trains people to edit the workflow and
    the contract together without reading either, which is how a real
    regression gets waved through.
    """
    document = _document(CI_WORKFLOW)
    gate = document[Key.JOBS][GATE_JOB]

    match style:
        case Equivalent.REORDERED_NEEDS:
            gate[Key.NEEDS] = list(reversed(gate[Key.NEEDS]))
        case Equivalent.BLOCK_STYLE_NEEDS:
            # `safe_dump` emits block style for every sequence, so rendering is
            # the mutation; asserting the parse makes that explicit rather than
            # incidental.
            rendered = yaml.dump({Key.NEEDS: gate[Key.NEEDS]}, Dumper=_VocabularyDumper)
            assert set(yaml.safe_load(rendered)[Key.NEEDS]) == set(gate[Key.NEEDS])
        case Equivalent.FULL_REFORMAT:
            # Rendering alone drops every comment and rewrites all quoting.
            pass
        case _:
            raise AssertionError(f"unhandled equivalence: {style}")

    assert _contract_accepts(_rendered(document, tmp_path / style)), (
        f"{style} changes nothing GitHub acts on and must not fail"
    )
