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

from . import candidateplan, imagebuild
from .actions import Run, Script
from .command import GateCommand
from .config import GateConfig
from .errors import GateError
from .execution import step
from .fileactions import MakeDir
from .plan import Plan
from .releasehead import ConfirmHead, RecordHead, head_file


def _require_channel(config: GateConfig, channel: str) -> None:
    if channel not in config.package.channels:
        raise GateError(
            f"unknown channel {channel!r}; expected one of "
            f"{', '.join(config.package.channels)}"
        )


def _require_profile(config: GateConfig, profile: str) -> None:
    from . import imagebuild

    known = imagebuild.profiles(config)
    if profile not in known:
        raise GateError(
            f"unknown profile {profile!r}; expected one of {', '.join(known)}"
        )


def _gate(plan: Plan, config: GateConfig, *, after):
    """The complete local proof, composed rather than launched.

    `Run(["just", "test"])` from here started a second gate, and both release
    commands are exclusive -- so the child waited out its timeout for the lock
    its own parent held, and no release could ever have run. Composed, the
    release plan *contains* the gate, so "nothing publishes before the
    complete proof passes" is an edge rather than a promise.
    """
    return candidateplan.compose(plan, config, after=after)


class ReleaseBinariesCommand(
    GateCommand,
    name="release-binaries",
    help="run the complete gate, then release packages for one channel",
):
    exclusive = True

    def resources(self):
        # The gate runs inside this command now, so this command holds what
        # the gate holds: an isolated home, the process accounting, the
        # Colima it may have started, and the evidence a failure leaves.
        from .candidate import gate_resources

        return gate_resources(self._config, self._runner)

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument("channel")

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config
        settings = config.release
        channel = self._args.channel

        _require_channel(config, channel)

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
        gate = _gate(plan, config, after=(recorded,))
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

    def resources(self):
        from .candidate import gate_resources

        return gate_resources(self._config, self._runner)

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument("channel")
        parser.add_argument("profile")

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config
        settings = config.release

        # Both arguments, checked in milliseconds. This command spends a
        # complete gate before it publishes, so a name that is wrong is worth
        # discovering now rather than forty minutes from now. `release-binaries`
        # validated its channel and this did not, which is the kind of asymmetry
        # nobody notices until the run that needed it.
        _require_channel(config, self._args.channel)
        _require_profile(config, self._args.profile)

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
        gate = _gate(plan, config, after=(recorded,))
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


class DevReadyCommand(
    GateCommand, name="dev-ready", help="run doctor once, on a fresh checkout"
):
    """A sentinel, so the first run is guided and every later one is quiet."""

    def plan(self) -> Plan:
        plan = Plan(self.name)
        if self._config.path(self._config.devloop.setup_sentinel).exists():
            return plan
        plan.add(imagebuild.doctor(self._config))
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
    """One string, carried to the guest without a host shell ever seeing it.

    `guest_command`, not `command`: the latter is the subparser's own slot, and
    a positional named that overwrote the subcommand name -- registry lookup
    then indexed a dict with a list and the public command could not dispatch.

    One positional rather than `nargs="+"`, because the payload is a command
    line for the guest, and rejoining a list is where its quoting is lost.
    `capsem run`, not `capsem exec`: the Rust CLI's `exec` executes in an
    *existing* session and takes one, so the payload would have been consumed
    as a session name. `run` is the one-shot fresh session this documents.
    """

    exclusive = True

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument("guest_command")

    def plan(self) -> Plan:
        plan = Plan(self.name)
        plan.add(
            step(
                "exec",
                Run([self._config.logs.cli, "run", self._args.guest_command]),
                contends=(self._config.exclusive("apple_vz"),),
            )
        )
        return plan
