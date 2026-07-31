"""Lint and type-check every line of Python in the repository.

`ruff check .` always covered the whole tree. `ty` did not: it ran on
`src/capsem` and nothing else, so `scripts/` -- which is release machinery, not
scratch -- and every test helper went unchecked. A type error in
`scripts/release-binaries.py` is a release bug; it had no gate at all.

Both halves now run over `src`, `scripts`, `tests`, and `guest`, at two
strictnesses. `src/` passes every ty rule and is checked with none disabled.
The rest is checked with the `ty_ratchet` list from pyproject held back --
roughly four hundred diagnostics dominated by inference over untyped fixture
data, which would otherwise force the choice between checking those trees
loosely and not checking them at all. That is the choice that left them
unchecked. Entries may leave the ratchet; nothing may join it.
"""

from __future__ import annotations

import argparse
import tomllib
from pathlib import Path

from .errors import GateError
from .proc import Runner

# Every directory in the repository holding first-party Python.
PYTHON_ROOTS = ("src", "scripts", "tests", "guest")

# The subset that already passes every rule, and must keep doing so.
STRICT_ROOTS = ("src",)

# A ty warning exits zero, so a warning-level rule on the ratchet below could
# never have been detected as fixed -- and a suppression comment left behind
# after its diagnostic was fixed would sit
# there forever, describing a problem that no longer exists.
TY_FLAGS = ("--error-on-warning",)


def ratchet(root: Path) -> list[str]:
    """Rules held back outside `src/`, read from the one place they are declared."""
    config = tomllib.loads((Path(root) / "pyproject.toml").read_text(encoding="utf-8"))
    try:
        return list(config["tool"]["capsem"]["gate"]["ty_ratchet"])
    except KeyError:
        raise GateError("pyproject declares no [tool.capsem.gate] ty_ratchet") from None


def _relaxed_roots(root: Path) -> list[str]:
    present = [name for name in PYTHON_ROOTS if (Path(root) / name).is_dir()]
    return [name for name in present if name not in STRICT_ROOTS]


def check(runner: Runner) -> None:
    """Run every Python source gate, reporting all failures rather than the first.

    A lint run that stops at the first tool leaves the second one's findings
    for the next push, which is how a gate takes three rounds to go green.
    """
    failures: list[str] = []

    runner.step("ruff")
    if runner.run(["uv", "run", "ruff", "check", "."], check=False) != 0:
        failures.append("ruff")

    runner.step("ty (strict)")
    strict = [name for name in STRICT_ROOTS if (runner.root / name).is_dir()]
    if runner.run(["uv", "run", "ty", "check", *TY_FLAGS, *strict], check=False) != 0:
        failures.append(f"ty ({', '.join(strict)})")

    relaxed = _relaxed_roots(runner.root)
    if relaxed:
        runner.step(f"ty ({', '.join(relaxed)})")
        held_back = [flag for rule in ratchet(runner.root) for flag in ("--ignore", rule)]
        if (
            runner.run(
                ["uv", "run", "ty", "check", *TY_FLAGS, *relaxed, *held_back],
                check=False,
            )
            != 0
        ):
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
