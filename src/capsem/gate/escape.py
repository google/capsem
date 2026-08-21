"""Which runner an action that leaves the kernel sandbox is given.

Its own module because both `actions` and `outside` need it and each is
already at the 300-line ceiling this package enforces on itself, and because
importing it from either of those would close a cycle.
"""

from __future__ import annotations

from .context import Context
from .errors import GateError
from .proc import Runner


def escaping_runner(context: Context, what: str) -> Runner:
    """The runner for an action that declared it must escape the sandbox.

    When a sandbox is in force, that must be the outside runner, and its
    absence is refused: falling back on the sandboxed one is what made
    `outside_sandbox=True` silently mean nothing under a command holding no
    `Egress`, so the action ran inside and failed hours later on something
    that named neither.

    When no sandbox is in force there is nothing to escape, and the ordinary
    runner is already outside. `Egress` is built `enabled=(mode != OFF)`, so an
    unsandboxed command holds a disabled one and has no outside runner by
    construction -- refusing there would fail every such command for the sake
    of a boundary that is not present. That mistake failed both Linux release
    builds twice.
    """
    if context.outside_runner is not None:
        return context.outside_runner
    if not context.sandboxed():
        return context.runner
    raise GateError(
        f"{what} must run outside the kernel sandbox, but this command holds "
        "no egress to run it with: declare `outside_egress` and include "
        "`Egress` in its resources."
    )
