"""What both release commands share: a dispatcher that consumes local proof.

Split out of `release`, which holds the two commands themselves. The seam is
the one the boundary guard asks for: this is the lifecycle every release has in
common -- how it is sandboxed, what it holds, how it refuses dirty source, and
which proof it demands before it tags, pushes, or dispatches anything.
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
from .qualificationevidence import AcceptQualification, QualificationPolicy
from .sandboxreport import SandboxReport
from .sourcecommit import SourceCommit


class QualifiedRelease:
    """A short dispatcher that consumes, but never repeats, local qualification."""

    _config: GateConfig
    _sandbox_mode: sandbox.SandboxMode
    sandboxed = sandbox.ENFORCE
    private_checkout = True
    outside_egress = True

    _args: argparse.Namespace

    qualification_policy: QualificationPolicy = QualificationPolicy.REQUIRE
    """A release needs a passing `just test <commit>` for its exact commit.

    Four stable attempts in one day each failed in the hosted lane after about
    two and a half hours, on a defect the local glow-up catches in minutes.
    `qualificationflow.decide` refuses a missing journal before the prefix, the
    machine lock, or any tag, push, or dispatch.

    The unattended nightly scheduler is the exception: it runs on a fresh
    hosted runner with no journal and no way to make one. `--force` is the
    operator's: a dependency advisory published during the two-hour proof
    refused three stable dispatches in a row, each fixed by a lockfile bump
    that then needed another full proof. A forced release still runs the
    citadel guards, the release contracts and the live advisory audit here,
    and the hosted lane qualifies its artifacts before publishing. Declared
    as a class default and narrowed per instance, because sandbox enforcement
    reads the class before a command exists.
    """

    def __init__(self, *args, **kwargs) -> None:
        super().__init__(*args, **kwargs)
        if (
            getattr(self._args, "channel", None) in self._config.release.unattended_channels
            or self._forced()
        ):
            self.qualification_policy = QualificationPolicy.NONE

    def _qualification_steps(self, plan: Plan, commit: SourceCommit, *, after: tuple) -> tuple:
        """Revalidate the exact journal at the graph edge, inside the prefix."""
        if self.qualification_policy is not QualificationPolicy.REQUIRE:
            return after
        return (
            plan.add(
                step(
                    "qualification.accept",
                    AcceptQualification(commit),
                    kind=Kind.STATIC_TEST,
                    speed=Speed.FAST,
                ),
                after=after,
            ),
        )

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
                        self._config,
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

        `--force` bypasses the developer checkout's clean-tree refusal and the
        local `just test` journal; it does not waive the hosted lane's product
        qualification. It used to waive
        *everything*, so a forced release could dispatch source that fails a
        six-second guard -- and did, three times in one afternoon, each costing
        a forty-minute lane to discover a line-count ratchet or stale contract.

        The fit is exact. What people force-release are gate and CI changes,
        and the citadel guards and release contracts are precisely the suites
        that judge those. So force means "prove the source despite the dirty
        outer checkout" rather than "prove nothing"; the hosted lane still
        builds and qualifies its artifacts before publication.

        Composed rather than invoked: a plan action may not start a second
        gate, so this is the same fragment `test-release-contracts` builds.
        """
        if not self._forced():
            return after
        from . import module_contracts, pytestsuite

        guards = plan.add(pytestsuite.citadel(self._config).as_step(self._config), after=after)
        return (module_contracts.release_contracts(plan, self._config, after=(guards,)),)

    def _live_advisory_proof(self, plan: Plan, *, after: tuple) -> tuple:
        """Fail known-live dependency drift before spending a hosted dispatch."""
        from . import audits

        live = audits.live(self._config)
        dependencies = plan.add(live.dependencies, after=after)
        rust_policy = plan.add(live.rust_policy, after=(dependencies,))
        return (rust_policy,)

    def resources(self, runner: Runner) -> tuple[Resource, ...]:
        return (
            SandboxReport(self._config, runner, mode=self._sandbox_mode),
            Egress(self._config, enabled=self._sandbox_mode is not sandbox.OFF),
        )
