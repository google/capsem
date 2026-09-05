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
from collections.abc import Iterator
from contextlib import contextmanager

from . import snapshot
from .cachetooling import CompilerCache
from .config import GateConfig
from .context import Context
from .errors import GateError
from .fileactions import RefreshSourceTimes
from .lifecycle import Resource, held
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


@contextmanager
def locked(config: GateConfig, runner: Runner, name: str, *, exclusive: bool) -> Iterator[tuple[Resource, ...]]:
    """Refresh compiler inputs after queueing, before observing immutable source."""
    if not exclusive:
        yield ()
        return
    before = None if runner.observing else snapshot.digest(config.root, config)
    with held(ExclusiveLock.for_gate(config, purpose=purpose(name))) as acquired:
        if before is not None and snapshot.digest(config.root, config) != before:
            raise GateError("source changed while waiting for the machine lock; start a fresh run")
        RefreshSourceTimes(config.root, config.boundary.rust.suffixes).perform(
            Context(runner, config, observing=runner.observing)
        )
        yield acquired


def holdings(
    config: GateConfig,
    runner: Runner,
    name: str,
    *,
    exclusive: bool,
    declared: tuple[Resource, ...],
) -> tuple[Resource, ...]:
    """Tooling and declared resources, inside the machine lock's outer scope."""
    if not exclusive:
        return declared
    return (
        CompilerCache(config, runner),
        *declared,
    )


def purpose(name: str) -> str:
    """What contention should call this, for whoever arrives next."""
    return f"capsem-gate {name}"
