"""SDK packages have isolated language checks in the shared gate graph."""

from __future__ import annotations

from .actions import Run
from .buildschema import SdkConfig, SourcePackageConfig
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .phase import Phase
from .plan import Plan
from .pythonenv import uv_run


def python_environment(
    plan: Plan,
    config: GateConfig,
    *,
    after: tuple[Step, ...] = (),
    project: str | None = None,
    prefix: str = "sdk.python",
) -> Step:
    """A Python package's environment, shared by every lane that uses it:
    dependencies fetched outside the sandbox, then the project built inside it
    without an isolated build, which would fetch its backend from the network."""
    proj = project or config.sdk_python.project
    prewarm = plan.shared(step(
        f"{prefix}.prewarm",
        Run(["uv", "sync", "--project", proj, "--frozen", "--no-install-project"], outside_sandbox=True),
        kind=Kind.COMPILE,
        needs=frozenset({Needs.DISK, Needs.NETWORK}),
        speed=Speed.FAST,
    ), after=after)
    return plan.shared(step(
        f"{prefix}.sync",
        Run(["uv", "sync", "--project", proj, "--frozen", "--no-build-isolation"]),
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
    bundle = phase.add(typescript_bundle(config), after=after)
    return python_environment(plan, config), example, bundle


def fragment(
    plan: Plan,
    config: GateConfig,
    *,
    after: tuple[Step, ...],
    settings: SourcePackageConfig | None = None,
    prefix: str = "sdk.python",
) -> tuple[Step, ...]:
    pkg = settings or config.sdk_python
    phase = plan.phase(f"fast.{prefix}")
    uv_prefix = ["uv", "run", "--project", pkg.project, "--frozen", "--no-sync"]
    synced = python_environment(plan, config, after=after, project=pkg.project, prefix=prefix)
    commands: dict[str, list[str]] = {}
    if isinstance(pkg, SdkConfig):
        commands["generate"] = uv_run(
            config, "python", "-m", "capsem_builder.sdkgen", "--check",
            "--specification", pkg.specification, "--python-package", pkg.source,
        )
    commands.update({
        "lint": [*uv_prefix, "ruff", "check", "--config", config.suites.pytest.project_manifest,
                 pkg.source, pkg.tests],
        "types": [*uv_prefix, "ty", "check", "--project", pkg.project, "--error-on-warning",
                  "--python-platform", "all", pkg.source, pkg.tests],
        "build": [*uv_prefix, "python", "-m", "build", "--no-isolation",
                  "--outdir", pkg.build_output, pkg.project],
    })
    checks = tuple(phase.add(step(
        label, Run(argv), kind=Kind.PACKAGE if label == "build" else Kind.LINT, speed=Speed.FAST,
    ), after=(synced,)) for label, argv in commands.items())
    tested = phase.add(step(
        "tests", Run(["uv", "run", "--frozen", "--no-sync", "pytest"], cwd=config.path(pkg.project)),
        kind=Kind.UNIT_TEST, speed=Speed.FAST,
    ), after=(synced,))
    return (*checks, tested)


def inspect_fragment(plan: Plan, config: GateConfig, *, after: tuple[Step, ...]) -> tuple[Step, ...]:
    """The Inspect AI extension: the SDK lane's checks, less generation. Its
    prewarm also installs the editable SDK it resolves from this checkout."""
    return fragment(
        plan,
        config,
        after=after,
        settings=config.integrations_inspect_ai,
        prefix="integrations.inspect-ai",
    )


def typescript_fragment(plan: Plan, config: GateConfig, *, after: tuple[Step, ...]) -> tuple[Step, ...]:
    settings = config.sdk_typescript
    phase = plan.phase("fast.sdk.typescript")
    root = config.path(settings.project)
    checks = tuple(phase.add(step(
        label, Run(["pnpm", "run", command], cwd=root),
        kind=Kind.LINT, speed=Speed.FAST,
    ), after=after) for label, command in (("lint", "lint"), ("types", "check")))
    built = phase.add(step(
        "build", Run(["pnpm", "pack", "--pack-destination", str(config.path(settings.build_output))], cwd=root),
        kind=Kind.PACKAGE, speed=Speed.FAST,
    ), after=after)
    tested = phase.add(step(
        "tests", Run(["pnpm", "run", "test:sdk"], cwd=root),
        kind=Kind.UNIT_TEST, speed=Speed.FAST,
    ), after=(built,))
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
    mcp_tested = mcp_phase.add(
        step(
            "tests",
            Run(["pnpm", "run", "test:mcp"], cwd=mcp_root),
            kind=Kind.UNIT_TEST,
            speed=Speed.FAST,
        ),
        after=(mcp_built,),
    )
    return (*checks, built, tested, generated, mcp_built, mcp_tested)


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
