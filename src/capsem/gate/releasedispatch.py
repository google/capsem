"""What both release commands share: a dispatcher that consumes qualification.

Split out of `release`, which holds the two commands themselves. The seam is
the one the boundary guard asks for: this is the lifecycle every release has in
common -- how it is sandboxed, what it holds, and whether the channel it
targets demands a proof an operator made.
"""

from __future__ import annotations

import argparse

from . import sandbox
from .config import GateConfig
from .egress import Egress
from .execution import Kind, Speed, step
from .lifecycle import Resource
from .plan import Plan
from .proc import Runner
from .qualificationevidence import AcceptQualification, QualificationPolicy
from .sandboxreport import SandboxReport
from .sourcecommit import SourceCommit


class QualifiedRelease:
    """A short dispatcher that consumes, but never repeats, qualification."""

    _config: GateConfig
    _sandbox_mode: sandbox.SandboxMode
    sandboxed = sandbox.ENFORCE
    private_checkout = True
    outside_egress = True

    _args: argparse.Namespace

    @property
    def qualification_policy(self) -> QualificationPolicy:
        """Stable consumes a proof an operator made; nightly makes its own.

        A journal is written only by `just test` and archived per machine, so
        requiring one is requiring a human at a particular keyboard. That is
        the right bar for stable, which publishes deliberately. It is an
        impossible bar for the daily rebuild, which runs on a fresh hosted
        runner that has no journal and no way to produce one -- and it is an
        unnecessary one, because the lanes a release dispatches prove
        themselves before publishing anything.
        """
        channel = getattr(self._args, "channel", None)
        if channel in self._config.release.locally_qualified_channels:
            return QualificationPolicy.REQUIRE
        return QualificationPolicy.NONE

    def _qualification_steps(self, plan: Plan, commit: SourceCommit) -> tuple:
        """The accept step, for the channels that consume an operator's proof."""
        if self.qualification_policy is not QualificationPolicy.REQUIRE:
            return ()
        return (
            plan.add(
                step(
                    "qualification.accept",
                    AcceptQualification(commit),
                    kind=Kind.STATIC_TEST,
                    speed=Speed.FAST,
                )
            ),
        )

    def resources(self, runner: Runner) -> tuple[Resource, ...]:
        return (
            SandboxReport(self._config, runner, mode=self._sandbox_mode),
            Egress(self._config, enabled=self._sandbox_mode is not sandbox.OFF),
        )
