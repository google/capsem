"""The revision a release publishes is the one the gate qualified.

`release.py` captured `git rev-parse HEAD` while *building* its plan, which was
wrong twice over. It ran a command during `--dry-run`, through a runner it
constructed itself so nothing recorded it. And it meant the head was read
before the plan existed, so the value baked into the confirm step came from
whenever the description happened to be built rather than from the run.

The capture is a step now, and the confirmation reads what that step recorded.
The order is what makes a release safe: capture, gate, re-assert. If the head
moves while a forty-minute gate is running, the confirmation must fail rather
than publish a revision nothing tested.
"""

from __future__ import annotations

import argparse
from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import cli  # noqa: F401 - imported so every command registers
from capsem.gate.command import GateCommand

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _command(name: str, **args):
    return GateCommand.registry[name](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False, **args),
    )


def _described(name: str, **args) -> str:
    return _command(name, **args)._describe().describe()


@pytest.mark.parametrize(
    ("name", "args"),
    [
        ("release-binaries", {"channel": "nightly"}),
        ("release-profile", {"channel": "nightly", "profile": "code"}),
    ],
)
def test_a_release_plan_can_be_described_without_running_anything(name, args) -> None:
    """The seal makes this the regression test for the whole class.

    Asking what a release would do must cost nothing and change nothing --
    especially for the one command whose mistakes are public and irreversible.
    """
    assert _described(name, **args)


@pytest.mark.parametrize(
    ("name", "args"),
    [
        ("release-binaries", {"channel": "nightly"}),
        ("release-profile", {"channel": "nightly", "profile": "code"}),
    ],
)
def test_the_head_is_captured_before_the_gate_and_confirmed_after(name, args) -> None:
    """Capture, qualify, re-assert -- and never in another order.

    A release that captured the head *after* the gate would confirm whatever
    the tree had drifted to, which is precisely the thing being guarded
    against.
    """
    plan = _command(name, **args)._describe()
    order = list(plan.labels)

    assert order.index("record-head") < order.index("gate")
    assert order.index("gate") < order.index("confirm-head")


@pytest.mark.parametrize(
    ("name", "args"),
    [
        ("release-binaries", {"channel": "nightly"}),
        ("release-profile", {"channel": "nightly", "profile": "code"}),
    ],
)
def test_no_revision_is_baked_into_the_description(name, args) -> None:
    """A described plan carrying a concrete sha was one read at plan time.

    That value is stale the moment anyone commits, so a dry run printed a
    promise about a revision the eventual run would not be testing.
    """
    import re

    described = _described(name, **args)

    assert not re.search(r"\b[0-9a-f]{40}\b", described), (
        f"a revision was resolved while describing the plan:\n{described}"
    )
