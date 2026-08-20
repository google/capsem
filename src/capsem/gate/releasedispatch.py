"""What both release commands share: a dispatcher that consumes qualification.

Split out of `release`, which holds the two commands themselves. The seam is
the one the boundary guard asks for: this is the lifecycle every release has in
common -- how it is sandboxed, what it holds, and whether the channel it
targets demands a proof an operator made.
"""

from __future__ import annotations

import argparse

from . import sandbox
from .actions import Script
from .config import GateConfig
from .egress import Egress
from .execution import Kind, Speed, step
from .lifecycle import Resource
from .plan import Plan
from .proc import Runner
from .qualificationevidence import (
    AcceptQualification,
    QualificationPolicy,
    WaiveQualification,
)
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

    qualification_policy: QualificationPolicy = QualificationPolicy.REQUIRE
    """Stable consumes a proof an operator made; nightly makes its own.

    A journal is written only by `just test` and archived per machine, so
    requiring one is requiring a human at a particular keyboard. That is the
    right bar for stable, which publishes deliberately. It is an impossible bar
    for the daily rebuild, which runs on a fresh hosted runner that has no
    journal and no way to produce one -- and an unnecessary one, because the
    lanes a release dispatches prove themselves before publishing anything.

    Declared as a class default and narrowed per instance, not computed as a
    property: the gate reads this off the *class* to decide sandbox enforcement
    before it ever builds a command, and a property object is neither policy.
    """

    def __init__(self, *args, **kwargs) -> None:
        super().__init__(*args, **kwargs)
        channel = getattr(self._args, "channel", None)
        if channel not in self._config.release.locally_qualified_channels:
            self.qualification_policy = QualificationPolicy.NONE

    def _worktree_steps(self, plan: Plan, commit: SourceCommit) -> tuple:
        """Refuse a release from a dirty tree, before anything is accepted.

        The run works from a detached copy of `commit`, so an uncommitted fix
        is not in the release and cannot be. Nothing used to say so, and the
        resulting build looked like the change had done nothing.

        The outer checkout is the subject, not this one: inside a prefix the
        tree is a clean copy of the commit by construction, which would make
        the check pass by asking the wrong tree.
        """
        if self._forced():
            return ()
        from . import qualificationevidence

        outer = qualificationevidence.authority(self._config).root
        return (
            plan.add(
                step(
                    "source.worktree-clean",
                    Script(
                        self._config.release.clean_worktree,
                        str(outer),
                        str(commit),
                    ),
                    kind=Kind.STATIC_TEST,
                    speed=Speed.FAST,
                )
            ),
        )

    def _forced(self) -> bool:
        """Whether the operator typed `--force`, which is the whole safeguard."""
        return getattr(self._args, "force", "false") == "true"

    def _forced_source_proof(self, plan: Plan, *, after: tuple) -> tuple:
        """The cheap proof a forced release still owes.

        `--force` waives the two-and-a-half-hour product qualification, and
        that is the point: the commits worth forcing are the ones that do not
        change the product. But it used to waive *everything*, so a forced
        release could dispatch source that fails a six-second guard -- and did,
        three times in one afternoon, each costing a forty-minute lane to
        discover a line-count ratchet or a stale contract.

        The fit is exact. What people force-release are gate and CI changes,
        and the citadel guards and release contracts are precisely the suites
        that judge those. So force now means "prove the source, skip the
        artifacts" rather than "prove nothing", and it costs about four minutes
        against the dispatch it replaces.

        Composed rather than invoked: a plan action may not start a second
        gate, so this is the same fragment `test-release-contracts` builds.
        """
        if not self._forced():
            return after
        from . import module_contracts, pytestsuite

        guards = plan.add(
            pytestsuite.citadel(self._config).as_step(self._config), after=after
        )
        return (module_contracts.release_contracts(plan, self._config, after=(guards,)),)

    def _qualification_steps(self, plan: Plan, commit: SourceCommit, *, after: tuple = ()) -> tuple:
        """The accept step, for the channels that consume an operator's proof."""
        if self.qualification_policy is not QualificationPolicy.REQUIRE:
            return ()
        return (
            plan.add(
                step(
                    "qualification.waived" if self._forced() else "qualification.accept",
                    WaiveQualification(commit) if self._forced() else AcceptQualification(commit),
                    kind=Kind.STATIC_TEST,
                    speed=Speed.FAST,
                ),
                after=after,
            ),
        )

    def resources(self, runner: Runner) -> tuple[Resource, ...]:
        return (
            SandboxReport(self._config, runner, mode=self._sandbox_mode),
            Egress(self._config, enabled=self._sandbox_mode is not sandbox.OFF),
        )
