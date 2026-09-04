"""What a command checks and takes before its plan runs.

Split from `command` at the module ceiling, and along a real seam: none of
this depends on the command object, only on what the command *declared*. Both
answers are policy about starting -- who may take the machine lock, and in
which order things are acquired -- and both were learned from a deadlock
rather than designed.

Not `gatelaunch`, which is a different launch entirely: that one re-execs
under a private bytecode cache before this package is imported at all.
"""

from __future__ import annotations

import os

from .cachetooling import CompilerCache
from .config import GateConfig
from .errors import GateError
from .lifecycle import Resource
from .locks import ExclusiveLock
from .proc import Runner


def refuse_inside_a_run(config: GateConfig, name: str, *, exclusive: bool) -> None:
    """Refuse to take a lock this process tree is already holding.

    Only for commands that take it. A read-only command is exactly what
    someone wants from inside a running gate -- `runs last` while it works is
    the point of `runs last`.

    `GuardedRunner` cannot see this one: nothing is spawned. A pytest step
    calling `cli.main(["storage", ...])` simply blocked on the lock its own
    grandparent held, and the run stayed alive-looking for two hours.
    """
    if not exclusive:
        return
    holder = os.environ.get(config.locks.gate.run_marker)
    if holder is None:
        return
    raise GateError(
        f"{name} takes the machine lock, and this process is already "
        f"inside the gate run holding it ({holder}). It would wait out its "
        "full timeout for a lock that cannot be released until it returns. "
        "Compose this command's fragment into that plan, or drive its plan "
        "directly if this is a test."
    )


def holdings(
    config: GateConfig,
    runner: Runner,
    name: str,
    *,
    exclusive: bool,
    declared: tuple[Resource, ...],
) -> tuple[Resource, ...]:
    """The machine lock first, then whatever the command declared.

    First because it is released last, and because the resources a command
    declares are the ones that wipe trees -- taking the lock after one of
    those has started is taking it too late.
    """
    if not exclusive:
        return declared
    return (
        ExclusiveLock.for_gate(config, purpose=purpose(name)),
        CompilerCache(config, runner),
        *declared,
    )


def purpose(name: str) -> str:
    """What contention should call this, for whoever arrives next."""
    return f"capsem-gate {name}"
