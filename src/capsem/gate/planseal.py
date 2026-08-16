"""Building a plan must not perform it.

A plan describes work. If constructing the description does the work, then
`--dry-run` is not merely incomplete -- it is actively misleading, and the
description can go stale between being assembled and being run. The release
plan captured `git rev-parse HEAD` while being built, so asking what a release
*would* do touched the machine.

Its own module rather than a rule inside `proc`, because it is a rule about
plans and `proc` is about processes. The two met only where the rule had to be
enforced, which is not the same as belonging together.
"""

from __future__ import annotations

import shlex
from collections.abc import Iterator
from contextlib import contextmanager
from contextvars import ContextVar
from typing import TYPE_CHECKING

from .errors import GateError

if TYPE_CHECKING:  # `plan` imports this module's `sealed`, so keep it lazy.
    from .plan import Plan

#: Set while a command is building its plan. Ambient rather than a property of
#: one runner, and that is the whole point: `release.py` built a *fresh*
#: `Runner(config.root)` inside `plan()` to capture `git rev-parse HEAD`, and a
#: seal that swapped the command's own runner never saw it. The dry run printed
#: a real revision while the recording runner observed nothing -- the machine
#: touched, invisibly, by the one operation whose entire value is not touching
#: it. A context variable is reachable from every runner however it was built.
_SEALED: ContextVar[bool] = ContextVar("capsem_gate_plan_sealed", default=False)


@contextmanager
def sealed() -> Iterator[None]:
    """Refuse every invocation for the duration.

    Wrapped around plan construction. A plan describes work; if building the
    description performs it, `--dry-run` is not merely incomplete but actively
    misleading, and the description can go stale before it is executed.
    """
    token = _SEALED.set(True)
    try:
        yield
    finally:
        _SEALED.reset(token)


def _refuse_while_sealed(argv: tuple[str, ...]) -> None:
    if not _SEALED.get():
        return
    raise GateError(
        f"building a plan ran {shlex.join(argv)}. plan() must describe work, "
        f"not perform it, or --dry-run touches the machine. Express the probe "
        f"as a read-only step in the plan instead."
    )


def describe(command) -> Plan:
    """Build the plan with the machine sealed off.

    Ambient rather than a swapped-in runner: a module that builds its own
    `Runner` inside `plan()` -- which `release.py` did, to capture
    `git rev-parse HEAD` -- escapes anything scoped to this instance.
    """
    with sealed():
        return command.plan()
