"""Argument parsing and dispatch. Nothing decides anything here.

Every command is contributed by the module that implements it, through a
`register(subparsers)` function, so this file never grows a branch about what a
command means. If a rule appears in this file, it is in the wrong file --
`tests/test_gate_cli_is_thin.py` says so.
"""

from __future__ import annotations

import argparse
import sys
from collections.abc import Sequence

from . import install, installimage, project_root, storage, versions
from .errors import GateError
from .proc import Runner


# Each module owns its own subcommand surface.
COMMAND_MODULES = (versions, storage, installimage, install)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="capsem-gate",
        description="Build and release gate operations invoked by the justfile.",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)
    for module in COMMAND_MODULES:
        module.register(subparsers)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        runner = Runner(project_root())
        return args.handler(args, runner)
    except GateError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("interrupted", file=sys.stderr)
        return 130


if __name__ == "__main__":  # pragma: no cover - exercised through the console script
    raise SystemExit(main())
