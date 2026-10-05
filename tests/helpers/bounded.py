"""Machine work started by a test takes the machine lock, as every direct
command does.

A fixture that ran a bare `cargo build` compiled outside the lock: run from a
plain `pytest`, it rebuilt the shared `cache/target/cargo/debug` binaries while
a gate run elsewhere was booting VMs from them, and that run tested another
checkout's service. Through the bounded wrapper it queues behind a running
gate instead; inside a gate run the wrapper sees the gate's marker and runs
at once, since the lock is already held.
"""

from __future__ import annotations

import sys
from collections.abc import Mapping, Sequence

from helpers.constants import PROJECT_ROOT

WRAPPER = PROJECT_ROOT / "build_system" / "scripts" / "ci" / "run-bounded-command.py"


def bounded(
    argv: Sequence[str], timeout_seconds: int, *, env: Mapping[str, str] | None = None,
) -> list[str]:
    """Bound `argv`, applying explicit probe settings after cache containment.

    Pass only fixture-owned overrides, never the inherited environment: the
    wrapper still owns cache defaults and the genuine parent-run marker.
    `env NAME=value cargo ...` remains visible to its machine-lease predicate.
    """
    if env:
        argv = ["env", *(f"{key}={value}" for key, value in env.items()), *argv]
    return [sys.executable, str(WRAPPER), "--timeout-seconds", str(timeout_seconds), "--", *argv]
