"""The Linux builder image is a step, and the two lanes that need it share it.

`installimage.prepare()` and `CrossCompiler._prepare_builder()` both ran
`just _build-host-image`. That recipe does not exist -- the justfile carries its
heading and no body, and `just --show _build-host-image` has been failing for as
long as the calls have been there. Install-image preflight and cross-compilation
were both broken at runtime, which is to say static qualification and the
package lanes were.

Nothing noticed because nothing checked that a name written in Python resolves
to something real, and because the unit tests around both modules stopped at
the recipe boundary rather than crossing it.

The image is `hostimage.image(config)`, a step that already existed. Composed
rather than dispatched, it is built once per plan however many lanes want it.
"""

from __future__ import annotations

import argparse
from pathlib import Path

import pytest
from helpers.gate import RecordingRunner

from capsem.gate import (
    cli,  # noqa: F401 - imported so every command registers
    hostimage,
)
from capsem.gate import config as gate_config
from capsem.gate.command import GateCommand
from capsem.gate.plan import Plan

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)


def _plan(name: str, **args) -> Plan:
    return GateCommand.registry[name](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False, **args),
    )._describe()


@pytest.mark.parametrize(
    ("name", "args"),
    [("install-image", {}), ("cross-compile", {"arch": "arm64"})],
)
def test_the_builder_image_is_a_step_rather_than_a_recipe(name, args) -> None:
    """Not `just _build-host-image`, which has never existed."""
    plan = _plan(name, **args)

    assert hostimage.STEP in plan.labels, (
        f"{name} does not build the host image it depends on: {plan.labels}"
    )


@pytest.mark.parametrize(
    ("name", "args"),
    [("install-image", {}), ("cross-compile", {"arch": "arm64"})],
)
def test_everything_that_needs_the_builder_waits_for_it(name, args) -> None:
    """A lane that builds a package before its builder exists fails late."""
    plan = _plan(name, **args)
    order = list(plan.labels)
    built = order.index(hostimage.STEP)

    assert built == 0 or all(
        order.index(label) > built
        for label in order
        if label.startswith(("install-image", "package."))
    )


def test_two_lanes_in_one_plan_build_the_builder_once() -> None:
    """The diamond `shared` exists for.

    Composed into a candidate plan these lanes both want the image; building a
    six-gigabyte Docker image twice is the waste, and adding it twice is a
    duplicate-label error that would stop the composition outright.
    """
    plan = Plan("composed")
    hostimage.fragment(plan, CONFIG)
    hostimage.fragment(plan, CONFIG)

    assert list(plan.labels).count(hostimage.STEP) == 1


def test_chained_lanes_do_not_make_the_builder_depend_on_them() -> None:
    """Shared groundwork sits before everything, not after its caller.

    The glow-up lane chains architectures so the second package build waits for
    the first to release its disk. Passing that `after` down to the shared
    host-image step made the image depend on a package that depends on the
    image -- a cycle, and one that only appears once two lanes are composed
    into a single plan.

    The builder image has no ordering requirement of its own. Only the package
    that runs inside it does.
    """
    from capsem.gate import crosscompile

    plan = Plan("chained")
    first = crosscompile.fragment(plan, CONFIG, CONFIG.arch("arm64"))
    crosscompile.fragment(plan, CONFIG, CONFIG.arch("x86_64"), after=(first,))

    assert plan.labels, "a cycle would raise before returning any order"
    assert list(plan.labels).count(hostimage.STEP) == 1
