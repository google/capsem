"""Direct machine work queues on the gate's lock instead of on luck.

`ExclusiveLock` coordinated `capsem-gate` processes and nothing else, while the
bounded-command wrapper -- the one sanctioned way to run a direct command --
sent every worktree's cargo into the shared target directory and onto every
core. `[locks.bounded]` in `config/gate.toml` records what that cost. The
wrapper is the choke point, so the lease is taken here.

Nothing about this is new machinery: it is the same kernel lock, the same
holder record and the same run marker, taken by one more kind of holder.
"""

from __future__ import annotations

import shlex
import sys
from collections.abc import Iterator, Mapping, Sequence
from contextlib import contextmanager
from pathlib import Path, PurePath

from ..cache.config import load_paths, load_policy
from ..cache.enforcement import EnforcementResult, enforce_repository
from . import config as gate_config
from .errors import GateError
from .lifecycle import held
from .locks import ExclusiveLock
from .lockschema import BoundedLeaseConfig


def machine_work(command: Sequence[str], settings: BoundedLeaseConfig) -> bool:
    """Whether `command` runs a leased program, seen through its wrappers.

    Position, not substring: `cargo` in a path or an argument is not cargo in
    command position, and `env RUST_LOG=debug cargo build` is.
    """
    tokens = list(command)
    while tokens and (PurePath(tokens[0]).name in settings.wrappers or _assignment(tokens[0])):
        tokens.pop(0)
    if not tokens or PurePath(tokens[0]).name not in settings.programs:
        return False
    subcommand = next((token for token in tokens[1:] if not token.startswith("+")), "")
    return subcommand not in settings.exempt_subcommands


def _assignment(token: str) -> bool:
    name, separator, _ = token.partition("=")
    return bool(separator) and name.isidentifier()


def _to_stderr(message: str) -> None:
    print(message, file=sys.stderr, flush=True)


@contextmanager
def leased(
    command: Sequence[str], root: Path, inherited: Mapping[str, str]
) -> Iterator[dict[str, str]]:
    """Hold the machine for `command` if it needs it; yield what to export.

    A command already inside a run holds nothing: the lock is not reentrant,
    and its parent has the machine for it.
    """
    config = gate_config.load(root)
    if inherited.get(config.locks.gate.run_marker) or not machine_work(command, config.locks.bounded):
        yield {}
        return
    lock = ExclusiveLock.for_gate(
        config, purpose=f"bounded: {shlex.join(command)}", announce=_to_stderr
    )
    with held(lock):
        _enforce_cargo_cache(root, command)
        yield lock.environment()


def _enforce_cargo_cache(root: Path, command: Sequence[str]) -> None:
    """Apply the shared Cargo target contract before direct machine work.

    Gate plans expose the same operation as a timed prerequisite. Direct Cargo
    has no plan, so the mandatory bounded-command choke point owns this half of
    the invariant while it holds the same machine lock as a gate.
    """
    policy = load_policy(root)
    paths = load_paths(root)
    result = enforce_repository(
        paths,
        policy,
        "cargo",
        reason=f"bounded direct command: {shlex.join(command)}",
    )
    if result.violations:
        raise GateError("; ".join(result.violations))
    if result.pruned:
        _to_stderr(_maintenance_notice(result))


def _maintenance_notice(result: EnforcementResult) -> str:
    return (
        f"Cargo cache maintenance applied {result.action_count} prune actions; "
        f"owned usage {result.before_size_bytes} -> {result.after_size_bytes} bytes"
    )
