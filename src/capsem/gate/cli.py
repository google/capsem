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
import sys
from collections.abc import Sequence

from . import (
    assets,
    candidate,
    crosscompile,
    debproof,
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
    project_root,
    release,
    runs,
    service,
    smoke,
    storage,
    testmodules,
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
    assets,
    storage,
    installimage,
    install,
    crosscompile,
    debproof,
    lint,
    doctor,
    runs,
    gc,
    testmodules,
    vmmodules,
    imagebuild,
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
        "--timing",
        action="store_true",
        help="print where the time went, by critical path",
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


def main(argv: Sequence[str] | None = None) -> int:
    raw = invocation(argv)
    args = build_parser().parse_args(argv)
    try:
        GateCommand.registry[args.gate_command](
            Runner(project_root()), args, invocation=raw
        ).execute()
    except GateError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("interrupted", file=sys.stderr)
        return 130
    return 0


if __name__ == "__main__":  # pragma: no cover - exercised through the console script
    raise SystemExit(main())
