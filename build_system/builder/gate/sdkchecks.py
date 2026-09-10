"""SDK packages have isolated language checks in the shared gate graph."""

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


def typescript_fragment(plan: Plan, config: GateConfig, *, after: tuple[Step, ...]) -> tuple[Step, ...]:
    settings = config.sdk_typescript
    phase = plan.phase("fast.sdk.typescript")
    root = config.path(settings.project)
    checks = tuple(phase.add(step(
        label, Run(["pnpm", "run", command], cwd=root),
        kind=Kind.UNIT_TEST if label == "tests" else Kind.LINT, speed=Speed.FAST,
    ), after=after) for label, command in (("lint", "lint"), ("types", "check"), ("tests", "test")))
    built = phase.add(step(
        "build", Run(["pnpm", "pack", "--pack-destination", str(config.path(settings.build_output))], cwd=root),
        kind=Kind.PACKAGE, speed=Speed.FAST,
    ), after=after)
    generated = phase.add(step(
        "generate", Run(uv_run(config, "python", "-m", "capsem_builder.sdkgen", "--check",
                               "--specification", settings.specification, "--typescript-source", settings.source)),
        kind=Kind.LINT, speed=Speed.FAST,
    ), after=after)
    return (*checks, built, generated)


def rust_fragment(plan: Plan, config: GateConfig, *, after: tuple[Step, ...]) -> tuple[Step, ...]:
    settings = config.sdk_rust
    generated = plan.phase("fast.sdk.rust").add(step(
        "generate", Run(uv_run(config, "python", "-m", "capsem_builder.sdkgen", "--check",
                               "--specification", settings.specification, "--rust-source", settings.source)),
        kind=Kind.LINT, speed=Speed.FAST,
    ), after=after)
    return (generated,)


def typescript_bundle(config: GateConfig) -> Step:
    """Compile the linked SDK before a standalone frontend consumer starts."""
    return step(
        "sdk.typescript.bundle",
        Run(["pnpm", "run", "build"], cwd=config.path(config.sdk_typescript.project)),
        kind=Kind.COMPILE, speed=Speed.FAST,
    )
