"""Qualify one immutable source commit, then dispatch one release family.

The selected commit is prepared on main before this command starts. The gate
runs solely from its detached full-SHA prefix; after the complete proof it may
create immutable transport/version refs, but it never edits tracked source.
"""

from __future__ import annotations

import os
from pathlib import Path

from . import candidateplan, imagebuild
from .actions import Run, Script
from .candidate import CompleteGate
from .command import GateCommand
from .config import GateConfig
from .errors import GateError
from .execution import Kind, Needs, Speed, step
from .fileactions import MakeDir
from .plan import Plan
from .sourcecommit import SourceCommit


def _checkout(config: GateConfig) -> Path:
    """The detached immutable repository this release qualifies and dispatches."""
    return config.root


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
    publishes = True
    uses_qualification = True
    outside_egress = True

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument("channel")
        parser.add_argument("source_commit", type=SourceCommit)

    def source_commit(self) -> SourceCommit:
        return self._args.source_commit

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config
        settings = config.release
        channel = self._args.channel
        checkout = _checkout(config)

        _require_channel(config, channel)

        checked = plan.add(
            step(
                "source.remote-main",
                Script(
                    settings.source,
                    str(self.source_commit()),
                    "--ref-template",
                    settings.source_ref_template,
                    "--check",
                    root=checkout,
                    outside_sandbox=True,
                ),
                kind=Kind.STATIC_TEST,
                needs=frozenset({Needs.NETWORK}),
                speed=Speed.FAST,
            )
        )
        prepared = plan.add(
            step(
                "precheck",
                Script(
                    *settings.notes,
                    channel,
                    str(self.source_commit()),
                    root=checkout,
                    outside_sandbox=True,
                ),
                MakeDir(config.path(settings.preflight_dir)),
                kind=Kind.STATIC_TEST,
                speed=Speed.FAST,
            ),
            after=(checked,),
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
                    outside_sandbox=True,
                ),
                kind=Kind.STATIC_TEST,
                needs=frozenset({Needs.NETWORK}),
                speed=Speed.FAST,
            ),
            after=(prepared,),
        )
        gate = _gate(plan, config, qualification=self.qualification, after=(fetched,))
        published = plan.add(
            step(
                "source.publish-ref",
                Script(
                    settings.source,
                    str(self.source_commit()),
                    "--ref-template",
                    settings.source_ref_template,
                    root=checkout,
                    outside_sandbox=True,
                ),
                kind=Kind.PUBLISH,
                needs=frozenset({Needs.NETWORK}),
                speed=Speed.FAST,
            ),
            after=(gate,),
        )
        plan.add(
            step(
                "release",
                Script(
                    settings.binaries,
                    channel,
                    str(self.source_commit()),
                    root=checkout,
                    outside_sandbox=True,
                ),
                kind=Kind.PUBLISH,
                needs=frozenset({Needs.NETWORK}),
                speed=Speed.SLOW,
            ),
            after=(published,),
        )
        return plan


class ReleaseProfileCommand(
    CompleteGate,
    GateCommand,
    name="release-profile",
    help="run the complete gate, then release one channel profile",
):
    exclusive = True
    publishes = True
    uses_qualification = True
    outside_egress = True

    @classmethod
    def add_arguments(cls, parser) -> None:
        parser.add_argument("channel")
        parser.add_argument("profile")
        parser.add_argument("source_commit", type=SourceCommit)

    def source_commit(self) -> SourceCommit:
        return self._args.source_commit

    def plan(self) -> Plan:
        plan = Plan(self.name)
        config = self._config
        settings = config.release
        checkout = _checkout(config)

        # Both arguments, checked in milliseconds. This command spends a
        # complete gate before it publishes, so a name that is wrong is worth
        # discovering now rather than forty minutes from now. `release-binaries`
        # validated its channel and this did not, which is the kind of asymmetry
        # nobody notices until the run that needed it.
        _require_channel(config, self._args.channel)
        _require_profile(config, self._args.profile)

        checked = plan.add(
            step(
                "source.remote-main",
                Script(
                    settings.source,
                    str(self.source_commit()),
                    "--ref-template",
                    settings.source_ref_template,
                    "--check",
                    root=checkout,
                    outside_sandbox=True,
                ),
                MakeDir(config.path(settings.preflight_dir)),
                kind=Kind.STATIC_TEST,
                needs=frozenset({Needs.NETWORK}),
                speed=Speed.FAST,
            )
        )
        gate = _gate(plan, config, qualification=self.qualification, after=(checked,))
        published = plan.add(
            step(
                "source.publish-ref",
                Script(
                    settings.source,
                    str(self.source_commit()),
                    "--ref-template",
                    settings.source_ref_template,
                    root=checkout,
                    outside_sandbox=True,
                ),
                kind=Kind.PUBLISH,
                needs=frozenset({Needs.NETWORK}),
                speed=Speed.FAST,
            ),
            after=(gate,),
        )
        plan.add(
            step(
                "release",
                # In the checkout for the same reason as the binary lane's:
                # this authors an immutable publication and dispatches a
                # workflow, and it must do that from the repository being
                # released rather than from a tree about to be reclaimed.
                Run(
                    [
                        *settings.profile,
                        "--channel",
                        self._args.channel,
                        "--profile",
                        self._args.profile,
                        "--source-commit",
                        str(self.source_commit()),
                    ],
                    cwd=checkout,
                    outside_sandbox=True,
                ),
                kind=Kind.PUBLISH,
                needs=frozenset({Needs.NETWORK}),
                speed=Speed.SLOW,
            ),
            after=(published,),
        )
        return plan
