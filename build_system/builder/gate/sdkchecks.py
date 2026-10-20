"""SDK packages have isolated language checks in the shared gate graph."""

from __future__ import annotations

from .actions import Run
from .buildschema import PythonPackageConfig, SourcePackageConfig
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .fileactions import MakeDir
from .phase import Phase
from .plan import Plan
from .pythonenv import uv_run


def python_environment(
    plan: Plan,
    config: GateConfig,
    *,
    project: str = "",
    after: tuple[Step, ...] = (),
    prefix: str = "sdk.python",
) -> Step:
    """A Python package's environment, shared by every lane that uses it:
    dependencies fetched outside the sandbox, then the project built inside it
    without an isolated build, which would fetch its backend from the network."""
    project = project or config.sdk_python.project
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
    compiled = typescript_compiled(plan, config, after=after)
    typescript = phase.shared(typescript_package(config), after=(compiled,))
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


def python_package_prewarm(config: GateConfig) -> Step:
    """Rehydrate build backends after the candidate cache-enforcement phase.

    Clean consumers install the sdist offline, so uv must still have its
    PEP 517 backend after the gate has enforced the shared cache budget.
    """
    settings = config.sdk_python
    return step(
        "sdk.python.package-prewarm",
        Run(
            [
                "uv",
                "export",
                "--project",
                settings.project,
                "--frozen",
                "--only-group",
                "package-build",
                "--no-emit-project",
                "--format",
                "requirements.txt",
                "--output-file",
                settings.package_build_requirements,
            ],
        ),
        Run(
            [
                "uv",
                "pip",
                "install",
                "--python",
                settings.environment_python,
                "--reinstall",
                "--requirements",
                settings.package_build_requirements,
            ],
            outside_sandbox=True,
        ),
        kind=Kind.COMPILE,
        needs=frozenset({Needs.DISK, Needs.NETWORK}),
        speed=Speed.FAST,
    )


def typescript_package(config: GateConfig) -> Step:
    settings = config.sdk_typescript
    return step(
        "fast.sdk.typescript.build",
        Run(["pnpm", "pack", "--config.ignore-scripts=true", "--pack-destination", str(config.path(settings.build_output))],
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
    settings: PythonPackageConfig,
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
        "tests", MakeDir(config.path(settings.pytest_scratch).parent),
        Run(["uv", "run", "--frozen", "--no-sync", "pytest",
                      "--basetemp", str(config.path(settings.pytest_scratch))], cwd=config.path(settings.project)),
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
    compiled = typescript_compiled(plan, config, after=after)
    built = phase.shared(typescript_package(config), after=(compiled,))
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
    archives = {config.environment.mcp_package_archives: str(config.path(mcp.build_output))}
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
    mcp_typed = mcp_phase.add(
        step("types", Run(["pnpm", "run", "check"], cwd=mcp_root), kind=Kind.LINT, speed=Speed.FAST),
        after=(mcp_built,),
    )
    mcp_warmed = mcp_phase.add(
        step(
            "package-prewarm",
            Run(["pnpm", "run", "prewarm:packed"], cwd=mcp_root, env=archives, outside_sandbox=True),
            kind=Kind.COMPILE,
            needs=frozenset({Needs.DISK, Needs.NETWORK}),
            speed=Speed.FAST,
        ),
        after=(mcp_built,),
    )
    mcp_tested = mcp_phase.add(
        step(
            "tests",
            Run(["pnpm", "run", "test:mcp"], cwd=mcp_root, env=archives),
            kind=Kind.UNIT_TEST,
            speed=Speed.FAST,
        ),
        after=(mcp_warmed,),
    )
    return (*checks, built, warmed, tested, generated, mcp_built, mcp_typed, mcp_warmed, mcp_tested)


def typescript_bundle(config: GateConfig) -> Step:
    """Compile the linked SDK before a standalone frontend consumer starts."""
    settings = config.sdk_typescript
    return step(
        "sdk.typescript.bundle",
        Run(["pnpm", "run", "build"], cwd=config.path(settings.project),
            timeout_seconds=settings.build_timeout_seconds),
        kind=Kind.COMPILE, speed=Speed.FAST,
    )


def typescript_compiled(plan: Plan, config: GateConfig, *, after: tuple[Step, ...]) -> Step:
    """Reuse earlier output without moving its producer behind later consumers."""
    bundle = typescript_bundle(config)
    # A standalone functional/artifact lane may install Node after runtime
    # preparation. When a prior phase already compiled the SDK, adding that
    # later setup as its prerequisite closes a cycle. Keep those dependencies
    # on the consumer; shared still verifies the producer's actions match.
    existing = any(node.label == bundle.label for node in plan.steps)
    return plan.shared(bundle, after=() if existing else after)
