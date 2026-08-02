"""Give back everything the gate can take.

Scattered across the justfile there were four ways to reclaim something and no
way to reclaim all of it: `_docker-gc`, `_clean-host-image`,
`_clean-docker-test-targets`, and whatever `rm -rf` a recipe happened to run on
its way in. A developer whose disk was full had to know which one to reach for.

One command, three depths. The default clears the trees the gate creates. The
aggressive form also releases the Docker rails, which are expensive to rebuild
and therefore not something to discard by accident.
"""

from __future__ import annotations

from .actions import Action, Call
from .command import GateCommand
from .context import Context
from .disk import footprint, reclaim
from .execution import step
from .plan import Plan
from .runhistory import free_gb
from .storage import Storage

_GB = 1024**3


class GcCommand(
    GateCommand, name="gc", help="reclaim the disk the gate is holding"
):
    exclusive = True

    def should_record(self) -> bool:
        """`--dry-run` surveys; anything else deletes.

        This was `records = False` with "only reads runs" copied from the run
        readers. A normal gc reclaims whole trees, so a failed or partial one
        left terminal output and nothing durable -- while every other mutating
        command in the gate is locked, visible and timed.
        """
        return not self._args.dry_run

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument(
            "--aggressive",
            action="store_true",
            help="also release the Docker rails and build cache",
        )

    def plan(self) -> Plan:
        plan = Plan(self.name)

        if self._args.dry_run:
            # A dry run of a reclaimer should say what it would free, which is
            # more useful than the argv it would run.
            #
            # The survey is an `Action` rather than a string computed here: a
            # plan must describe without doing, and walking the disk while the
            # description is being built is doing. `render` measures because
            # that *is* the description -- and it is read-only, which is the
            # distinction the seal is about.
            plan.add(step("survey", _Survey(self._config)))
            return plan

        trees = plan.add(step("trees", Call("reclaim the gate's own trees", _trees)))
        if self._args.aggressive:
            plan.add(
                step(
                    "rails",
                    Call("release the Docker rails and build cache", _rails),
                    contends=(self._config.exclusive("docker_daemon"),),
                ),
                after=(trees,),
            )
        return plan


class _Survey(Action, name="survey"):
    """What the reclaimer would free, measured when asked."""

    def __init__(self, config) -> None:
        self._config = config

    def render(self) -> str:
        return _measure(self._config)

    def perform(self, context: Context) -> None:
        """Nothing: the answer is the description."""


def _measure(config) -> str:
    measured = footprint(config)
    if not measured:
        return "nothing to reclaim"
    lines = [
        f"{size / _GB:>8.2f} GB  {relative}"
        for relative, size in sorted(measured.items(), key=lambda e: -e[1])
    ]
    total = sum(measured.values()) / _GB
    return "would reclaim:\n          " + "\n          ".join(
        [*lines, f"{total:>8.2f} GB  total"]
    )


def _trees(context: Context) -> None:
    recovered = reclaim(context.config)
    context.runner.note(
        f"reclaimed {recovered.gb_freed:.2f} GB from "
        f"{len(recovered.trees)} trees; {recovered.free_after_gb:.1f} GB free"
    )


def _rails(context: Context) -> None:
    storage = Storage(context.runner)
    storage.clean(scope="all")
    context.runner.note(f"{free_gb(context.config.root):.1f} GB free after the rails")
