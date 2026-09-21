"""The SDKs surviving a managed restart, proved under real launchd.

The proof registers a disposable launchd job with KeepAlive, restarts the
service underneath a connected SDK, and checks the SDK reconnects explicitly.
launchd refuses `launchctl bootstrap` from any Seatbelt-sandboxed caller, so
the sandboxed functional suites cannot host it; it runs in the macOS glow-up
lane, which already installs and drives launchd, outside the sandbox.
"""

from __future__ import annotations

from . import pytestsuite, sdkchecks
from .actions import Run
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .plan import Plan


def fragment(plan: Plan, config: GateConfig, *, after: tuple[Step, ...]) -> Step:
    """Run the managed-restart suite once the SDK it drives is usable."""
    settings = config.suites.pytest
    suite = pytestsuite.Suite(
        label="pytest.managed-restart",
        paths=(settings.managed_restart,),
        profile=settings.base_profile,
    )
    return plan.phase("glowup").add(
        step(
            suite.label,
            Run(suite.argv(config), env=suite.environment(config), outside_sandbox=True),
            contends=(config.exclusive("host_service"),),
            kind=Kind.E2E,
            # NETWORK because it escapes the sandbox; the suite itself only
            # talks to the service and gateway over loopback.
            needs=frozenset({Needs.DISK, Needs.NETWORK}),
            speed=Speed.SLOW,
        ),
        after=(*after, sdkchecks.python_environment(plan, config)),
    )
