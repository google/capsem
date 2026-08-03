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
from .candidate import CompleteGate
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
            f"unknown channel {channel!r}; expected one of {', '.join(config.package.channels)}"
        )


def _require_profile(config: GateConfig, profile: str) -> None:

    known = imagebuild.profiles(config)
    if profile not in known:
        raise GateError(f"unknown profile {profile!r}; expected one of {', '.join(known)}")


def _gate(plan: Plan, config: GateConfig, *, qualification, after):
    """The complete local proof, composed rather than launched.

    `Run(["just", "test"])` from here started a second gate, and both release
    commands are exclusive -- so the child waited out its timeout for the lock
    its own parent held, and no release could ever have run. Composed, the
    release plan *contains* the gate, so "nothing publishes before the
    complete proof passes" is an edge rather than a promise.
    """
    return candidateplan.compose(plan, config, qualification=qualification, after=after)


class ReleaseBinariesCommand(
    CompleteGate,
    GateCommand,
    name="release-binaries",
    help="run the complete gate, then release packages for one channel",
):
    """The gate runs inside this command, so it holds what the gate holds and
    keeps the host awake for the same reason candidate does."""

    exclusive = True

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
                    "--channel",
                    channel,
                    "--repository",
                    os.environ.get(settings.repository_variable, settings.default_repository),
                    "--require-profile-membership",
                    "--output",
                    settings.channel_source,
                ),
            ),
            after=(checked,),
        )

        recorded = plan.add(step("record-head", RecordHead(head_file(config))), after=(fetched,))
        gate = _gate(plan, config, qualification=self.qualification, after=(recorded,))
        confirmed = plan.add(
            step("confirm-head", ConfirmHead(settings.publish, head_file(config))),
            after=(gate,),
        )
        plan.add(step("release", Script(settings.binaries, channel)), after=(confirmed,))
        return plan


class ReleaseProfileCommand(
    CompleteGate,
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
        recorded = plan.add(step("record-head", RecordHead(head_file(config))), after=(checked,))
        gate = _gate(plan, config, qualification=self.qualification, after=(recorded,))
        confirmed = plan.add(
            step("confirm-head", ConfirmHead(settings.publish, head_file(config))),
            after=(gate,),
        )
        plan.add(
            step(
                "release",
                Run(
                    [
                        *settings.profile,
                        "--channel",
                        self._args.channel,
                        "--profile",
                        self._args.profile,
                    ]
                ),
            ),
            after=(confirmed,),
        )
        return plan
