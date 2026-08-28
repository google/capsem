"""Citadel guard: workflow scripts run only after their repository exists.

GitHub jobs start with an empty workspace. Moving a gate body from inline YAML
to ``scripts/`` makes it lintable and testable, but also creates a runtime
dependency on checkout in that *same job*. The branch-protection aggregator
once made exactly that move without adding checkout; every PR would have died
with ``scripts/require-ci-jobs.sh: No such file or directory`` before judging
any dependency result.
"""

from __future__ import annotations

from enum import StrEnum
from pathlib import Path

import pytest
import yaml
from helpers.workflow_contract import canonical_shell_commands, direct_script_paths

PROJECT_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = PROJECT_ROOT / ".github" / "workflows"
CHECKOUT_ACTION = "actions/checkout@"
TOOL_ACTIONS = {
    "uv": ("astral-sh/setup-uv@",),
    "pnpm": ("pnpm/action-setup@", "actions/setup-node@"),
    "just": ("extractions/setup-just@",),
}

PROVISIONING_RATIONALE = """\
A workflow command may use a repository tool only after that job provisions it.
Jobs are isolated, and moving a script does not move its runtime onto PATH. A
checkout supplies source; setup-uv, pnpm plus Node, and setup-just supply the
locked command interpreters. The setup must be earlier and unconditional in the
same job, or the workflow is green only on a runner with accidental ambient state.
"""


class Key(StrEnum):
    """The workflow vocabulary this guard reads."""

    JOBS = "jobs"
    STEPS = "steps"
    USES = "uses"
    RUN = "run"
    NAME = "name"
    IF = "if"
    CONTINUE_ON_ERROR = "continue-on-error"


def _is_unconditional_checkout(step: dict) -> bool:
    uses = step.get(Key.USES)
    return (
        isinstance(uses, str)
        and uses.startswith(CHECKOUT_ACTION)
        and step.get(Key.IF) is None
        and step.get(Key.CONTINUE_ON_ERROR) is not True
    )


def _unconditional_action(step: dict) -> str | None:
    uses = step.get(Key.USES)
    if (
        isinstance(uses, str)
        and step.get(Key.IF) is None
        and step.get(Key.CONTINUE_ON_ERROR) is not True
    ):
        return uses
    return None


def _uses_tool(command: tuple[str, ...], tool: str) -> bool:
    return any(token == tool or token.endswith(f"/{tool}") for token in command)


def _tool_provisioning_offenders(documents: dict[str, dict]) -> list[str]:
    offenders: list[str] = []
    for workflow_name, document in sorted(documents.items()):
        for job_name, job in (document.get(Key.JOBS) or {}).items():
            if not isinstance(job, dict):
                continue
            actions: set[str] = set()
            for step in job.get(Key.STEPS) or ():
                if not isinstance(step, dict):
                    continue
                if action := _unconditional_action(step):
                    actions.add(action)
                body = step.get(Key.RUN)
                if not isinstance(body, str):
                    continue
                for command in canonical_shell_commands(body):
                    for tool, required in TOOL_ACTIONS.items():
                        if not _uses_tool(command, tool):
                            continue
                        missing = [
                            prefix
                            for prefix in required
                            if not any(action.startswith(prefix) for action in actions)
                        ]
                        if missing:
                            name = step.get(Key.NAME, "<unnamed>")
                            offenders.append(
                                f"{workflow_name}:{job_name}:{name}: {tool} missing {missing}"
                            )
    return offenders


def _workflow_documents() -> dict[str, dict]:
    return {
        path.name: yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        for path in sorted(WORKFLOWS.glob("*.yaml"))
    }


def test_repository_scripts_have_an_earlier_checkout_in_the_same_job() -> None:
    """A script present in another job or in the source tree is not present here."""
    offenders: list[str] = []
    dispatches = 0
    for workflow_path in sorted(WORKFLOWS.glob("*.yaml")):
        document = yaml.safe_load(workflow_path.read_text(encoding="utf-8")) or {}
        for job_name, job in (document.get(Key.JOBS) or {}).items():
            if not isinstance(job, dict):
                continue
            checked_out = False
            for step in job.get(Key.STEPS) or ():
                if not isinstance(step, dict):
                    continue
                checked_out = checked_out or _is_unconditional_checkout(step)
                body = step.get(Key.RUN)
                if not isinstance(body, str):
                    continue
                for command in canonical_shell_commands(body):
                    for script in direct_script_paths(command):
                        dispatches += 1
                        if not checked_out:
                            name = step.get(Key.NAME, "<unnamed>")
                            offenders.append(
                                f"{workflow_path.name}:{job_name}:{name}: {script}"
                            )

    assert dispatches, "no workflow script dispatch found; checkout guard is vacuous"
    assert not offenders, (
        "GitHub jobs have independent empty workspaces. A checked-in script can "
        "run only after an unconditional actions/checkout step in that same job; "
        f"missing or late checkout: {offenders}"
    )


def test_workflow_tools_are_provisioned_before_use_in_the_same_job() -> None:
    offenders = _tool_provisioning_offenders(_workflow_documents())
    assert not offenders, PROVISIONING_RATIONALE + "\n" + "\n".join(offenders)


@pytest.mark.parametrize(
    ("tool", "actions"),
    [
        ("uv", ()),
        ("pnpm", ("pnpm/action-setup@pinned",)),
        ("pnpm", ("actions/setup-node@pinned",)),
        ("just", ()),
    ],
)
def test_each_missing_tool_setup_is_observed_red(
    tool: str, actions: tuple[str, ...]
) -> None:
    steps = [{"uses": action} for action in actions]
    steps.append({"name": "Use tool", "run": f"{tool} verify"})
    documents = {"fixture.yaml": {"jobs": {"build": {"steps": steps}}}}
    assert _tool_provisioning_offenders(documents), PROVISIONING_RATIONALE


def test_late_or_conditional_tool_setup_is_observed_red() -> None:
    documents = {
        "fixture.yaml": {
            "jobs": {
                "build": {
                    "steps": [
                        {"name": "Use tool", "run": "uv run python verify.py"},
                        {"uses": "astral-sh/setup-uv@pinned"},
                        {"uses": "extractions/setup-just@pinned", "if": "false"},
                        {"name": "Use conditional tool", "run": "just verify"},
                    ]
                }
            }
        }
    }
    offenders = _tool_provisioning_offenders(documents)
    assert len(offenders) == 2, PROVISIONING_RATIONALE
