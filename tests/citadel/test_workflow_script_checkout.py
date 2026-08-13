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

import yaml
from helpers.workflow_contract import canonical_shell_commands, direct_script_paths

PROJECT_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = PROJECT_ROOT / ".github" / "workflows"
CHECKOUT_ACTION = "actions/checkout@"


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
