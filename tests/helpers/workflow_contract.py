"""Structural contracts for mandatory GitHub workflow commands."""

from __future__ import annotations

import re
import subprocess
import textwrap
from collections.abc import Mapping
from dataclasses import dataclass
from itertools import pairwise
from pathlib import Path
from typing import Any

import yaml
from capsem_builder.gate.shellnodes import Command, commands
from capsem_builder.gate.shellparse import parse as parse_shell
from yaml.nodes import MappingNode, Node, SequenceNode

from .shelltokens import OPERATORS, REDIRECTS, tokenize

_CONTINUATION = re.compile(r"\\\r?\n")
_GITHUB_EXPRESSION = re.compile(r"\$\{\{\s*(.*?)\s*}}")
# `needs.<job>.result`, read only from inside a `${{ }}` expression: the same
# text outside one is prose, and matching it anywhere would make a comment
# authoritative.
_NEEDS_RESULT = re.compile(r"\bneeds\.([A-Za-z0-9_-]+)\.result\b")
_DIRECT_SCRIPT = re.compile(
    r"^(?:scripts|build_system/scripts)/[A-Za-z0-9_./-]+\.(?:sh|py)$"
)


def workflow_document(path: Path) -> dict[str, Any]:
    """Load one workflow as YAML instead of rediscovering its indentation."""
    document = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    assert isinstance(document, dict), f"{path.name}: workflow must be a mapping"
    return document


def workflow_jobs(path: Path) -> Mapping[str, Any]:
    """The workflow's jobs, preserving YAML's declaration order."""
    jobs = workflow_document(path).get("jobs") or {}
    assert isinstance(jobs, dict) and jobs, f"{path.name}: workflow must declare jobs"
    return jobs


def workflow_job(path: Path, name: str) -> dict[str, Any]:
    """One exact workflow job selected by YAML identity."""
    job = workflow_jobs(path).get(name)
    assert isinstance(job, dict), f"{path.name}: missing or invalid job {name!r}"
    return job


def workflow_step(path: Path, job_name: str, step_name: str) -> dict[str, Any]:
    """One exact named step selected from a parsed workflow."""
    matches = [
        step
        for step in workflow_job(path, job_name).get("steps") or ()
        if isinstance(step, dict) and step.get("name") == step_name
    ]
    assert len(matches) == 1, (
        f"{path.name}:{job_name}: expected one step named {step_name!r}, "
        f"found {len(matches)}"
    )
    return matches[0]


def _mapping_value(node: Node, key: str) -> Node | None:
    if not isinstance(node, MappingNode):
        return None
    return next(
        (value for found, value in node.value if getattr(found, "value", None) == key),
        None,
    )


def workflow_job_source(source: str, name: str) -> str:
    """One job's original YAML bytes, selected through the parsed document."""
    document = yaml.compose(source)
    jobs = _mapping_value(document, "jobs") if document is not None else None
    job = _mapping_value(jobs, name) if jobs is not None else None
    assert job is not None, f"workflow is missing job {name!r}"
    selected = source[job.start_mark.index : job.end_mark.index]
    return textwrap.dedent(" " * job.start_mark.column + selected)


def workflow_step_source(job_source: str, name: str) -> str:
    """One step's original YAML bytes, selected by its parsed ``name`` key."""
    job = yaml.compose(job_source)
    steps = _mapping_value(job, "steps") if job is not None else None
    assert isinstance(steps, SequenceNode), f"job has no steps while selecting {name!r}"
    matches = [
        step
        for step in steps.value
        if getattr(_mapping_value(step, "name"), "value", None) == name
    ]
    assert len(matches) == 1, f"expected one step named {name!r}"
    step = matches[0]
    selected = job_source[step.start_mark.index : step.end_mark.index]
    return textwrap.dedent(" " * step.start_mark.column + selected)


def parsed_commands(shell: str, *, origin: str) -> tuple[Command, ...]:
    """Commands in one shell body, independent of quoting and presentation."""
    return tuple(commands(parse_shell(shell, origin=origin)))


def emitted_assignment_names(shell: str, *, origin: str) -> frozenset[str]:
    """Assignment records emitted by ``echo`` or ``printf`` commands.

    GitHub environment steps write ``NAME=value`` records to ``GITHUB_ENV``.
    Reading their shell as text made ``echo`` and ``printf`` different
    contracts. The parser makes the command and its arguments authoritative;
    splitting a printf format's literal ``\\n`` then recovers its records.
    """
    found: set[str] = set()
    for command in parsed_commands(shell, origin=origin):
        if command.program not in {"echo", "printf"}:
            continue
        for argument in command.argv[1:]:
            for record in argument.replace("\\n", "\n").splitlines():
                name, separator, _value = record.partition("=")
                if separator and name and name.replace("_", "a").isalnum():
                    found.add(name)
    return frozenset(found)


def dispatched_script_paths(shell: str, *, origin: str) -> tuple[str, ...]:
    """Tracked scripts reached directly through a shell interpreter."""
    return tuple(
        path
        for command in parsed_commands(shell, origin=origin)
        if command.program in {"bash", "sh"}
        for path in direct_script_paths(command.argv)
    )


_JUST_OPTIONS_WITH_VALUE = frozenset(
    {"--chooser", "--justfile", "--shell", "--shell-arg", "--working-directory", "-f", "-d"}
)


def just_recipe_names(shell: str, *, origin: str) -> tuple[str, ...]:
    """Recipes invoked in shell command position, not comments or arguments."""
    found: list[str] = []
    for command in parsed_commands(shell, origin=origin):
        if command.program != "just":
            continue
        arguments = iter(command.argv[command.argv.index("just") + 1 :])
        for argument in arguments:
            if argument in _JUST_OPTIONS_WITH_VALUE:
                next(arguments, None)
            elif argument == "--set":
                next(arguments, None)
                next(arguments, None)
            elif argument == "--":
                if recipe := next(arguments, None):
                    found.append(recipe)
                break
            elif not argument.startswith("-"):
                found.append(argument)
                break
    return tuple(found)


def direct_script_paths(command: tuple[str, ...]) -> tuple[str, ...]:
    """Tracked repository scripts a simple workflow command dispatches.

    Kept beside the shell tokenizer because both the reachable-source reader
    and the checkout guard need the same answer. A second token scan in the
    guard would eventually disagree about the owned script roots or supported
    suffixes.
    """
    return tuple(
        candidate
        for token in command
        if _DIRECT_SCRIPT.fullmatch(candidate := token.removeprefix("./"))
    )


def workflow_reachable_text(
    root: Path,
    workflow: Path,
    *,
    job: str | None = None,
) -> str:
    """Render workflow steps with directly dispatched tracked scripts inline.

    A contract about what a lane executes must follow a command moved from a
    YAML ``run:`` body into a checked-in script. Each script is inserted after
    the step dispatching it, so ordering assertions keep their meaning. The
    traversal is deliberately one level: this follows a workflow dispatch,
    not an arbitrary shell call graph.
    """
    workflow_source = workflow.read_text(encoding="utf-8")
    document = yaml.safe_load(workflow_source) or {}
    jobs = document.get("jobs") or {}
    selected = jobs if job is None else {job: jobs.get(job)}
    assert all(isinstance(definition, dict) for definition in selected.values()), (
        f"{workflow.name}: missing or invalid job {job!r}"
    )
    tracked = set(
        subprocess.run(
            ["git", "ls-files"],
            cwd=root,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.splitlines()
    )

    rendered: list[str] = [workflow_source] if job is None else []
    for job_name, definition in selected.items():
        assert isinstance(definition, dict)
        if job is not None:
            metadata = {key: value for key, value in definition.items() if key != "steps"}
            rendered.append(yaml.safe_dump({job_name: metadata}, sort_keys=False))
        for step in definition.get("steps") or ():
            assert isinstance(step, dict), f"{workflow.name}:{job_name}: invalid step"
            if job is not None:
                rendered.append(yaml.safe_dump(step, sort_keys=False))
            if not isinstance(step.get("run"), str):
                continue
            for command in canonical_shell_commands(step["run"]):
                for candidate in direct_script_paths(command):
                    assert candidate in tracked, (
                        f"{workflow.name}:{job_name} dispatches untracked {candidate}"
                    )
                    rendered.append((root / candidate).read_text(encoding="utf-8"))
    return "\n".join(rendered)


def workflow_reachable_shell(root: Path, workflow: Path, *, job: str) -> str:
    """A job's run bodies plus directly dispatched tracked scripts.

    Unlike ``workflow_reachable_text``, this is a shell-only subject suitable
    for ``shellparse``. Feeding YAML containing shell to the parser produces a
    plausible tree of the container rather than the program inside it.
    """
    tracked = set(
        subprocess.run(
            ["git", "ls-files"],
            cwd=root,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.splitlines()
    )
    rendered: list[str] = []
    for index, step in enumerate(workflow_job(workflow, job).get("steps") or ()):
        if not isinstance(step, dict) or not isinstance(step.get("run"), str):
            continue
        body = step["run"]
        rendered.append(body)
        for command in parsed_commands(body, origin=f"{workflow.name}:{job}:{index}"):
            for candidate in direct_script_paths(command.argv):
                assert candidate in tracked, (
                    f"{workflow.name}:{job} dispatches untracked {candidate}"
                )
                rendered.append((root / candidate).read_text(encoding="utf-8"))
    return "\n".join(rendered)


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
