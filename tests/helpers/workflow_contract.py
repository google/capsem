"""Structural contracts for mandatory GitHub workflow commands."""

from __future__ import annotations

import re
from collections.abc import Mapping
from dataclasses import dataclass
from itertools import pairwise
from typing import Any

from .shelltokens import OPERATORS, REDIRECTS, tokenize

_CONTINUATION = re.compile(r"\\\r?\n")
_GITHUB_EXPRESSION = re.compile(r"\$\{\{\s*(.*?)\s*}}")
# `needs.<job>.result`, read only from inside a `${{ }}` expression: the same
# text outside one is prose, and matching it anywhere would make a comment
# authoritative.
_NEEDS_RESULT = re.compile(r"\bneeds\.([A-Za-z0-9_-]+)\.result\b")


def referenced_need_results(value: object) -> frozenset[str]:
    """Jobs whose `.result` an expression reads.

    Lives beside the expression pattern rather than in a caller, because this
    module already owns what a `${{ }}` is: `canonical_shell_commands` masks
    them so `shlex` cannot split one, and a second reader elsewhere would be a
    second definition to keep in step.

    Non-expression text yields nothing, so a step whose env merely mentions
    `needs.foo.result` in a comment is not treated as reading it.
    """
    if not isinstance(value, str):
        return frozenset()
    return frozenset(
        job
        for expression in _GITHUB_EXPRESSION.findall(value)
        for job in _NEEDS_RESULT.findall(expression)
    )


@dataclass(frozen=True)
class RequiredJustStep:
    """One workflow step whose exact ``just`` commands must succeed."""

    workflow: str
    job: str
    step: str
    commands: tuple[str, ...]
    condition: str | bool | None = None


@dataclass(frozen=True)
class _ObservedJustStep:
    commands: tuple[tuple[str, ...], ...]
    condition: object


def canonical_shell_commands(script: str) -> tuple[tuple[str, ...], ...]:
    """Return simple shell commands independent of presentation details.

    Delegates to `shelltokens.tokenize`, which scans characters rather than
    lines. Comments, quoting, repeated whitespace and line continuations
    therefore cannot become accidental contract authority, shell operators
    remain their own tokens so a fail-open suffix cannot disappear, and a word
    containing a newline no longer raises.
    """
    return tokenize(script)


def _canonical_condition(value: object) -> object:
    if not isinstance(value, str):
        return value
    text = value.strip()
    if not (text.startswith("${{") and text.endswith("}}")):
        return text
    inner = text[3:-2]
    normalized: list[str] = []
    quote: str | None = None
    escaped = False
    for character in inner:
        if escaped:
            normalized.append(character)
            escaped = False
        elif quote and character == "\\":
            normalized.append(character)
            escaped = True
        elif character in ("'", '"'):
            normalized.append(character)
            quote = None if quote == character else character if quote is None else quote
        elif not character.isspace() or quote:
            normalized.append(character)
    return "${{" + "".join(normalized) + "}}"


def _disabled(value: object) -> bool:
    return _canonical_condition(value) in (None, False, "false", "${{false}}")


def _unconditional(value: object) -> bool:
    return _canonical_condition(value) in (None, True, "true", "${{true}}")


def masks_failure(command: tuple[str, ...]) -> bool:
    """Whether a command's exit status can no longer fail the step running it.

    Public because two contracts ask it of two different step selections:
    `assert_unmasked_step` for the mandatory `just` steps declared here, and
    `tests/citadel/test_workflow_enforcement.py` for any step turning a dependency's
    `needs.<job>.result` into a job outcome. One definition, so a fail-open
    spelling learned in either place is caught in both.
    """
    pairs = tuple(pairwise(command))
    return "||" in command or any(pair == (";", "true") for pair in pairs)


def disables_fail_fast(command: tuple[str, ...]) -> bool:
    """Whether a command turns off the shell's exit-on-error behaviour.

    Any `set` carrying a `+` option, not the exact `set +e` this used to match.
    An adversarial pass walked `set +ex` and `set +o errexit` straight past that
    equality while doing the same thing.
    """
    if command[:1] != ("set",):
        return False
    return any(argument.startswith("+") for argument in command[1:])


#: Every token that can change how a command's exit status is consumed.
STATUS_CONSUMING = frozenset(OPERATORS) | frozenset(REDIRECTS)


def is_bare_command(command: tuple[str, ...]) -> bool:
    """Whether a command stands alone, with nothing consuming its status.

    A whitelist, deliberately, and the reason is measured. `masks_failure`
    enumerates ways to neutralise a check, and enumerating was losing: an
    adversarial pass got five past it -- `; :`, a trailing `&`, `| cat`,
    `set +ex`, `set +o errexit` -- and nothing suggests that list was complete.

    Inverting it is complete by construction. An enforcement comparison must be
    the whole command: no operator, no redirection, nothing after it. Anything
    that changes how its exit status is read appears as a token here, whether
    or not anyone predicted that spelling.

    `masks_failure` stays for `assert_unmasked_step`, which asks a different
    question of whole declared `just` steps rather than of one comparison.
    """
    return not any(token in STATUS_CONSUMING for token in command)


def assert_unmasked_step(
    workflow_name: str,
    workflow: Mapping[str, Any],
    job_name: str,
    step_name: str,
    *,
    condition: str | bool | None = None,
) -> tuple[tuple[str, ...], ...]:
    """Return one required step's commands after proving failure propagation."""
    jobs = workflow.get("jobs")
    assert isinstance(jobs, dict), f"{workflow_name}: jobs must be a mapping"
    job = jobs.get(job_name)
    assert isinstance(job, dict), f"{workflow_name}:{job_name}: required job is missing"
    assert _disabled(job.get("continue-on-error")), (
        f"{workflow_name}:{job_name}: job masks enforcement failures"
    )
    assert _canonical_condition(job.get("if")) not in (False, "false", "${{false}}"), (
        f"{workflow_name}:{job_name}: job is statically disabled"
    )
    matching = [
        step
        for step in job.get("steps") or ()
        if isinstance(step, dict) and step.get("name") == step_name
    ]
    assert len(matching) == 1, (
        f"{workflow_name}:{job_name}:{step_name}: expected exactly one required step"
    )
    step = matching[0]
    assert _disabled(step.get("continue-on-error")), (
        f"{workflow_name}:{job_name}:{step_name}: step masks enforcement failures"
    )
    condition_matches = (
        _unconditional(step.get("if"))
        if condition is None
        else _canonical_condition(step.get("if")) == _canonical_condition(condition)
    )
    assert condition_matches, (
        f"{workflow_name}:{job_name}:{step_name}: enforcement condition drifted; "
        f"expected={condition!r}, actual={step.get('if')!r}"
    )
    run = step.get("run")
    assert isinstance(run, str) and run.strip(), (
        f"{workflow_name}:{job_name}:{step_name}: required step has no command"
    )
    commands = canonical_shell_commands(run)
    assert not any(masks_failure(command) for command in commands), (
        f"{workflow_name}:{job_name}:{step_name}: shell masks an enforcement failure"
    )
    assert not any(disables_fail_fast(command) for command in commands), (
        f"{workflow_name}:{job_name}:{step_name}: shell disables fail-fast behavior"
    )
    pipeline_indexes = [index for index, command in enumerate(commands) if "|" in command]
    if pipeline_indexes:
        pipefail_indexes = [
            index
            for index, command in enumerate(commands)
            if command in (("set", "-o", "pipefail"), ("set", "-eo", "pipefail"))
        ]
        assert pipefail_indexes and pipefail_indexes[0] < pipeline_indexes[0], (
            f"{workflow_name}:{job_name}:{step_name}: pipeline can discard enforcement status"
        )
    return commands


def _workflow_just_steps(
    workflows: Mapping[str, Mapping[str, Any]],
) -> dict[tuple[str, str, str], _ObservedJustStep]:
    found: dict[tuple[str, str, str], _ObservedJustStep] = {}
    for workflow_name, workflow in workflows.items():
        jobs = workflow.get("jobs")
        assert isinstance(jobs, dict), f"{workflow_name}: jobs must be a mapping"
        for job_name, job in jobs.items():
            assert isinstance(job, dict), f"{workflow_name}:{job_name}: job must be a mapping"
            for step in job.get("steps") or ():
                assert isinstance(step, dict), f"{workflow_name}:{job_name}: step must be a mapping"
                run = step.get("run")
                if not isinstance(run, str):
                    continue
                mentions_just = re.search(r"(?<![A-Za-z0-9_.-])just(?![A-Za-z0-9_.-])", run)
                if not mentions_just:
                    continue
                commands = canonical_shell_commands(run)
                assert any(command and command[0] == "just" for command in commands), (
                    f"{workflow_name}:{job_name}: just invocation is hidden behind shell indirection"
                )
                step_name = step.get("name")
                assert isinstance(step_name, str) and step_name, (
                    f"{workflow_name}:{job_name}: every just step needs a stable name"
                )
                key = (workflow_name, str(job_name), step_name)
                assert key not in found, f"duplicate just enforcement step: {key}"
                found[key] = _ObservedJustStep(commands, step.get("if"))
    return found


def assert_required_just_steps(
    workflows: Mapping[str, Mapping[str, Any]],
    required: tuple[RequiredJustStep, ...],
) -> None:
    """Require the declared ``just`` inventory without fail-open shell policy."""
    expected = {(item.workflow, item.job, item.step): item for item in required}
    assert len(expected) == len(required), "required just-step inventory contains duplicates"

    actual = _workflow_just_steps(workflows)
    assert actual.keys() == expected.keys(), (
        "workflow just-step inventory drifted; classify every added/removed enforcement step: "
        f"missing={sorted(expected.keys() - actual.keys())}, "
        f"unexpected={sorted(actual.keys() - expected.keys())}"
    )

    for key, item in expected.items():
        assert_unmasked_step(
            item.workflow,
            workflows[item.workflow],
            item.job,
            item.step,
            condition=item.condition,
        )
        expected_commands = tuple(
            command for source in item.commands for command in canonical_shell_commands(source)
        )
        observed = actual[key]
        assert observed.commands == expected_commands, (
            f"{':'.join(key)} must run only its exact mandatory commands; "
            f"expected={expected_commands!r}, actual={observed.commands!r}"
        )
