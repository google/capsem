"""Lint and type-check every line of Python in the repository.

`ruff check .` always covered the whole tree. `ty` did not: it ran on
`src/capsem` and nothing else, so `scripts/` -- which is release machinery, not
scratch -- and every test helper went unchecked. A type error in
`scripts/release-binaries.py` is a release bug; it had no gate at all.

Which trees are checked, which are checked strictly, and which rules are held
back on the rest are all `[lint]` in `config/gate.toml`. `src/` passes every ty
rule and is checked with none disabled; the other trees hold back the
`ty_ratchet` list -- roughly four hundred diagnostics dominated by inference
over untyped fixture data, which would otherwise force the choice between
checking those trees loosely and not checking them at all. That is the choice
that left them unchecked. Entries may leave the ratchet; nothing may join it.
"""

from __future__ import annotations

import argparse

from . import config as gate_config
from .errors import GateError
from .proc import Runner


def check(runner: Runner) -> None:
    """Run every Python source gate, reporting all failures rather than the first.

    A lint run that stops at the first tool leaves the second one's findings
    for the next push, which is how a gate takes three rounds to go green.
    """
    settings = gate_config.for_root(runner.root).lint
    failures: list[str] = []

    runner.step("ruff")
    if runner.run(["uv", "run", "ruff", "check", "."], check=False) != 0:
        failures.append("ruff")

    present = [
        name for name in settings.python_roots if (runner.root / name).is_dir()
    ]
    strict = [name for name in present if name in settings.strict_roots]
    relaxed = [name for name in present if name not in settings.strict_roots]

    if strict:
        runner.step(f"ty ({', '.join(strict)}, every rule)")
        if runner.run(
            ["uv", "run", "ty", "check", *settings.ty_flags, *strict], check=False
        ) != 0:
            failures.append(f"ty ({', '.join(strict)})")

    if relaxed:
        runner.step(f"ty ({', '.join(relaxed)})")
        held_back = [flag for rule in settings.ty_ratchet for flag in ("--ignore", rule)]
        if runner.run(
            ["uv", "run", "ty", "check", *settings.ty_flags, *relaxed, *held_back],
            check=False,
        ) != 0:
            failures.append(f"ty ({', '.join(relaxed)})")

    if failures:
        raise GateError(f"Python source gates failed: {', '.join(failures)}")


def register(subparsers: argparse._SubParsersAction) -> None:
    parser = subparsers.add_parser(
        "lint", help="ruff and ty over every first-party Python tree"
    )
    parser.set_defaults(handler=_command)


def _command(args: argparse.Namespace, runner: Runner) -> int:
    check(runner)
    return 0
