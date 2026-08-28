"""The development surfaces, and getting a machine ready to run one.

Split out of `release.py`, which owned the two release commands and had
accumulated these as well -- a module claiming one responsibility while
holding two unrelated ones is the clearest kind of drift to correct.

`just dev` selects among these by name rather than building a recipe name out
of the argument it was given, which used to make the *dispatch* attacker-chosen.
"""

from __future__ import annotations

from . import imagebuild
from .actions import Run
from .command import GateCommand
from .errors import GateError
from .execution import Kind, Needs, Speed, step
from .plan import Plan


class DevReadyCommand(GateCommand, name="dev-ready", help="run doctor once, on a fresh checkout"):
    """A sentinel, so the first run is guided and every later one is quiet."""

    def plan(self) -> Plan:
        plan = Plan(self.name)
        if self._config.path(self._config.devloop.setup_sentinel).exists():
            return plan
        plan.add(imagebuild.doctor(self._config))
        return plan


class DevCommand(GateCommand, name="dev", help="run one development surface"):
    """Three surfaces, one selector.

    The frontend surface stays a passthrough to `pnpm run dev`: it is an
    interactive server, and putting a Python process between the terminal and
    it costs signal handling and gains nothing.

    `args` is reachable only here, not through `just dev`. `just` joins a
    variadic into one string before interpolating it, so the recipe could
    either quote it -- collapsing every argument into one -- or leave it as
    shell source, which is what it did. Neither preserves the boundaries, so
    the passthrough is spelled `uv run --project build_system --frozen capsem-gate dev tui --fixture` instead
    of pretending `just` can carry it.
    """

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument("surface", nargs="?", default="ui")
        parser.add_argument("args", nargs="*")

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config
        settings = config.devloop
        surface = self._args.surface

        if surface not in settings.surfaces:
            raise GateError(
                f"unknown surface {surface!r}; expected one of {', '.join(settings.surfaces)}"
            )

        if surface == "frontend":
            plan.add(
                step(
                    surface,
                    Run(settings.frontend_dev, cwd=config.path(settings.frontend_dir)),
                    kind=Kind.COMPILE,
                    needs=frozenset({Needs.DISK}),
                    speed=Speed.FAST,
                )
            )
        elif surface == "tui":
            plan.add(step(surface, Run([*settings.tui, *self._args.args]),
                kind=Kind.COMPILE,
                needs=frozenset({Needs.DISK}),
                speed=Speed.FAST,
            ))
        else:
            plan.add(
                step(
                    surface,
                    Run(
                        settings.tauri,
                        env=config.environment.content(assets=config.imagebuild.output),
                    ),
                    kind=Kind.COMPILE,
                    needs=frozenset({Needs.DISK}),
                    speed=Speed.FAST,
                )
            )
        return plan
