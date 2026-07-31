"""Docker seen as a disk budget rather than as containers.

Eleven recipes spelled
`uv run python scripts/docker-storage-policy.py release --boundary X --rail Y`
by hand, so the set of legal (boundary, rail) pairs existed only as a habit
spread across the justfile. A typo produced a release that silently did nothing
for the rail it was supposed to free, and the next build failed on ENOSPC
somewhere unrelated.

`config/gate.toml` makes that set a table under `[storage.phases]`. An unknown
phase now fails by name, before any storage is touched.
"""

from __future__ import annotations

import argparse

from . import config as gate_config
from .errors import GateError
from .proc import Runner


class Storage:
    """The gate's side of `scripts/docker-storage-policy.py`."""

    def __init__(self, runner: Runner) -> None:
        self._runner = runner
        self._config = gate_config.for_root(runner.root).storage

    def release(self, phase: str, *, best_effort: bool = False) -> None:
        """Release storage held for a rail that has finished."""
        try:
            named = self._config.phases[phase]
        except KeyError:
            raise GateError(
                f"unknown storage release phase {phase!r}; expected one of "
                f"{', '.join(sorted(self._config.phases))}"
            ) from None
        self._runner.script(
            self._config.policy_script,
            "release",
            "--boundary",
            named.boundary,
            "--rail",
            named.rail,
            check=not best_effort,
        )

    def gc(self, *, rail: str | None = None, best_effort: bool = False) -> None:
        args = ["gc"] + (["--rail", rail] if rail else [])
        self._runner.script(self._config.policy_script, *args, check=not best_effort)

    def clean(self, *, scope: str, rail: str | None = None) -> None:
        args = ["clean", "--scope", scope] + (["--rail", rail] if rail else [])
        self._runner.script(self._config.policy_script, *args)

    def capture_failure(self, *, rail: str, label: str) -> None:
        """Preserve evidence from a failed run.

        Never raises: it runs on the failure path, where a second failure would
        replace the first one the operator actually needs to read.
        """
        self._runner.script(
            self._config.policy_script,
            "capture-failure",
            "--rail",
            rail,
            "--label",
            label,
            check=False,
        )

    def ensure_space(self, rail: str, *boundary: str) -> None:
        """Refuse to start work the daemon does not have room to finish."""
        self._runner.run(
            ["bash", str(self._runner.root / self._config.ensure_space_script),
             rail, *boundary]
        )


def register(subparsers: argparse._SubParsersAction) -> None:
    storage = subparsers.add_parser(
        "storage", help="release, collect, or clear gate-owned Docker storage"
    )
    actions = storage.add_subparsers(dest="action", required=True)

    release = actions.add_parser("release", help="release storage a finished rail held")
    release.add_argument("phase")
    release.set_defaults(handler=_release_command)

    gc = actions.add_parser("gc", help="prune stopped containers and dangling images")
    gc.add_argument("--rail")
    gc.set_defaults(handler=_gc_command)

    clean = actions.add_parser("clean", help="deep cleanup for a cold rebuild")
    clean.add_argument("--scope", required=True)
    clean.add_argument("--rail")
    clean.set_defaults(handler=_clean_command)

    space = actions.add_parser("ensure-space", help="refuse work the daemon cannot finish")
    space.add_argument("rail")
    space.add_argument("boundary", nargs="?")
    space.set_defaults(handler=_ensure_space_command)


def _release_command(args: argparse.Namespace, runner: Runner) -> int:
    Storage(runner).release(args.phase)
    return 0


def _gc_command(args: argparse.Namespace, runner: Runner) -> int:
    Storage(runner).gc(rail=args.rail)
    return 0


def _clean_command(args: argparse.Namespace, runner: Runner) -> int:
    Storage(runner).clean(scope=args.scope, rail=args.rail)
    return 0


def _ensure_space_command(args: argparse.Namespace, runner: Runner) -> int:
    boundary = (args.boundary,) if args.boundary else ()
    Storage(runner).ensure_space(args.rail, *boundary)
    return 0
