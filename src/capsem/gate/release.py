"""Qualify one immutable source commit, then dispatch one release family.

The selected commit is prepared on main before this command starts. The gate
runs solely from its detached full-SHA prefix; after the complete proof it may
create immutable transport/version refs, but it never edits tracked source.
"""

from __future__ import annotations

import os
from pathlib import Path

from . import imagebuild, sandbox
from .actions import Run, Script
from .command import GateCommand
from .config import GateConfig
from .egress import Egress
from .errors import GateError
from .execution import Kind, Needs, Speed, step
from .fileactions import MakeDir
from .lifecycle import Resource
from .plan import Plan
from .proc import Runner
from .qualificationevidence import AcceptQualification, QualificationPolicy
from .sandboxreport import SandboxReport
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


class _QualifiedRelease:
    """A short dispatcher that consumes, but never repeats, qualification."""

    _config: GateConfig
    _sandbox_mode: sandbox.SandboxMode
    sandboxed = sandbox.ENFORCE
    qualification_policy = QualificationPolicy.REQUIRE
    private_checkout = True
    outside_egress = True

    def resources(self, runner: Runner) -> tuple[Resource, ...]:
        return (
            SandboxReport(self._config, runner, mode=self._sandbox_mode),
            Egress(self._config, enabled=self._sandbox_mode is not sandbox.OFF),
        )


class ReleaseBinariesCommand(
    _QualifiedRelease,
    GateCommand,
    name="release-binaries",
    help="release exact previously-qualified packages for one channel",
):
    exclusive = True
    publishes = True
    uses_qualification = True

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

        accepted = plan.add(
            step(
                "qualification.accept",
                AcceptQualification(self.source_commit()),
                kind=Kind.STATIC_TEST,
                speed=Speed.FAST,
            )
        )
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
            ),
            after=(accepted,),
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
            after=(fetched,),
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
    _QualifiedRelease,
    GateCommand,
    name="release-profile",
    help="release one exact previously-qualified channel profile",
):
    exclusive = True
    publishes = True
    uses_qualification = True

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

        accepted = plan.add(
            step(
                "qualification.accept",
                AcceptQualification(self.source_commit()),
                kind=Kind.STATIC_TEST,
                speed=Speed.FAST,
            )
        )
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
            ),
            after=(accepted,),
        )
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
            after=(checked,),
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
