"""The two release commands, and the order that is the whole point of them.

Nothing may stamp a version, mutate a tracked file, push, tag, or dispatch a
workflow before the complete local gate has passed against the exact HEAD being
published. In shell that order was six lines whose correctness was where they
sat; here it is edges, so a step cannot be moved above the gate by accident.

The cheap prechecks come first deliberately. A dirty tree, the wrong branch, or
missing release notes are deterministic failures, and finding them after forty
minutes of gate is forty minutes nobody gets back.
"""

from __future__ import annotations

import os

from .actions import Run, Script
from .command import GateCommand
from .config import GateConfig
from .errors import GateError
from .execution import step
from .fileactions import MakeDir
from .plan import Plan
from .releasehead import ConfirmHead, RecordHead, head_file


def _gate(config: GateConfig):
    """The complete local proof. Never a reduced one."""
    return step("gate", Run(["just", "test"]))


class ReleaseBinariesCommand(
    GateCommand,
    name="release-binaries",
    help="run the complete gate, then release packages for one channel",
):
    exclusive = True

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument("channel")

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config
        settings = config.release
        channel = self._args.channel

        if channel not in config.package.channels:
            raise GateError(
                f"unknown channel {channel!r}; expected one of "
                f"{', '.join(config.package.channels)}"
            )

        checked = plan.add(
            step(
                "precheck",
                # Seconds, not minutes: a dirty tree or the wrong branch is a
                # deterministic failure and should not cost a gate run. The
                # authoritative check still runs after, because the state can
                # drift while the gate is going.
                Script(*settings.precheck),
                Script(*settings.notes),
                MakeDir(config.path(settings.preflight_dir)),
            )
        )

        fetched = plan.add(
            step(
                "channel-source",
                Script(
                    settings.fetch_manifest,
                    "--channel", channel,
                    "--repository",
                    os.environ.get(settings.repository_variable, settings.default_repository),
                    "--require-profile-membership",
                    "--output", settings.channel_source,
                ),
            ),
            after=(checked,),
        )

        recorded = plan.add(
            step("record-head", RecordHead(head_file(config))), after=(fetched,)
        )
        gate = plan.add(_gate(config), after=(recorded,))
        confirmed = plan.add(
            step("confirm-head", ConfirmHead(settings.publish, head_file(config))),
            after=(gate,),
        )
        plan.add(step("release", Script(settings.binaries, channel)), after=(confirmed,))
        return plan


class ReleaseProfileCommand(
    GateCommand,
    name="release-profile",
    help="run the complete gate, then release one channel profile",
):
    exclusive = True

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument("channel")
        parser.add_argument("profile")

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config
        settings = config.release

        checked = plan.add(
            step(
                "precheck",
                Script(*settings.precheck),
                MakeDir(config.path(settings.preflight_dir)),
            )
        )
        recorded = plan.add(
            step("record-head", RecordHead(head_file(config))), after=(checked,)
        )
        gate = plan.add(_gate(config), after=(recorded,))
        confirmed = plan.add(
            step("confirm-head", ConfirmHead(settings.publish, head_file(config))),
            after=(gate,),
        )
        plan.add(
            step(
                "release",
                Run([
                    *settings.profile,
                    "--channel", self._args.channel,
                    "--profile", self._args.profile,
                ]),
            ),
            after=(confirmed,),
        )
        return plan


class CandidateModulesCommand(
    GateCommand,
    name="test-candidate",
    help="every checked-in module, after rebuilding the assets they run against",
):
    """Composition, and one thing that is not.

    The benchmark recordings are cleared exactly once here, before any module
    runs. Clearing them per module is what left a fortnight of full gates with
    an empty directory and froze the published arm64 history.
    """

    exclusive = True

    def plan(self) -> Plan:
        from .fileactions import Remove

        plan = Plan(self.name)
        config = self._config

        prepared = plan.add(
            step(
                "prepare",
                Run(["just", "_bootstrap"]),
                Run(["just", "_bound-docker-test-storage"]),
                Run(["just", "_clean-stale"]),
                Run(["just", "_check-generated-settings"]),
                Remove(config.path(config.workspace.benchmark_root)),
                Run(["just", "_prepared-runtime"]),
            )
        )

        previous = prepared
        for module in ("test-static", "test-artifacts", "test-functional", "test-glowup"):
            previous = plan.add(
                step(module, Run(["uv", "run", "capsem-gate", module])), after=(previous,)
            )
        plan.add(step("recipes", Run(["just", "_test-recipes"])), after=(previous,))
        return plan


class DevReadyCommand(
    GateCommand, name="dev-ready", help="run doctor once, on a fresh checkout"
):
    """A sentinel, so the first run is guided and every later one is quiet."""

    def plan(self) -> Plan:
        plan = Plan(self.name)
        if self._config.path(self._config.devloop.setup_sentinel).exists():
            return plan
        plan.add(step("first-run-doctor", Run(["just", "doctor"])))
        return plan


class DevCommand(
    GateCommand, name="dev", help="run one development surface"
):
    """Three surfaces, one selector.

    The frontend surface stays a passthrough to `pnpm run dev`: it is an
    interactive server, and putting a Python process between the terminal and
    it costs signal handling and gains nothing.
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
                f"unknown surface {surface!r}; expected one of "
                f"{', '.join(settings.surfaces)}"
            )

        if surface == "frontend":
            plan.add(
                step(
                    surface,
                    Run(settings.frontend_dev, cwd=config.path(settings.frontend_dir)),
                )
            )
        elif surface == "tui":
            plan.add(step(surface, Run([*settings.tui, *self._args.args])))
        else:
            plan.add(
                step(
                    surface,
                    Run(
                        settings.tauri,
                        env={"CAPSEM_ASSETS_DIR": config.imagebuild.output},
                    ),
                )
            )
        return plan


class ShellCommand(
    GateCommand, name="shell", help="start the service and enter a temporary VM"
):
    exclusive = True

    def plan(self) -> Plan:
        plan = Plan(self.name)
        plan.add(
            step("shell", Run([self._config.logs.cli, "shell"]),
                 contends=(self._config.exclusive("apple_vz"),))
        )
        return plan


class ExecCommand(
    GateCommand, name="exec", help="run one command in a fresh temporary VM"
):
    exclusive = True

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument("command", nargs="+")

    def plan(self) -> Plan:
        plan = Plan(self.name)
        plan.add(
            step(
                "exec",
                Run([self._config.logs.cli, "exec", *self._args.command]),
                contends=(self._config.exclusive("apple_vz"),),
            )
        )
        return plan
