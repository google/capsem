"""Argument parsing and dispatch. Nothing decides anything here.

Every command is contributed by the module that implements it, by subclassing
`GateCommand`, so this file never grows a branch about what a command means. If
a rule appears here, it is in the wrong file --
`test_the_cli_only_parses_and_dispatches` in `tests/test_gate_boundary.py`
says so.

The three inspection flags are declared once, on a shared parent parser, so
every command has them by construction rather than by each author remembering.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from collections.abc import Sequence
from pathlib import Path

from . import (
    assetplan,
    cancellation,
    candidate,
    crosscompile,
    debproofcommand,
    devloop,
    doctor,
    gc,
    guestcommands,
    hostimage,
    hostpackage,
    imagebuild,
    initrd,
    install,
    installimage,
    lint,
    linuxrustimage,
    module_contracts,
    project_root,
    release,
    runs,
    sandbox,
    service,
    smoke,
    staticmodule,
    storage,
    testmodules,
    toolchaincommands,
    versions,
    vmmodules,
)
from .command import GateCommand
from .errors import GateError
from .proc import Runner

# Imported for the registration their subclasses perform. Named rather than
# star-imported so the set is visible, and so removing one is a decision.
COMMAND_MODULES = (
    versions,
    candidate,
    assetplan,
    storage,
    installimage,
    install,
    crosscompile,
    debproofcommand,
    lint,
    doctor,
    runs,
    gc,
    staticmodule,
    testmodules,
    module_contracts,
    vmmodules,
    imagebuild,
    linuxrustimage,
    toolchaincommands,
    hostimage,
    hostpackage,
    service,
    smoke,
    initrd,
    release,
    devloop,
    guestcommands,
)


def _inspection() -> argparse.ArgumentParser:
    """Flags every command shares, so none of them can lack one."""
    shared = argparse.ArgumentParser(add_help=False)
    shared.add_argument(
        "--dry-run",
        action="store_true",
        help="print what would run, in order, without running it",
    )
    shared.add_argument(
        "--graph",
        action="store_true",
        help="print the step graph as a mermaid diagram",
    )
    shared.add_argument(
        "--prefix",
        default=None,
        metavar="PATH",
        help="work in this existing private checkout instead of making one",
    )
    shared.add_argument(
        "--from",
        dest="resume_from",
        default=None,
        metavar="STEP",
        help="carry every step before STEP and start there (never in a release)",
    )
    shared.add_argument(
        "--sandbox",
        type=sandbox.SandboxMode,
        choices=tuple(sandbox.SandboxMode),
        default=None,
        metavar="MODE",
        help=(
            "run under the host kernel sandbox: `enforce` uses Linux Bubblewrap "
            "or macOS Seatbelt; `report` is macOS-only. Defaults to the command."
        ),
    )
    shared.add_argument(
        "--timing",
        action="store_true",
        help="compatibility flag; recorded commands always print their timing",
    )
    return shared


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="capsem-gate",
        description="Build and release gate operations invoked by the justfile.",
    )
    # `gate_command`, not `command`: a subcommand storing its own positional in
    # `command` overwrote the subcommand name, and registry lookup then indexed
    # a dict with a list. `exec` did exactly that, and could not dispatch at
    # all. A guard in `tests/test_gate_exec_boundary.py` keeps the slot free.
    subparsers = parser.add_subparsers(dest="gate_command", required=True)
    shared = _inspection()

    for name, command in sorted(GateCommand.registry.items()):
        child = subparsers.add_parser(name, help=command.help, parents=[shared])
        command.add_arguments(child)

    return parser


def invocation(argv: Sequence[str] | None = None) -> tuple[str, ...]:
    """What was typed, kept before anything interprets it.

    Not reconstructed from the parsed namespace: reconstruction loses ordering,
    repeated flags, `--`, and the difference between an omitted option and one
    passed at its default. The previous attempt looked for a namespace field
    named `argv` that almost no command declares, so `release-binaries nightly`
    was recorded as `('release-binaries',)` and a failed release could not say
    which channel it had tried.
    """
    return ("capsem-gate", *(sys.argv[1:] if argv is None else argv))


# The one gate path not read from `config/gate.toml`, because the failure being
# recorded may be that the config could not be read at all -- which is exactly
# the failure that went unrecorded. Held equal to `[runlog].root` by
# `tests/citadel/test_startup_record_matches_the_run_log_root.py`.
STARTUP_RECORD = Path("target") / "gate-runs" / "startup.jsonl"


def record_startup_failure(root: Path | None, raw: tuple[str, ...], error: str) -> None:
    """Leave evidence for a failure that never reached a run directory.

    `[runlog]` promises a failed gate is "a directory you attach to a bug
    rather than a scrollback you had to be present for". Everything before the
    first step escaped that: two `release-profile` invocations died on an
    unparseable config, wrote no run directory and no ledger row, and left the
    digest still reporting the previous run as the last one.

    Appended, never rewritten, and best-effort: failing to record a failure
    must not replace it with a different one.
    """
    if root is None:
        return
    try:
        path = root / STARTUP_RECORD
        path.parent.mkdir(parents=True, exist_ok=True)
        row = {
            "schema": "capsem.gate.startup.v1",
            "ts": time.time(),
            "invocation": list(raw),
            "status": "failed",
            "error": error,
        }
        with path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(row) + "\n")
            handle.flush()
            os.fsync(handle.fileno())
    except OSError:
        return


def main(argv: Sequence[str] | None = None) -> int:
    raw = invocation(argv)
    args = build_parser().parse_args(argv)
    root: Path | None = None
    command: GateCommand | None = None

    def unrecorded(message: str) -> None:
        if command is None or not command.recorded:
            record_startup_failure(root, raw, message)

    try:
        root = project_root()
        with cancellation.unwind_sigterm():
            command = GateCommand.registry[args.gate_command](Runner(root), args, invocation=raw)
            command.execute()
    except GateError as exc:
        unrecorded(str(exc))
        print(f"ERROR: {exc}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        unrecorded("interrupted")
        print("interrupted", file=sys.stderr)
        return 130
    except cancellation.Terminated as exc:
        unrecorded(str(exc))
        print(str(exc), file=sys.stderr)
        return exc.exit_status
    return 0


if __name__ == "__main__":  # pragma: no cover - exercised through the console script
    raise SystemExit(main())
