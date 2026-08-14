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

from . import config as gate_config
from .actions import Call
from .command import GateCommand
from .errors import GateError
from .execution import Kind, Needs, Speed, Step, step
from .opacity import CallJustification, Effect, OpaqueKind, machine_effects
from .plan import Plan
from .proc import Runner
from .sourcecommit import SourceCommit


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

    def reclaim(self, resource: str, *, keep: str) -> None:
        """Retire the superseded tags of a repository keyed by content.

        `keep` is passed in rather than derived downstream: the caller is what
        knows which tag the current inputs resolve to, and a second derivation
        inside the policy script could disagree with it while holding the
        delete button.
        """
        self._runner.script(
            self._config.policy_script, "reclaim", "--resource", resource, "--keep", keep
        )

    def gc(self, *, rail: str | None = None, best_effort: bool = False) -> None:
        args = ["gc"] + (["--rail", rail] if rail else [])
        self._runner.script(self._config.policy_script, *args, check=not best_effort)

    def clean(self, *, scope: str, rail: str | None = None) -> None:
        args = ["clean", "--scope", scope] + (["--rail", rail] if rail else [])
        self._runner.script(self._config.policy_script, *args)

    def capture_failure(
        self,
        *,
        rail: str,
        label: str,
        run_id: str | None = None,
        source_commit: SourceCommit | None = None,
    ) -> None:
        """Preserve evidence from a failed run.

        Never raises: it runs on the failure path, where a second failure would
        replace the first one the operator actually needs to read.
        """
        identity = []
        if run_id is not None:
            identity.extend(("--run-id", run_id))
        if source_commit is not None:
            identity.extend(("--source-commit", str(source_commit)))
        self._runner.script(
            self._config.policy_script,
            "capture-failure",
            "--rail",
            rail,
            "--label",
            label,
            *identity,
            check=False,
        )

    def ensure_space(self, rail: str, *boundary: str) -> None:
        """Refuse to start work the daemon does not have room to finish."""
        self._runner.run(
            ["bash", str(self._runner.root / self._config.ensure_space_script), rail, *boundary]
        )


def release_action(phase: str) -> Call:
    """Hand back the storage a finished rail was holding.

    The action, so a caller that wants it inside a step of its own -- the
    candidate's storage-budget step bundles it with a capacity check -- uses
    the same spelling as `release_step` rather than writing a second one.
    """
    return Call(
        f"release the storage held after {phase}",
        lambda ctx: Storage(ctx.runner).release(phase),
        justification=CallJustification(
            kind=OpaqueKind.RUNTIME_DERIVED,
            reason="which rails a boundary releases is resolved from the storage policy at run time",
            effects=machine_effects(Effect.PROCESS, Effect.HOST_STATE),
        ),
    )


def release_step(config, phase: str) -> Step:
    """Hand back the storage a finished rail was holding.

    Composed rather than dispatched. The modules that mark these boundaries run
    inside a held machine lock, so `capsem-gate storage release` from a plan
    action was a child waiting for the lock its own parent held -- and the
    boundary it recorded landed in the child's run log rather than the gate's.
    """
    return step(
        f"storage.{phase}",
        release_action(phase),
        contends=(config.exclusive("docker_daemon"),),
        kind=Kind.STATIC_TEST,
        needs=frozenset({Needs.DISK}),
        speed=Speed.FAST,
    )


class StorageCommand(
    GateCommand,
    name="storage",
    help="release, collect, or clear gate-owned Docker storage",
):
    """Four operations on one resource, so they stay one command.

    Splitting them into four top-level names would read as four unrelated
    things; they are four points on one budget, and the justfile calls them
    that way.
    """

    exclusive = True

    @classmethod
    def add_arguments(cls, parser) -> None:
        actions = parser.add_subparsers(dest="action", required=True)

        release = actions.add_parser("release", help="release storage a finished rail held")
        release.add_argument("phase")

        collect = actions.add_parser("gc", help="prune stopped containers and dangling images")
        collect.add_argument("--rail")

        clean = actions.add_parser("clean", help="deep cleanup for a cold rebuild")
        clean.add_argument("--scope", required=True)
        clean.add_argument("--rail")

        space = actions.add_parser("ensure-space", help="refuse work the daemon cannot finish")
        space.add_argument("rail")
        space.add_argument("boundary", nargs="?")

    def plan(self) -> Plan:
        action = self._args.action
        plan = Plan(f"{self.name} {action}")
        plan.add(
            step(
                action,
                Call(
                    f"storage {action}",
                    self._operation(action),
                    justification=CallJustification(
                        kind=OpaqueKind.RUNTIME_DERIVED,
                        reason="which rails a boundary releases is resolved from the storage policy at run time",
                        effects=machine_effects(Effect.PROCESS, Effect.HOST_STATE),
                    ),
                ),
                contends=(self._config.exclusive("docker_daemon"),),
                kind=Kind.STATIC_TEST,
                needs=frozenset({Needs.DISK}),
                speed=Speed.FAST,
            )
        )
        return plan

    def _operation(self, action: str):
        args = self._args
        if action == "release":
            return lambda ctx: Storage(ctx.runner).release(args.phase)
        if action == "gc":
            return lambda ctx: Storage(ctx.runner).gc(rail=args.rail)
        if action == "clean":
            return lambda ctx: Storage(ctx.runner).clean(scope=args.scope, rail=args.rail)
        boundary = (args.boundary,) if args.boundary else ()
        return lambda ctx: Storage(ctx.runner).ensure_space(args.rail, *boundary)
