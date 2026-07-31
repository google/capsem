"""Docker seen as a disk budget rather than as containers.

Eleven recipes spelled
`uv run python scripts/docker-storage-policy.py release --boundary X --rail Y`
by hand, so the set of legal (boundary, rail) pairs existed only as a habit
spread across the justfile. A typo produced a release that silently did nothing
for the rail it was supposed to free, and the next build failed on ENOSPC
somewhere unrelated.

`RELEASE_PHASES` makes that set a table. An unknown phase now fails by name,
before any storage is touched.
"""

from __future__ import annotations

import argparse

from .errors import GateError
from .proc import Runner


POLICY_SCRIPT = "scripts/docker-storage-policy.py"
ENSURE_SPACE_SCRIPT = "scripts/ensure-docker-space.sh"

# Points in the gate at which storage held for a finished rail may be released,
# and the rail whose headroom that release is serving.
RELEASE_PHASES: dict[str, tuple[str, str]] = {
    "completed-linux-rust-target": ("after-linux-rust", "assets"),
    "completed-docker-rails": ("after-assets", "package"),
    "completed-buildkit-graph": ("after-packages", "package"),
    "completed-package-arm64": ("after-package-arm64", "install"),
    "completed-package-x86_64": ("after-package-x86_64", "install"),
    "deferred-install-target": ("before-packages", "package"),
    "candidate-boundary": ("candidate-boundary", "default"),
    "after-install": ("after-install", "install"),
}


class Storage:
    """The gate's side of `scripts/docker-storage-policy.py`."""

    def __init__(self, runner: Runner) -> None:
        self._runner = runner

    def release(self, phase: str, *, best_effort: bool = False) -> None:
        """Release storage held for a rail that has finished."""
        try:
            boundary, rail = RELEASE_PHASES[phase]
        except KeyError:
            raise GateError(
                f"unknown storage release phase {phase!r}; "
                f"expected one of {', '.join(sorted(RELEASE_PHASES))}"
            ) from None
        self._runner.script(
            POLICY_SCRIPT,
            "release",
            "--boundary",
            boundary,
            "--rail",
            rail,
            check=not best_effort,
        )

    def gc(self, *, rail: str | None = None, best_effort: bool = False) -> None:
        args = ["gc"] + (["--rail", rail] if rail else [])
        self._runner.script(POLICY_SCRIPT, *args, check=not best_effort)

    def clean(self, *, scope: str, rail: str | None = None) -> None:
        args = ["clean", "--scope", scope] + (["--rail", rail] if rail else [])
        self._runner.script(POLICY_SCRIPT, *args)

    def capture_failure(self, *, rail: str, label: str) -> None:
        """Preserve evidence from a failed run.

        Never raises: it runs on the failure path, where a second failure would
        replace the first one the operator actually needs to read.
        """
        self._runner.script(
            POLICY_SCRIPT,
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
            ["bash", str(self._runner.root / ENSURE_SPACE_SCRIPT), rail, *boundary]
        )


def register(subparsers: argparse._SubParsersAction) -> None:
    storage = subparsers.add_parser(
        "storage", help="release, collect, or clear gate-owned Docker storage"
    )
    actions = storage.add_subparsers(dest="action", required=True)

    release = actions.add_parser("release", help="release storage a finished rail held")
    release.add_argument("phase", choices=sorted(RELEASE_PHASES))
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
