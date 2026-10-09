"""SDK packages have isolated language checks in the shared gate graph."""

from __future__ import annotations

from .actions import Run
from .buildschema import SourcePackageConfig
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .phase import Phase
from .plan import Plan
from .pythonenv import uv_run


def python_environment(
    plan: Plan,
    config: GateConfig,
    *,
    project: str,
    after: tuple[Step, ...] = (),
    prefix: str = "sdk.python",
) -> Step:
    """A Python package's environment, shared by every lane that uses it:
    dependencies fetched outside the sandbox, then the project built inside it
    without an isolated build, which would fetch its backend from the network."""
    prewarm = plan.shared(step(
        f"{prefix}.prewarm",
        Run(["uv", "sync", "--project", project, "--frozen", "--no-install-project"], outside_sandbox=True),
        kind=Kind.COMPILE,
        needs=frozenset({Needs.DISK, Needs.NETWORK}),
        speed=Speed.FAST,
    ), after=after)
    return plan.shared(step(
        f"{prefix}.sync",
        Run(["uv", "sync", "--project", project, "--frozen", "--no-build-isolation"]),
        kind=Kind.COMPILE, needs=frozenset({Needs.DISK}), speed=Speed.FAST,
    ), after=(prewarm,))


def braavos(plan: Plan, phase: Phase, config: GateConfig, *, after: tuple[Step, ...]) -> tuple[Step, ...]:
    """Every SDK the SDK suites drive, usable before the suites start with no
    network. `after` carries the `toolchain.node` install the bundle builds from."""
    example = phase.add(step(
        "sdk.rust.example", Run(list(config.functional.sdk_rust_example)),
        contends=(config.exclusive("workspace_binaries"),),
        kind=Kind.COMPILE, needs=frozenset({Needs.DISK}), speed=Speed.SLOW,
    ), after=after)
    synced = python_environment(plan, config, project=config.sdk_python.project)
    inspect_env = python_environment(
        plan, config, project=config.integrations_inspect_ai.project, prefix="integrations.inspect-ai"
    )
    python = phase.shared(python_package(config.sdk_python), after=(synced,))
    inspect_pkg = phase.shared(
        python_package(config.integrations_inspect_ai, step_prefix="integrations.inspect-ai"),
        after=(inspect_env,),
    )
    typescript = phase.shared(typescript_package(config), after=after)
    warmed = phase.shared(typescript_prewarm(config), after=(typescript,))
    return python, inspect_env, inspect_pkg, example, typescript, warmed


def python_package(
    settings: SourcePackageConfig,
    *,
    step_prefix: str = "sdk.python",
) -> Step:
    return step(
        f"fast.{step_prefix}.build",
        Run(["uv", "run", "--project", settings.project, "--frozen", "--no-sync",
             "python", "-m", "build", "--no-isolation", "--outdir", settings.build_output,
             settings.project]),
        kind=Kind.PACKAGE, speed=Speed.FAST,
    )


def typescript_package(config: GateConfig) -> Step:
    settings = config.sdk_typescript
    return step(
        "fast.sdk.typescript.build",
        Run(["pnpm", "pack", "--pack-destination", str(config.path(settings.build_output))],
            cwd=config.path(settings.project)),
        kind=Kind.PACKAGE, speed=Speed.FAST,
    )


def typescript_prewarm(config: GateConfig) -> Step:
    return step(
        "fast.sdk.typescript.package-prewarm",
        Run(["pnpm", "run", "prewarm:package"], cwd=config.path(config.sdk_typescript.project),
            outside_sandbox=True),
        kind=Kind.COMPILE, needs=frozenset({Needs.DISK, Needs.NETWORK}), speed=Speed.FAST,
    )


def _python_package_fragment(
    plan: Plan,
    config: GateConfig,
    *,
    after: tuple[Step, ...],
    settings: SourcePackageConfig,
    step_prefix: str,
    extra_commands: dict[str, list[str]] | None = None,
    extra_tests_after: tuple[Step, ...] = (),
) -> tuple[Step, ...]:
    stage = f"fast.{step_prefix}"
    phase = plan.phase(stage)
    prefix = ["uv", "run", "--project", settings.project, "--frozen", "--no-sync"]
    synced = python_environment(
        plan, config, after=after, project=settings.project, prefix=step_prefix
    )
    commands: dict[str, list[str]] = dict(extra_commands) if extra_commands else {}
    commands.update({
        "lint": [*prefix, "ruff", "check", "--config", config.suites.pytest.project_manifest,
                 settings.source, settings.tests],
        "types": [*prefix, "ty", "check", "--project", settings.project, "--error-on-warning",
                  "--python-platform", "all", settings.source, settings.tests],
    })
    checks = {label: phase.add(step(
        label, Run(argv), kind=Kind.LINT, speed=Speed.FAST,
    ), after=(synced,)) for label, argv in commands.items()}
    checks["build"] = phase.shared(python_package(settings, step_prefix=step_prefix), after=(synced,))
    plan.record_stage(checks["build"].label, stage)
    tested = phase.add(step(
        "tests", Run(["uv", "run", "--frozen", "--no-sync", "pytest"], cwd=config.path(settings.project)),
        kind=Kind.UNIT_TEST, speed=Speed.FAST,
    ), after=(checks["build"], *extra_tests_after))
    return (*checks.values(), tested)


def fragment(
    plan: Plan,
    config: GateConfig,
    *,
    after: tuple[Step, ...],
) -> tuple[Step, ...]:
    settings = config.sdk_python
    return _python_package_fragment(
        plan,
        config,
        after=after,
        settings=settings,
        step_prefix="sdk.python",
        extra_commands={
            "generate": uv_run(
                config, "python", "-m", "capsem_builder.sdkgen", "--check",
                "--specification", settings.specification, "--python-package", settings.source,
            ),
        },
    )


def inspect_fragment(plan: Plan, config: GateConfig, *, after: tuple[Step, ...]) -> tuple[Step, ...]:
    sdk_synced = python_environment(plan, config, after=after, project=config.sdk_python.project)
    sdk_built = plan.shared(python_package(config.sdk_python), after=(sdk_synced,))
    return _python_package_fragment(
        plan,
        config,
        after=after,
        settings=config.integrations_inspect_ai,
        step_prefix="integrations.inspect-ai",
        extra_tests_after=(sdk_built,),
    )


def typescript_fragment(plan: Plan, config: GateConfig, *, after: tuple[Step, ...]) -> tuple[Step, ...]:
    settings = config.sdk_typescript
    phase = plan.phase("fast.sdk.typescript")
    root = config.path(settings.project)
    checks = tuple(phase.add(step(
        label, Run(["pnpm", "run", command], cwd=root),
        kind=Kind.LINT, speed=Speed.FAST,
    ), after=after) for label, command in (("lint", "lint"), ("types", "check")))
    built = phase.shared(typescript_package(config), after=after)
    warmed = phase.shared(typescript_prewarm(config), after=(built,))
    for shared in (built, warmed):
        plan.record_stage(shared.label, "fast.sdk.typescript")
    tested = phase.add(step(
        "tests", Run(["pnpm", "run", "test:sdk"], cwd=root),
        kind=Kind.UNIT_TEST, speed=Speed.FAST,
    ), after=(built, warmed))
    generated = phase.add(step(
        "generate", Run(uv_run(config, "python", "-m", "capsem_builder.sdkgen", "--check",
                               "--specification", settings.specification, "--typescript-source", settings.source)),
        kind=Kind.LINT, speed=Speed.FAST,
    ), after=after)
    mcp = config.mcp_typescript
    mcp_root = config.path(mcp.project)
    mcp_phase = plan.phase("fast.mcp.typescript")
    mcp_built = mcp_phase.add(
        step(
            "build",
            Run(["pnpm", "run", "build"], cwd=mcp_root),
            kind=Kind.PACKAGE,
            speed=Speed.FAST,
        ),
        after=(built,),
    )
    mcp_warmed = mcp_phase.add(
        step(
            "package-prewarm",
            Run(["pnpm", "run", "prewarm:packed"], cwd=mcp_root, outside_sandbox=True),
            kind=Kind.COMPILE,
            needs=frozenset({Needs.DISK, Needs.NETWORK}),
            speed=Speed.FAST,
        ),
        after=(mcp_built,),
    )
    mcp_tested = mcp_phase.add(
        step(
            "tests",
            Run(["pnpm", "run", "test:mcp"], cwd=mcp_root),
            kind=Kind.UNIT_TEST,
            speed=Speed.FAST,
        ),
        after=(mcp_warmed,),
    )
    return (*checks, built, warmed, tested, generated, mcp_built, mcp_warmed, mcp_tested)


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
