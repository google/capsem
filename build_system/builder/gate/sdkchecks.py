"""Python SDK checks use its own locked install and the shared gate graph."""

from __future__ import annotations

from .actions import Run
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .plan import Plan
from .pythonenv import uv_run


def fragment(plan: Plan, config: GateConfig, *, after: tuple[Step, ...]) -> tuple[Step, ...]:
    settings = config.sdk_python
    phase = plan.phase("fast.sdk.python")
    prefix = ["uv", "run", "--project", settings.project, "--frozen", "--no-sync"]
    synced = phase.add(step(
        "sync", Run(["uv", "sync", "--project", settings.project, "--frozen"]),
        kind=Kind.COMPILE, needs=frozenset({Needs.DISK}), speed=Speed.FAST,
    ), after=after)
    commands = {
        "generate": uv_run(config, "python", "-m", "capsem_builder.sdkgen", "--check",
                           "--specification", settings.specification, "--python-package", settings.source),
        "lint": [*prefix, "ruff", "check", "--config", config.suites.pytest.project_manifest,
                 settings.source, settings.tests],
        "types": [*prefix, "ty", "check", "--project", settings.project, "--error-on-warning",
                  "--python-platform", "all", settings.source, settings.tests],
        "build": ["uv", "build", "--project", settings.project, "--no-sources",
                  "--out-dir", settings.build_output],
    }
    checks = tuple(phase.add(step(
        label, Run(argv), kind=Kind.PACKAGE if label == "build" else Kind.LINT, speed=Speed.FAST,
    ), after=(synced,)) for label, argv in commands.items())
    tested = phase.add(step(
        "tests", Run(["uv", "run", "--frozen", "--no-sync", "pytest"], cwd=config.path(settings.project)),
        kind=Kind.UNIT_TEST, speed=Speed.FAST,
    ), after=(synced,))
    return (*checks, tested)
