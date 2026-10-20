"""The SDKs surviving a managed restart under the real native supervisor.

Supervisor setup precedes inherited privilege confinement. macOS uses the
disposable launchd job; Linux enters the isolated systemd manager's closed
network and drops privilege before the SDK worker runs. This one native
glow-up owner remains separate from ordinary functional suites.
"""

from __future__ import annotations

from . import host, pytestsuite, sdkchecks, toolchain
from .actions import Run
from .config import GateConfig
from .content import RuntimeContent
from .execution import Kind, Needs, Speed, Step, step
from .plan import Plan
from .pythonenv import uv_run


def fragment(
    plan: Plan, config: GateConfig, *, after: tuple[Step, ...], content: RuntimeContent
) -> Step:
    """Run the managed-restart suite once the SDK it drives is usable."""
    settings = config.suites.pytest
    suite = pytestsuite.Suite(
        label="pytest.managed-restart",
        paths=(settings.managed_restart,),
        assets_dir=str(content.assets),
    )
    bundle = sdkchecks.typescript_bundle(config)
    if bundle.label in plan.labels:
        bundled = plan.step_named(bundle.label)
    else:
        node = toolchain.node(config, (config.sdk_typescript.project,))
        installed = (
            plan.step_named("fast.toolchain.node")
            if "fast.toolchain.node" in plan.labels
            else plan.step_named(node.label) if node.label in plan.labels else plan.shared(node)
        )
        bundled = plan.shared(bundle, after=(installed,))
    argv = suite.argv(config)
    if not host.on_macos():
        argv = uv_run(config, "python", "-m", settings.managed_restart_driver, "--", *argv)
    return plan.phase("glowup").add(
        step(
            suite.label,
            Run(argv, env=suite.environment(config), outside_sandbox=True),
            contends=(config.exclusive("host_service"),),
            kind=Kind.E2E,
            # NETWORK because it escapes the sandbox; the suite itself only
            # talks to the service and gateway over loopback.
            needs=frozenset({Needs.DISK, Needs.NETWORK}),
            speed=Speed.SLOW,
        ),
        after=(*after, bundled, sdkchecks.python_environment(plan, config)),
    )
